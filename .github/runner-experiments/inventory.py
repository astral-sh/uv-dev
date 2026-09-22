"""Record static runner metadata and boundary counters without background sampling."""

import hashlib
import json
import os
import subprocess
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path


def command(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, check=False)
    return {
        "status": result.returncode,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def read(path):
    try:
        return Path(path).read_text()
    except OSError as error:
        return {"error": str(error)}


def inventory():
    temporary_directory = (
        "/tmp/uv-runner-acceptance"
        if os.environ.get("TEMP_STORAGE") == "tmpfs"
        else tempfile.gettempdir()
    )
    counters = [
        "/proc/stat",
        "/proc/vmstat",
        "/proc/meminfo",
        "/proc/pressure/cpu",
        "/proc/pressure/io",
        "/proc/pressure/memory",
        "/sys/fs/cgroup/cpu.stat",
        "/sys/fs/cgroup/cpu.max",
        "/sys/fs/cgroup/memory.events",
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/io.stat",
        "/sys/kernel/mm/transparent_hugepage/enabled",
        "/sys/kernel/mm/transparent_hugepage/defrag",
    ]
    source_files = [
        "Cargo.lock",
        "Cargo.toml",
        "rust-toolchain.toml",
        ".config/nextest.toml",
    ]
    return {
        "recorded_at": datetime.now(UTC).isoformat(),
        "storage": os.environ.get("TEMP_STORAGE"),
        "temporary_directory": temporary_directory,
        "github": {
            key: os.environ.get(key)
            for key in [
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "RUNNER_NAME",
            ]
        },
        "kernel": command(["uname", "-a"]),
        "cpu": command(["lscpu", "--json"]),
        "microcodes": sorted(
            {
                line.split(":", 1)[1].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("microcode")
            }
        ),
        "mitigations": {
            path.name: read(path)
            for path in sorted(
                Path("/sys/devices/system/cpu/vulnerabilities").glob("*")
            )
        },
        "filesystems": {
            path: command(["findmnt", "--json", "-T", path])
            for path in [
                ".",
                "target",
                "/tmp",
                temporary_directory,
                "/btrfs",
                "/tmpfs",
                "/ext4",
            ]
        },
        "rustc": command(["rustc", "-Vv"]),
        "toolchains": command(["rustup", "toolchain", "list"]),
        "source_hashes": {
            path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
            for path in source_files
        },
        "counters": {path: read(path) for path in counters},
    }


if __name__ == "__main__":
    output = Path(sys.argv[1])
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(inventory(), indent=2) + "\n")
