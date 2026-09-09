"""Controlled, disposable Linux runner experiments for uv#20426."""

import argparse
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from monitor import Monitor, VmPerf, probe_perf, read_file

SOURCE_REVISION = "e9837f6e09e481bf5d1c2c2f13b641c14a366518"
AUTH_FIX = "51bcea71165dc26c1fd9ea6e6686ba413ae1f679"
CARGO = [
    "cargo",
    "nextest",
    "run",
    "--cargo-profile",
    "fast-build-nightly",
    "-Z",
    "panic-abort-tests",
    "-Z",
    "checksum-freshness",
    "--features",
    "test-python-patch,test-universal,native-auth,secret-service",
    "--workspace",
    "--profile",
    "ci-linux",
]
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def capture(command):
    return subprocess.check_output(command, text=True).strip()


def counters():
    """Read VM-wide pressure and CPU counters around each measured process."""
    result = {}
    for filename in (
        "/proc/stat",
        "/proc/pressure/cpu",
        "/proc/pressure/io",
        "/proc/pressure/memory",
        "/sys/fs/cgroup/cpu.stat",
        "/sys/fs/cgroup/memory.events",
    ):
        path = Path(filename)
        if path.exists():
            result[filename] = path.read_text()
    return result


def measure(command, name, environment, results, instrumentation=None):
    logfile = results / f"{name}.log"
    timefile = results / f"{name}.time"
    monitor = None
    monitoring = None
    with (
        VmPerf(instrumentation, results / f"{name}.perf.txt") as profiler,
        logfile.open("w") as output,
        (results / f"{name}.events.jsonl").open("w") as events,
    ):
        before = counters()
        started_utc_ns = time.time_ns()
        started_monotonic_ns = time.monotonic_ns()
        process = subprocess.Popen(
            ["/usr/bin/time", "-v", "-o", str(timefile), *command],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if instrumentation is not None:
            monitor = Monitor(process.pid, results / f"{name}.monitor.jsonl")
            monitor.start()
        try:
            for line in process.stdout:
                events.write(
                    json.dumps(
                        {
                            "utc_ns": time.time_ns(),
                            "monotonic_ns": time.monotonic_ns(),
                            "line": ANSI.sub("", line).rstrip(),
                        }
                    )
                    + "\n"
                )
                print(line, end="", flush=True)
                output.write(line)
            returncode = process.wait()
            ended_utc_ns = time.time_ns()
            ended_monotonic_ns = time.monotonic_ns()
        finally:
            if monitor is not None:
                monitoring = monitor.stop()
    elapsed = (ended_monotonic_ns - started_monotonic_ns) / 1e9
    after = counters()
    log = ANSI.sub("", logfile.read_text())
    summaries = re.findall(r"Summary \[\s*([0-9.]+)s\] ([^\n]+)", log)
    test_summary = summaries[-1][1] if summaries else None
    counts = re.search(r"(\d+) tests run: (\d+) passed", test_summary or "")
    timing = timefile.read_text()
    row = {
        "name": name,
        "command": command,
        "perf_capture": profiler.result,
        "started_utc_ns": started_utc_ns,
        "ended_utc_ns": ended_utc_ns,
        "started_monotonic_ns": started_monotonic_ns,
        "ended_monotonic_ns": ended_monotonic_ns,
        "monitoring": monitoring,
        "returncode": returncode,
        "elapsed_seconds": elapsed,
        "test_seconds": float(summaries[-1][0]) if summaries else None,
        "test_summary": test_summary,
        "compile_summary": re.findall(r"Finished [^\n]*profile[^\n]*", log),
        "tests_run": int(counts[1]) if counts else None,
        "tests_passed": int(counts[2]) if counts else None,
        "user_seconds": float(
            re.search(r"User time \(seconds\): ([0-9.]+)", timing)[1]
        ),
        "system_seconds": float(
            re.search(r"System time \(seconds\): ([0-9.]+)", timing)[1]
        ),
        "tmpdir": environment["TMPDIR"],
        "filesystem": capture(
            ["findmnt", "-n", "-o", "FSTYPE,TARGET", "-T", environment["TMPDIR"]]
        ),
        "counters_before": before,
        "counters_after": after,
    }
    (results / f"{name}.json").write_text(json.dumps(row, indent=2) + "\n")
    junit = Path("target/nextest/ci-linux/junit.xml")
    if junit.exists():
        shutil.copyfile(junit, results / f"{name}.junit.xml")
    print(
        "RCA_RESULT "
        + json.dumps(
            {
                key: value
                for key, value in row.items()
                if not key.startswith("counters_")
            }
        ),
        flush=True,
    )
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "mode", choices=("cpu", "filesystem", "high-workers", "monitored-filesystem")
    )
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args()
    variants = {
        "cpu": [8, 20, 40],
        "filesystem": ["native", "ext4", "tmpfs"],
        "high-workers": [40, 64, 80],
        "monitored-filesystem": ["native", "ext4", "tmpfs"],
    }[arguments.mode]
    replicated = arguments.mode in ("high-workers", "monitored-filesystem")
    if replicated:
        replica = int(os.environ["RCA_REPLICA"])
        if replica not in (1, 2):
            raise RuntimeError(f"Unexpected replica: {replica}")
        if replica == 2:
            variants = variants[::-1]
    # Rotate each variant through every position to balance order effects.
    schedules = [variants[index:] + variants[:index] for index in range(len(variants))]
    if arguments.dry_run:
        print(
            json.dumps(
                {
                    "mode": arguments.mode,
                    "schedules": schedules,
                    "source": SOURCE_REVISION,
                    "native_monitor_order": [
                        [False, True]
                        if (round_index + replica) % 2 == 0
                        else [True, False]
                        for round_index in (1, 2, 3)
                    ]
                    if arguments.mode == "monitored-filesystem"
                    else None,
                }
            )
        )
        return

    if capture(["git", "rev-parse", "HEAD"]) != SOURCE_REVISION:
        raise RuntimeError("Unexpected uv source revision")
    changed = capture(["git", "status", "--porcelain"])
    if changed:
        raise RuntimeError(f"Unexpected source changes: {changed}")
    if replicated:
        # Keep the native credential store isolated across repeated full suites.
        patch = Path(__file__).with_name("native-auth-isolation.patch")
        subprocess.run(["git", "apply", "--check", str(patch)], check=True)
        subprocess.run(["git", "apply", str(patch)], check=True)
        expected_cpus = 32 if arguments.mode == "high-workers" else 16
        if len(os.sched_getaffinity(0)) != expected_cpus:
            raise RuntimeError(f"Expected a {expected_cpus}-vCPU runner")

    results = arguments.results.resolve()
    results.mkdir(parents=True, exist_ok=True)
    scratch = Path.home() / "code" / "tmp" / "uv-runner-rca"
    native = scratch / "native"
    native.mkdir(parents=True, exist_ok=True)
    seed = scratch / "python-seed"
    seed.mkdir(exist_ok=True)
    environment = dict(os.environ, TMPDIR=str(native), UV_PYTHON_CACHE_DIR=str(seed))
    metadata = {
        "source": SOURCE_REVISION,
        "mode": arguments.mode,
        "runner": os.environ.get("RCA_RUNNER"),
        "schedules": schedules,
        "lscpu": json.loads(capture(["lscpu", "-J"])),
        "affinity": sorted(os.sched_getaffinity(0)),
        "kernel": capture(["uname", "-sr"]),
        "source_diff": capture(["git", "diff"]),
    }
    if replicated:
        metadata.update(
            replica=replica,
            common_auth_fix=AUTH_FIX,
            rustc=capture(["rustc", "-Vv"]),
            nextest=capture(["cargo", "nextest", "--version"]),
            storage=json.loads(
                capture(
                    [
                        "findmnt",
                        "--json",
                        "--output",
                        "TARGET,SOURCE,FSTYPE,OPTIONS",
                        "--target",
                        str(native),
                    ]
                )
            ),
        )
    instrumentation = None
    if arguments.mode == "monitored-filesystem":
        instrumentation = probe_perf(results)
        metadata.update(
            perf=instrumentation,
            clock_ticks=os.sysconf("SC_CLK_TCK"),
            page_size=os.sysconf("SC_PAGE_SIZE"),
            monitoring_interval_seconds=1,
            thread_interval_seconds=2,
            mounts=capture(["findmnt", "-J", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"]),
            block_devices=capture(
                ["lsblk", "-J", "-o", "NAME,TYPE,SIZE,FSTYPE,MOUNTPOINTS,PKNAME"]
            ),
            monitoring_settings={
                name: read_file(Path(name))
                for name in (
                    "/proc/sys/kernel/perf_event_paranoid",
                    "/proc/sys/kernel/sched_schedstats",
                    "/proc/sys/kernel/task_delayacct",
                    "/proc/sys/kernel/kptr_restrict",
                )
            },
        )
    for filename in ("cpu.max", "cpuset.cpus.effective", "memory.max"):
        path = Path("/sys/fs/cgroup") / filename
        if path.exists():
            metadata[filename] = path.read_text().strip()
    (results / "machine.json").write_text(json.dumps(metadata, indent=2) + "\n")
    command = CARGO.copy()
    subprocess.run([*command, "--no-run"], env=environment, check=True)
    warmup_workers = "40" if arguments.mode == "high-workers" else "20"
    warmup = measure(
        [*command, "--test-threads", warmup_workers], "warmup", environment, results
    )
    expected_count = warmup["tests_run"]
    if (
        warmup["returncode"]
        or not expected_count
        or warmup["tests_passed"] != expected_count
    ):
        raise RuntimeError("Warmup failed; refusing to compare incomplete workloads")
    if expected_count != 4941:
        raise RuntimeError(f"Expected 4941 tests, got {expected_count}")
    print("PYTHON_CACHE_SEED " + capture(["du", "-sh", str(seed)]), flush=True)

    failures = []
    samples = []
    for round_index, schedule in enumerate(schedules, start=1):
        for variant in schedule:
            if arguments.mode == "monitored-filesystem":
                monitor_order = [True]
                if variant == "native":
                    monitor_order = (
                        [False, True]
                        if (round_index + replica) % 2 == 0
                        else [True, False]
                    )
                for enabled in monitor_order:
                    name = (
                        f"round-{round_index}-{variant}"
                        if enabled
                        else f"overhead-{round_index}-native"
                    )
                    samples.append(
                        (variant, name, instrumentation if enabled else None)
                    )
            else:
                samples.append((variant, f"round-{round_index}-{variant}", None))
    for variant, name, sample_instrumentation in samples:
        parent = (
            scratch / str(variant)
            if arguments.mode in ("filesystem", "monitored-filesystem")
            else native
        )
        with tempfile.TemporaryDirectory(prefix="sample-", dir=parent) as temporary:
            # Copy the same warmed Python archive cache before every measurement.
            # Copies and cleanup are outside the measured child process.
            python_cache = Path(temporary) / "python-downloads"
            subprocess.run(
                ["cp", "-a", "--reflink=auto", str(seed), str(python_cache)],
                check=True,
            )
            sample_environment = dict(
                environment, TMPDIR=temporary, UV_PYTHON_CACHE_DIR=str(python_cache)
            )
            workers = (
                str(variant) if arguments.mode in ("cpu", "high-workers") else "20"
            )
            row = measure(
                [*command, "--test-threads", workers],
                name,
                sample_environment,
                results,
                sample_instrumentation,
            )
            if (
                row["returncode"]
                or row["tests_run"] != expected_count
                or row["tests_passed"] != expected_count
            ):
                failures.append(row["name"])
            if sample_instrumentation is not None and (
                not row["monitoring"]["samples"] or row["monitoring"]["errors"]
            ):
                failures.append(f"{row['name']}: monitoring incomplete")
            if row["perf_capture"] is not None and row["perf_capture"]["returncode"]:
                failures.append(f"{row['name']}: VM perf failed")
    if failures:
        raise RuntimeError(f"Incomplete samples: {failures}")


if __name__ == "__main__":
    main()
