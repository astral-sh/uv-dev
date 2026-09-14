"""Sample kernel execution on disposable CI runners without exporting raw traces."""

import os
import select
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from differentials import COMMAND, RESULTS, inventory, run_phase, write_json
from monitor import read_file


def kernel_settings():
    paths = [
        Path("/sys/kernel/mm/transparent_hugepage/enabled"),
        Path("/sys/kernel/mm/transparent_hugepage/defrag"),
        Path("/proc/sys/kernel/perf_event_paranoid"),
        Path("/proc/sys/kernel/kptr_restrict"),
        Path("/proc/sys/vm/dirty_ratio"),
        Path("/proc/sys/vm/dirty_background_ratio"),
        Path("/proc/sys/vm/swappiness"),
        Path("/sys/fs/cgroup/cpu.max"),
        *sorted(Path("/sys/devices/system/cpu/vulnerabilities").glob("*")),
    ]
    write_json(RESULTS / "kernel-settings.json", {str(p): read_file(p) for p in paths})


def record_command(executable, output):
    return [
        executable,
        "record",
        "-a",
        "-e",
        "cpu-clock:k",
        "-F",
        "99",
        "--call-graph",
        "fp",
        "--kernel-callchains",
        "--no-buildid",
        "--no-buildid-cache",
        "-o",
        str(output),
    ]


def capture_helper(executable, output):
    # EOF stops recording if the unprivileged driver exits. A fixed bound also
    # protects against an inherited pipe keeping the helper alive.
    with subprocess.Popen(
        record_command(executable, output),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        start_new_session=True,
    ) as process:
        try:
            time.sleep(0.2)
            if process.poll() is not None:
                raise RuntimeError("Kernel recording exited during startup")
            print("ready", flush=True)
            if not select.select([sys.stdin], [], [], 650)[0]:
                raise RuntimeError("Kernel recording exceeded its time bound")
            sys.stdin.read(1)
            if process.poll() is not None:
                raise RuntimeError("Kernel recording exited before the workload")
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise RuntimeError("Kernel recording did not stop") from None
        if process.returncode not in (0, -signal.SIGINT):
            raise RuntimeError(f"Kernel recording failed: {process.returncode}")


class KernelProfile:
    def __init__(self, label):
        self.label = label
        self.process = None
        self.temporary = None
        self.executable = os.environ["RCA_PERF"]

    def __enter__(self):
        scratch = Path.home() / "code/tmp/uv-kernel-profiles"
        scratch.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=scratch)
        self.raw = Path(self.temporary.name) / "perf.data"
        self.process = subprocess.Popen(
            [
                "sudo",
                "-n",
                "env",
                f"PYTHONPATH={Path(__file__).resolve().parent}",
                sys.executable,
                str(Path(__file__).resolve()),
                "--capture",
                self.executable,
                str(self.raw),
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if (
            not select.select([self.process.stdout], [], [], 15)[0]
            or self.process.stdout.readline().strip() != "ready"
        ):
            self.__exit__(None, None, None)
            raise RuntimeError("Kernel recording failed to start")
        return self

    def __exit__(self, exception_type, exception, traceback):
        try:
            self.process.stdin.close()
            self.process.stdin = None
            stdout, stderr = self.process.communicate(timeout=20)
            capture = {
                "scope": "whole VM; kernel CPU samples only",
                "event": "cpu-clock:k",
                "frequency_hz": 99,
                "returncode": self.process.returncode,
                "stdout": stdout,
                "stderr": stderr,
                "reports": {},
            }
            for name, graph in [("symbols", "none"), ("callers", "graph,0.5,caller")]:
                result = subprocess.run(
                    [
                        "sudo",
                        "-n",
                        self.executable,
                        "report",
                        "--stdio",
                        "--stdio-color",
                        "never",
                        "-i",
                        str(self.raw),
                        "--kallsyms",
                        "/proc/kallsyms",
                        "--no-children",
                        "--sort",
                        "symbol",
                        "--show-nr-samples",
                        "--percent-limit",
                        "0.1",
                        "--call-graph",
                        graph,
                    ],
                    capture_output=True,
                    text=True,
                    timeout=60,
                    check=False,
                )
                (RESULTS / f"{self.label}.{name}.txt").write_text(result.stdout)
                capture["reports"][name] = {
                    "returncode": result.returncode,
                    "stderr": result.stderr,
                }
            write_json(RESULTS / f"{self.label}.capture.json", capture)
            if self.process.returncode or any(
                report["returncode"] for report in capture["reports"].values()
            ):
                raise RuntimeError("Kernel capture failed; see capture.json")
        finally:
            # Raw perf records include process mapping metadata. Export only
            # aggregate kernel symbols and kernel call graphs, never perf.data.
            if self.temporary:
                self.temporary.cleanup()


def main():
    RESULTS.mkdir(exist_ok=True)
    inventory()
    kernel_settings()
    run_phase("compile", [*COMMAND, "--no-run"])
    run_phase("warmup-tests", COMMAND)
    order = [False, True] if int(os.environ["RCA_REPLICA"]) % 2 else [True, False]
    for sampled in order:
        if sampled:
            with KernelProfile("kernel-tests"):
                run_phase("kernel-tests", COMMAND)
        else:
            run_phase("control-tests", COMMAND)


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--capture":
        capture_helper(*sys.argv[2:])
    else:
        main()
