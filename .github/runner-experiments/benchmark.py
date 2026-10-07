"""Controlled, disposable Linux runner experiments for uv#20426."""

import json
import re
import shutil
import subprocess
import time
from pathlib import Path

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
