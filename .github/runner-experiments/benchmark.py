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
    parser.add_argument("mode", choices=("cpu", "filesystem", "cache-volume"))
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args()
    variants = {
        "cpu": [8, 20, 40],
        "filesystem": ["native", "ext4", "tmpfs"],
        "cache-volume": ["native", "cache-volume"],
    }[arguments.mode]
    # Rotate each variant through every position to balance order effects.
    schedules = [variants[index:] + variants[:index] for index in range(len(variants))]
    if arguments.mode == "cache-volume":
        replica = int(os.environ["RCA_REPLICA"])
        if replica not in (1, 2):
            raise RuntimeError(f"Unexpected replica: {replica}")
        if replica == 2:
            variants = variants[::-1]
        schedules = [variants, variants[::-1], variants]
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
    changed = capture(["git", "status", "--porcelain"])
    if changed:
        raise RuntimeError(f"Unexpected source changes: {changed}")
    if arguments.mode == "cache-volume":
        # Both storage variants get the same native-auth isolation repair.
        patch = Path(__file__).with_name("native-auth-isolation.patch")
        subprocess.run(["git", "apply", "--check", str(patch)], check=True)
        subprocess.run(["git", "apply", str(patch)], check=True)

    results = arguments.results.resolve()
    results.mkdir(parents=True, exist_ok=True)
    scratch = Path.home() / "code" / "tmp" / "uv-runner-rca"
    native = scratch / "native"
    native.mkdir(parents=True, exist_ok=True)
    storage_paths = {
        variant: scratch / ("volume" if variant == "cache-volume" else str(variant))
        for variant in variants
    }
    seed = scratch / "python-seed"
    seed.mkdir(exist_ok=True)
    environment = dict(os.environ, TMPDIR=str(native), UV_PYTHON_CACHE_DIR=str(seed))
    if arguments.mode == "cache-volume":
        cache_directory = storage_paths["cache-volume"]
        # Different path lengths can trigger uv's long-shebang wrapper and alter
        # snapshots independently of the underlying storage.
        if len(os.fsencode(cache_directory)) != len(os.fsencode(native)):
            raise RuntimeError("Storage paths must have equal byte lengths")
        if not cache_directory.is_mount():
            raise RuntimeError(
                "Namespace cache-volume scratch directory is not mounted"
            )
        if cache_directory.stat().st_dev != Path("/cache").stat().st_dev:
            raise RuntimeError("Scratch directory is not on the Namespace cache volume")
        if cache_directory.stat().st_dev == native.stat().st_dev:
            raise RuntimeError(
                "Cache volume and native scratch are on the same filesystem"
            )
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
    if arguments.mode == "cache-volume":
        metadata.update(
            common_auth_fix=AUTH_FIX,
            replica=replica,
            storage={
                variant: {
                    "device": storage_paths[variant].stat().st_dev,
                    "mount": json.loads(
                        capture(
                            [
                                "findmnt",
                                "--json",
                                "--output",
                                "TARGET,SOURCE,FSTYPE,OPTIONS",
                                "--target",
                                str(storage_paths[variant]),
                            ]
                        )
                    ),
                }
                for variant in variants
            },
            scope="Test temporary files and identical warmed Python download caches; no cross-job cache reuse or Rust cache changes",
        )
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
    if arguments.mode == "cache-volume":
        # Populate each storage path once before the paired measurements.
        for variant in variants:
            with tempfile.TemporaryDirectory(
                prefix="warmup-", dir=storage_paths[variant]
            ) as temporary:
                python_cache = Path(temporary) / "python-downloads"
                subprocess.run(
                    ["cp", "-a", "--reflink=auto", str(seed), str(python_cache)],
                    check=True,
                )
                sample_environment = dict(
                    environment, TMPDIR=temporary, UV_PYTHON_CACHE_DIR=str(python_cache)
                )
                row = measure(
                    [*command, "--test-threads", "20"],
                    f"warmup-{variant}",
                    sample_environment,
                    results,
                )
                if (
                    row["returncode"]
                    or row["tests_run"] != expected_count
                    or row["tests_passed"] != expected_count
                ):
                    raise RuntimeError(f"Incomplete warmup: {variant}")
    for round_index, schedule in enumerate(schedules, start=1):
        for variant in schedule:
            parent = (
                storage_paths[variant]
                if arguments.mode in ("filesystem", "cache-volume")
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
