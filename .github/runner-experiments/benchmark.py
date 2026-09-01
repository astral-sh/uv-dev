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

SOURCE_REVISION = "e9837f6e09e481bf5d1c2c2f13b641c14a366518"
INSTALL_POOL_OVERRIDE = "UV_RUNNER_EXPERIMENT_INSTALL_THREADS"
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
    ):
        path = Path(filename)
        if path.exists():
            result[filename] = path.read_text()
    return result


def measure(command, name, environment, results):
    logfile = results / f"{name}.log"
    timefile = results / f"{name}.time"
    before = counters()
    started = time.monotonic()
    with logfile.open("w") as output:
        process = subprocess.Popen(
            ["/usr/bin/time", "-v", "-o", str(timefile), *command],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        for line in process.stdout:
            print(line, end="", flush=True)
            output.write(line)
        returncode = process.wait()
    elapsed = time.monotonic() - started
    after = counters()
    log = ANSI.sub("", logfile.read_text())
    summaries = re.findall(r"Summary \[\s*([0-9.]+)s\] ([^\n]+)", log)
    test_summary = summaries[-1][1] if summaries else None
    counts = re.search(r"(\d+) tests run: (\d+) passed", test_summary or "")
    timing = timefile.read_text()
    row = {
        "name": name,
        "command": command,
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
        "install_pool_override": environment.get(INSTALL_POOL_OVERRIDE),
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
    parser.add_argument("mode", choices=("cpu", "filesystem", "install-pool"))
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args()
    variants = {
        "cpu": [8, 20, 40],
        "filesystem": ["native", "ext4", "tmpfs"],
        "install-pool": ["default", "1", "8"],
    }[arguments.mode]
    # Rotate each variant through every position to balance order effects.
    schedules = [variants[index:] + variants[:index] for index in range(len(variants))]
    if arguments.dry_run:
        print(
            json.dumps(
                {
                    "mode": arguments.mode,
                    "schedules": schedules,
                    "source": SOURCE_REVISION,
                }
            )
        )
        return

    if capture(["git", "rev-parse", "HEAD"]) != SOURCE_REVISION:
        raise RuntimeError("Unexpected uv source revision")
    changed = capture(["git", "diff", "--name-only"])
    if changed:
        raise RuntimeError(f"Unexpected source changes: {changed}")
    if arguments.mode == "install-pool":
        # Every variant uses the same test-only hook, including the default control.
        # Explicit per-test environment settings are applied after this hook.
        patch = Path(__file__).with_name("install-pool.patch")
        subprocess.run(["git", "apply", "--check", str(patch)], check=True)
        subprocess.run(["git", "apply", str(patch)], check=True)

    results = arguments.results.resolve()
    results.mkdir(parents=True, exist_ok=True)
    scratch = Path.home() / "code" / "tmp" / "uv-runner-rca"
    native = scratch / "native"
    native.mkdir(parents=True, exist_ok=True)
    seed = scratch / "python-seed"
    seed.mkdir(exist_ok=True)
    environment = dict(os.environ, TMPDIR=str(native), UV_PYTHON_CACHE_DIR=str(seed))
    environment.pop(INSTALL_POOL_OVERRIDE, None)
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
    for filename in ("cpu.max", "cpuset.cpus.effective", "memory.max"):
        path = Path("/sys/fs/cgroup") / filename
        if path.exists():
            metadata[filename] = path.read_text().strip()
    (results / "machine.json").write_text(json.dumps(metadata, indent=2) + "\n")
    command = CARGO.copy()
    subprocess.run([*command, "--no-run"], env=environment, check=True)
    warmup = measure([*command, "--test-threads", "20"], "warmup", environment, results)
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
    for round_index, schedule in enumerate(schedules, start=1):
        for variant in schedule:
            parent = (
                scratch / str(variant) if arguments.mode == "filesystem" else native
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
                if arguments.mode == "install-pool" and variant != "default":
                    sample_environment[INSTALL_POOL_OVERRIDE] = variant
                workers = str(variant) if arguments.mode == "cpu" else "20"
                row = measure(
                    [*command, "--test-threads", workers],
                    f"round-{round_index}-{variant}",
                    sample_environment,
                    results,
                )
                if (
                    row["returncode"]
                    or row["tests_run"] != expected_count
                    or row["tests_passed"] != expected_count
                ):
                    failures.append(row["name"])
    if failures:
        raise RuntimeError(f"Incomplete samples: {failures}")


if __name__ == "__main__":
    main()
