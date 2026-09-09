"""Read-only Linux sampling for disposable benchmark runners.

VM counters include work outside the benchmark. Process/thread snapshots only include
the measured process tree, miss tasks that exit between samples, and are not a trace.
No command lines, environments, or network addresses are collected.
"""

import json
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

COUNTERS = (
    "/proc/stat",
    "/proc/loadavg",
    "/proc/meminfo",
    "/proc/vmstat",
    "/proc/diskstats",
    "/proc/net/dev",
    "/proc/pressure/cpu",
    "/proc/pressure/io",
    "/proc/pressure/memory",
    "/sys/fs/cgroup/cpu.stat",
    "/sys/fs/cgroup/cpu.pressure",
    "/sys/fs/cgroup/memory.current",
    "/sys/fs/cgroup/memory.stat",
    "/sys/fs/cgroup/memory.events",
    "/sys/fs/cgroup/memory.pressure",
    "/sys/fs/cgroup/io.stat",
    "/sys/fs/cgroup/io.pressure",
)


def read_file(path):
    try:
        return path.read_text().strip()
    except OSError as error:
        return {"error": type(error).__name__}


def parse_stat(text):
    """Parse proc stat without splitting process names containing spaces or ')'."""
    prefix, _, tail = text.rpartition(")")
    process, name = prefix.split("(", 1)
    fields = tail.split()
    return {
        "pid": int(process),
        "comm": name,
        "state": fields[0],
        "ppid": int(fields[1]),
        "minor_faults": int(fields[7]),
        "major_faults": int(fields[9]),
        "user_ticks": int(fields[11]),
        "system_ticks": int(fields[12]),
        "threads": int(fields[17]),
        "start_ticks": int(fields[19]),
        "rss_pages": int(fields[21]),
        "cpu": int(fields[36]),
        "block_io_delay_ticks": int(fields[39]),
    }


def process_tree(root_pid, proc=Path("/proc")):
    processes = {}
    races = 0
    for path in proc.glob("[0-9]*/stat"):
        try:
            row = parse_stat(path.read_text())
        except (OSError, ValueError, IndexError):
            races += 1
            continue
        processes[row["pid"]] = row
    selected = {root_pid} & processes.keys()
    while (
        children := {pid for pid, row in processes.items() if row["ppid"] in selected}
        - selected
    ):
        selected.update(children)
    result = []
    for pid in sorted(selected):
        row = processes[pid]
        directory = proc / str(pid)
        row["io"] = read_file(directory / "io")
        row["tasks"] = []
        for path in directory.glob("task/[0-9]*/stat"):
            try:
                task = parse_stat(path.read_text())
                # Memory and faults in /proc/PID/stat are process-wide; do not
                # add task RSS values together as though memory were unshared.
                row["tasks"].append(
                    {
                        "tid": task["pid"],
                        "start_ticks": task["start_ticks"],
                        "state": task["state"],
                        "user_ticks": task["user_ticks"],
                        "system_ticks": task["system_ticks"],
                        "cpu": task["cpu"],
                        "schedstat": read_file(path.with_name("schedstat")),
                        "wchan": read_file(path.with_name("wchan")),
                    }
                )
            except (OSError, ValueError, IndexError):
                races += 1
        # Reject PID reuse while the task list was being read.
        try:
            if (
                parse_stat((directory / "stat").read_text())["start_ticks"]
                != row["start_ticks"]
            ):
                races += 1
                continue
        except (OSError, ValueError, IndexError):
            races += 1
            continue
        result.append(row)
    return {
        "processes": result,
        "outside_processes": [
            row for pid, row in processes.items() if pid not in selected
        ],
        "read_races": races,
    }


class Monitor:
    def __init__(self, root_pid, path, interval=1.0):
        self.root_pid = root_pid
        self.path = path
        self.interval = interval
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.summary = {
            "samples": 0,
            "task_samples": 0,
            "errors": [],
            "completed": False,
        }

    def start(self):
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.thread.join(timeout=10)
        if self.thread.is_alive():
            raise RuntimeError("Monitoring did not stop; refusing to overlap samples")
        if not self.summary["completed"] and not self.summary["errors"]:
            self.summary["errors"].append("Sampler exited unexpectedly")
        return self.summary

    def run(self):
        cpu_started = time.thread_time_ns()
        try:
            with self.path.open("w") as output:
                while not self.stop_event.is_set():
                    started = time.monotonic_ns()
                    row = {
                        "utc_ns": time.time_ns(),
                        "monotonic_ns": started,
                        "counters": {name: read_file(Path(name)) for name in COUNTERS},
                        "frequency_khz": {
                            str(path): read_file(path)
                            for path in Path("/sys/devices/system/cpu").glob(
                                "cpu[0-9]*/cpufreq/scaling_cur_freq"
                            )
                        },
                    }
                    # Thread enumeration is more expensive; sample it every 2s.
                    if self.summary["samples"] % 2 == 0:
                        row["tree"] = process_tree(self.root_pid)
                        self.summary["task_samples"] += 1
                    row["read_duration_ns"] = time.monotonic_ns() - started
                    row["sampler_cpu_ns"] = time.thread_time_ns() - cpu_started
                    output.write(json.dumps(row, separators=(",", ":")) + "\n")
                    self.summary["samples"] += 1
                    elapsed = (time.monotonic_ns() - started) / 1e9
                    self.summary["max_collection_seconds"] = max(
                        elapsed, self.summary.get("max_collection_seconds", 0)
                    )
                    self.stop_event.wait(max(0, self.interval - elapsed))
            self.summary["completed"] = True
        except (OSError, ValueError) as error:
            self.summary["errors"].append(f"{type(error).__name__}: {error}")
        finally:
            self.summary["cpu_seconds"] = (time.thread_time_ns() - cpu_started) / 1e9


def perf_supported(returncode, text):
    return returncode == 0 and not re.search(
        r"<not (?:supported|counted)>|permission|not supported|Access to performance",
        text,
        re.IGNORECASE,
    )


def probe_perf(results):
    """Optional PMU/software counters; missing support must remain explicit."""
    executable = os.environ.get("RCA_PERF") or shutil.which("perf")
    report = {"executable": executable, "events": [], "probes": [], "scope": "whole VM"}
    if executable:
        for events in (
            "task-clock,context-switches,cpu-migrations,page-faults",
            "{cycles,instructions}",
            "{cache-references,cache-misses}",
        ):
            command = [
                "sudo",
                "-n",
                executable,
                "stat",
                "-a",
                "-x",
                ";",
                "-e",
                events,
                "--",
                "sleep",
                "0.1",
            ]
            try:
                result = subprocess.run(
                    command, capture_output=True, text=True, timeout=10, check=False
                )
                supported = perf_supported(result.returncode, result.stderr)
                report["probes"].append(
                    {
                        "events": events,
                        "returncode": result.returncode,
                        "output": result.stderr,
                        "supported": supported,
                    }
                )
                if supported:
                    report["events"].append(events)
            except (OSError, subprocess.TimeoutExpired) as error:
                report["probes"].append(
                    {"events": events, "error": str(error), "supported": False}
                )
        if report["events"]:
            # Check the exact combination and interval mode before running uv.
            command = perf_command(
                report, results / "perf-combined-probe.txt", ["sleep", "1.1"]
            )
            try:
                result = subprocess.run(
                    command, capture_output=True, text=True, timeout=10, check=False
                )
                text = (results / "perf-combined-probe.txt").read_text() + result.stderr
                report["combined_supported"] = perf_supported(result.returncode, text)
            except (OSError, subprocess.TimeoutExpired) as error:
                report["combined_supported"] = False
                report["combined_error"] = str(error)
            if not report["combined_supported"]:
                report["events"] = []
    (results / "perf-capabilities.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def perf_command(report, path, command, *, privileged=True):
    if not report["events"]:
        return command
    return (["sudo", "-n"] if privileged else []) + [
        report["executable"],
        "stat",
        "-a",
        "--no-big-num",
        "-x",
        ";",
        "-I",
        "1000",
        "-e",
        ",".join(report["events"]),
        "-o",
        str(path),
        *(["--", *command] if command else []),
    ]


def perf_coverage(path):
    coverage = {}
    for line in path.read_text().splitlines():
        fields = line.split(";")
        if len(fields) < 7:
            continue
        row = coverage.setdefault(fields[3], {"counted": 0, "missing": 0})
        try:
            float(fields[0])
            float(fields[1])
        except ValueError:
            row["missing"] += 1
        else:
            row["counted"] += 1
    return coverage


class VmPerf:
    """Keep privileged VM counting separate from the unprivileged workload."""

    def __init__(self, report, path):
        self.report = report
        self.path = path
        self.process = None
        self.result = None

    def __enter__(self):
        if self.report is None or not self.report["events"]:
            return self
        # EOF on this pipe stops the helper, including if the caller exits.
        self.process = subprocess.Popen(
            [
                "sudo",
                "-n",
                sys.executable,
                str(Path(__file__).resolve()),
                "--vm-perf",
                self.report["executable"],
                str(self.path),
                ",".join(self.report["events"]),
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if (
            not select.select([self.process.stdout], [], [], 10)[0]
            or self.process.stdout.readline().strip() != "ready"
        ):
            self.__exit__(None, None, None)
            raise RuntimeError(f"VM perf failed to start: {self.result}")
        return self

    def __exit__(self, exception_type, exception, traceback):
        if self.process is not None:
            self.process.stdin.close()
            self.process.stdin = None
            stdout, stderr = self.process.communicate(timeout=15)
            self.result = {
                "scope": "whole VM",
                "returncode": self.process.returncode,
                "stdout": stdout,
                "stderr": stderr,
                "coverage": perf_coverage(self.path) if self.path.exists() else {},
            }
            self.path.with_suffix(".capture.json").write_text(
                json.dumps(self.result, indent=2) + "\n"
            )


def vm_perf_helper(executable, output, events):
    report = {"executable": executable, "events": [events]}
    # Count without a dummy child: perf does not reliably stop that child when
    # interrupted, and an orphan could keep the helper's output pipes open.
    # A fixed upper bound protects against a leaked pipe; normal shutdown is EOF.
    with subprocess.Popen(
        perf_command(report, Path(output), [], privileged=False)
        + ["--interval-count", "3600"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        start_new_session=True,
    ) as process:
        try:
            time.sleep(0.05)
            if process.poll() is not None:
                raise RuntimeError("perf exited during startup")
            print("ready", flush=True)
            sys.stdin.read(1)
            if process.poll() is not None:
                raise RuntimeError(
                    f"perf exited before the sample finished: {process.returncode}"
                )
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise RuntimeError("perf did not stop after SIGINT") from None


if __name__ == "__main__":
    if len(sys.argv) != 5 or sys.argv[1] != "--vm-perf":
        raise SystemExit(
            "Internal VM perf helper: expected executable, output, and events"
        )
    vm_perf_helper(*sys.argv[2:])
