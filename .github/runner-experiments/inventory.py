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


def inventory(temporary_directory=None, environment=None):
    temporary_directory = temporary_directory or tempfile.gettempdir()
    environment = environment if environment is not None else os.environ
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
        "temporary_environment": {
            key: environment.get(key) for key in ["TMPDIR", "TEMP", "TMP"]
        },
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
                "/tmp/uv-tests",
                os.environ.get("UV_INTERNAL__TEST_LOWLINKS_FS", "/ext4"),
            ]
        },
        "mountinfo": read("/proc/self/mountinfo"),
        "filesystem_capacity": command(["df", "-B1", "-i", "/tmp", "/tmp/uv-tests"]),
        "filesystem_space": command(["df", "-B1", "/tmp", "/tmp/uv-tests"]),
        "source_tree": command(["git", "rev-parse", "HEAD:crates"]),
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
