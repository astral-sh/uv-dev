"""Compare restored and equal-work builds on disposable Linux CI runners."""

import hashlib
import json
import os
import re
import signal
import subprocess
import time
from datetime import UTC, datetime
from pathlib import Path

from monitor import Monitor, VmPerf, probe_perf

RESULTS = Path("runner-diagnostics")
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


def utc_now():
    return datetime.now(UTC).isoformat()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def inventory():
    commands = {
        "kernel": ["uname", "-a"],
        "cpu": ["lscpu", "--json"],
        "rustc": ["rustc", "-vV"],
        "cargo": ["cargo", "-V"],
        "toolchains": ["rustup", "toolchain", "list"],
        "source": ["git", "rev-parse", "HEAD"],
        "filesystem": ["findmnt", "-J", "-T", ".", "-o", "TARGET,FSTYPE"],
    }
    result = {"recorded_at": utc_now(), "commands": {}}
    for name, command in commands.items():
        response = subprocess.run(command, capture_output=True, text=True, check=False)
        result["commands"][name] = {
            "returncode": response.returncode,
            "stdout": response.stdout,
            "stderr": response.stderr,
        }
    result["environment"] = {
        key: os.environ.get(key)
        for key in (
            "CARGO_HOME",
            "RUSTUP_HOME",
            "CARGO_INCREMENTAL",
            "RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "RUSTC_WRAPPER",
            "RUSTC_BOOTSTRAP",
        )
    }
    result["microcode"] = sorted(
        {
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("microcode")
        }
    )
    result["manifest_hashes"] = {
        name: hashlib.sha256(Path(name).read_bytes()).hexdigest()
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            ".cargo/config.toml",
            "rust-toolchain.toml",
        )
    }
    write_json(RESULTS / "inventory.json", result)


def fingerprint_inventory(label):
    rows = []
    for path in sorted(Path("target/fast-build-nightly/.fingerprint").glob("uv-*/*")):
        if path.is_file():
            stat = path.stat()
            rows.append(
                {
                    "path": str(path),
                    "bytes": stat.st_size,
                    "mtime_ns": stat.st_mtime_ns,
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                }
            )
    write_json(RESULTS / f"{label}.fingerprints.json", rows)


def summarize_log(text):
    text = ANSI.sub("", text)
    passed = set(
        re.findall(r"\bPASS\s+\[\s*[0-9.]+s\]\s+\(\s*\d+/\d+\)\s+([^\n]+)", text)
    )
    return {
        "compiled_crates": sorted(set(re.findall(r"\bCompiling\s+([^\n]+)", text))),
        "nextest_summaries": re.findall(r"Summary[^\n]+", text),
        "passing_test_count": len(passed),
        "pass_identity_sha256": hashlib.sha256(
            "\n".join(sorted(passed)).encode()
        ).hexdigest(),
    }


def run_phase(label, command, *, fingerprint=False, perf=None):
    environment = dict(os.environ)
    environment["LC_ALL"] = "C"
    if fingerprint:
        # Set logging after cache restore so it cannot alter the action's key.
        environment["CARGO_LOG"] = "cargo::core::compiler::fingerprint=debug"
    started_at, started = utc_now(), time.monotonic()
    monitor = None
    monitor_summary = None
    log_path = RESULTS / f"{label}.log"
    time_path = RESULTS / f"{label}.time.txt"
    with log_path.open("w") as output, VmPerf(perf, RESULTS / f"{label}.perf.csv"):
        process = subprocess.Popen(
            ["/usr/bin/time", "-v", "-o", str(time_path), *command],
            stdout=output,
            stderr=subprocess.STDOUT,
            env=environment,
            start_new_session=True,
        )
        if perf is not None:
            monitor = Monitor(process.pid, RESULTS / f"{label}.monitor.jsonl")
            monitor.start()
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
        finally:
            if monitor:
                monitor_summary = monitor.stop()
    result = {
        "label": label,
        "started_at": started_at,
        "completed_at": utc_now(),
        "wall_seconds": time.monotonic() - started,
        "returncode": returncode,
        "monitored": perf is not None,
        "monitor": monitor_summary,
        **summarize_log(log_path.read_text()),
    }
    write_json(RESULTS / f"{label}.json", result)
    print(json.dumps(result), flush=True)
    if returncode:
        raise RuntimeError(
            f"{label} failed with exit code {returncode}; see artifact log"
        )
    if (
        "--no-run" not in command
        and command[:4] == COMMAND[:4]
        and result["passing_test_count"] != 5044
    ):
        raise RuntimeError(f"{label} did not complete the expected test set")
    return result


def main():
    RESULTS.mkdir(exist_ok=True)
    inventory()
    fingerprint_inventory("restored")
    run_phase("restored-compile", [*COMMAND, "--no-run"], fingerprint=True)
    run_phase("restored-tests", COMMAND)
    fingerprint_inventory("after-restored-build")
    # Only remove workspace artifacts in this disposable checkout. Registry
    # dependencies remain warm; no cache entry is removed or overwritten.
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
        )
    )
    members = set(metadata["workspace_members"])
    packages = sorted(
        package["name"] for package in metadata["packages"] if package["id"] in members
    )
    write_json(RESULTS / "cleaned-workspace-packages.json", packages)
    clean = ["cargo", "clean", "--profile", "fast-build-nightly"]
    for package in packages:
        clean.extend(["--package", package])
    run_phase("clean-workspace", clean)
    run_phase("equal-work-compile", [*COMMAND, "--no-run"], fingerprint=True)
    fingerprint_inventory("after-equal-build")
    perf = probe_perf(RESULTS)
    monitored_first = int(os.environ["RCA_REPLICA"]) % 2 == 0
    order = [True, False] if monitored_first else [False, True]
    for monitored in order:
        run_phase(
            "equal-tests-monitored" if monitored else "equal-tests-control",
            COMMAND,
            perf=perf if monitored else None,
        )


if __name__ == "__main__":
    main()
