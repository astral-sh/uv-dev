"""Compare nextest concurrency within each disposable runner, without live sampling."""

import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import time
from datetime import UTC, datetime
from pathlib import Path

from inventory import inventory

RESULTS = Path("benchmark-evidence")
EXPECTED_TESTS = 5044
EXPECTED_IDENTITY = "d8f2099c869cb128d68d327a9731b8d948c654a306a79041f86e8f56cdb4563d"
# Each order occurs twice in the fixed twelve-runner cohort. Sample zero is a pilot.
ORDERS = {
    0: (20, 32, 40),
    1: (32, 40, 20),
    2: (20, 32, 40),
    3: (40, 20, 32),
    4: (32, 20, 40),
    5: (40, 32, 20),
    6: (20, 40, 32),
    7: (40, 32, 20),
    8: (32, 20, 40),
    9: (20, 40, 32),
    10: (40, 20, 32),
    11: (20, 32, 40),
    12: (32, 40, 20),
}
COMMAND = [
    "uv",
    "run",
    "--only-dev",
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
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def now():
    return datetime.now(UTC).isoformat()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def summarize_log(text):
    text = ANSI.sub("", text)
    passed = set(
        re.findall(r"\bPASS\s+\[\s*[0-9.]+s\]\s+\(\s*\d+/\d+\)\s+([^\n]+)", text)
    )
    summaries = re.findall(
        r"Summary\s+\[\s*([0-9.]+)s\]\s+(\d+) tests run: (\d+) passed([^\n]*)",
        text,
    )
    compilations = re.findall(
        r"Finished `fast-build-nightly` profile.*? in ([^\n]+)", text
    )
    result = {
        "compiled_crates": sorted(set(re.findall(r"\bCompiling\s+([^\n]+)", text))),
        "compile_seconds": sum(
            float(value) * {"h": 3600, "m": 60, "s": 1}[unit]
            for value, unit in re.findall(r"([0-9.]+)([hms])", compilations[-1])
        )
        if compilations
        else None,
        "passing_test_count": len(passed),
        "pass_identity_sha256": hashlib.sha256(
            "\n".join(sorted(passed)).encode()
        ).hexdigest(),
        "summaries": re.findall(r"Summary[^\n]+", text),
        "valid_test_set": False,
    }
    if len(summaries) == 1:
        duration, count, pass_count, suffix = summaries[0]
        skipped = re.search(r"(\d+) skipped", suffix)
        result.update(
            test_seconds=float(duration),
            test_count=int(count),
            passed_count=int(pass_count),
            skipped_count=int(skipped[1]) if skipped else 0,
        )
        result["valid_test_set"] = (
            int(count) == int(pass_count) == len(passed) == EXPECTED_TESTS
            and result["skipped_count"] == 4
            and result["pass_identity_sha256"] == EXPECTED_IDENTITY
        )
    return result


def run_phase(label, workers, *, warmup):
    command = [*COMMAND, "--test-threads", str(workers)]
    log_path = RESULTS / f"{label}.log"
    time_path = RESULTS / f"{label}.time.txt"
    # Remove only prior reports so a failed invocation cannot reuse stale JUnit data.
    for path in Path("target/nextest").glob("ci*/junit.xml"):
        path.unlink()
    write_json(RESULTS / f"{label}.before.json", inventory())
    started_at, started = now(), time.monotonic()
    print(f"Starting {label}: {workers} workers (warmup={warmup})", flush=True)
    with log_path.open("w") as output:
        process = subprocess.Popen(
            ["/usr/bin/time", "-v", "-o", str(time_path), *command],
            stdout=output,
            stderr=subprocess.STDOUT,
            env={**os.environ, "LC_ALL": "C"},
            start_new_session=True,
        )
        try:
            returncode = process.wait(timeout=600)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            returncode = 124
    wall_seconds, completed_at = time.monotonic() - started, now()
    write_json(RESULTS / f"{label}.after.json", inventory())
    reports = []
    for path in Path("target/nextest").glob("ci*/junit.xml"):
        destination = RESULTS / f"{label}.{path.parent.name}.junit.xml"
        shutil.copyfile(path, destination)
        reports.append(destination.name)
    result = {
        "label": label,
        "workers": workers,
        "warmup": warmup,
        "command": command,
        "started_at": started_at,
        "completed_at": completed_at,
        "wall_seconds": wall_seconds,
        "returncode": returncode,
        "junit_reports": reports,
        **summarize_log(log_path.read_text()),
    }
    result["valid"] = returncode == 0 and result["valid_test_set"]
    write_json(RESULTS / f"{label}.json", result)
    print(json.dumps(result), flush=True)
    return result


def main():
    sample = int(os.environ["WORKER_SAMPLE"])
    order = ORDERS[sample]
    RESULTS.mkdir(exist_ok=True)
    result = {
        "sample": sample,
        "pilot": sample == 0,
        "order": order,
        "phases": [],
        "complete": False,
    }
    path = RESULTS / "worker-results.json"
    write_json(path, result)
    # A full excluded pass preconditions test and package caches on every runner.
    phases = [("warmup-20", 20, True)] + [
        (f"period-{period}-workers-{workers}", workers, False)
        for period, workers in enumerate(order, start=1)
    ]
    for label, workers, warmup in phases:
        result["phases"].append(run_phase(label, workers, warmup=warmup))
        write_json(path, result)
    result["complete"] = True
    result["valid"] = all(phase["valid"] for phase in result["phases"])
    result["no_measured_rebuilds"] = all(
        not phase["compiled_crates"]
        for phase in result["phases"]
        if not phase["warmup"]
    )
    write_json(path, result)
    return 0 if result["valid"] and result["no_measured_rebuilds"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
