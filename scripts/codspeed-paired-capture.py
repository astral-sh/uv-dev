"""Retain a fixed pair of CodSpeed simulation profiles without uploading measurements to CodSpeed."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
from datetime import datetime, timezone
from pathlib import Path

SOURCES = {
    "parent": (
        "9c4df678e4e0dd7be20fa1fab315b385cd751247",
        "55806f59e0c67e1c6afa5d0b84bcd868b8136389",
    ),
    "child": (
        "97e0bd1a7accb029e1c1af04e373c7734305690d",
        "7621c50d83ebb9886a50a674d674d1d9918380c7",
    ),
}
BENCHMARKS = [
    "discover_workspace_from_all_members",
    "discover_workspace_from_all_members_with_excludes",
    "run_python_version_synthetic_workspace",
]
ENVIRONMENT_KEYS = (
    "CODSPEED_SKIP_UPLOAD",
    "CARGO_INCREMENTAL",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTUP_TOOLCHAIN",
    "MALLOC_CONF",
    "_RJEM_MALLOC_CONF",
    "JEMALLOC_SYS_WITH_MALLOC_CONF",
    "MALLOC_ARENA_MAX",
    "MALLOC_MMAP_THRESHOLD_",
    "MALLOC_TRIM_THRESHOLD_",
    "GLIBC_TUNABLES",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "TMPDIR",
    "TMP",
    "TEMP",
    "UV_CACHE_DIR",
    "UV_PYTHON_INSTALL_DIR",
    "RUNNER_NAME",
    "RUNNER_OS",
    "RUNNER_ARCH",
)


def command(*args):
    result = subprocess.run(args, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def profiles(temp_root):
    return sorted(
        str(path) for path in temp_root.glob("profile.*.out") if path.is_dir()
    )


def new_profile(before, temp_root):
    added = set(profiles(temp_root)) - set(before)
    if len(added) != 1:
        raise ValueError(
            f"Expected one new CodSpeed profile directory, found {len(added)}"
        )
    return Path(added.pop())


def provenance(label):
    expected_sha, expected_tree = SOURCES[label]
    actual_sha = command("git", "rev-parse", "HEAD")
    actual_tree = command("git", "rev-parse", "HEAD^{tree}")
    if (actual_sha, actual_tree) != (expected_sha, expected_tree):
        raise ValueError("Unexpected benchmark source commit or tree")
    command("git", "diff", "--exit-code", "HEAD")
    if os.environ.get("CODSPEED_SKIP_UPLOAD") != "true":
        raise ValueError("CodSpeed upload must be disabled")
    return {
        "label": label,
        "source_sha": actual_sha,
        "source_tree": actual_tree,
        "workflow_sha": os.environ["CAPTURE_WORKFLOW_SHA"],
        "run_id": os.environ["GITHUB_RUN_ID"],
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
        "event": os.environ["GITHUB_EVENT_NAME"],
        "ref": os.environ["GITHUB_REF"],
        "cwd": str(Path.cwd()),
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "hostname": platform.node(),
        "kernel": platform.release(),
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "cpu": json.loads(command("lscpu", "--json")),
        "rustc": command("rustc", "-vV"),
        "cargo": command("cargo", "-V"),
        "libc": command("ldd", "--version"),
        "environment": {key: os.environ.get(key) for key in ENVIRONMENT_KEYS},
        "input_sha256": {
            path: sha256(Path(path))
            for path in (
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                "crates/uv-bench/benches/workspace_discovery.rs",
                "crates/uv-workspace/src/workspace.rs",
                "crates/uv-performance-memory-allocator/src/lib.rs",
            )
        },
    }


def prepare(root, label):
    directory = root / label
    directory.mkdir(parents=True, exist_ok=False)
    write_json(directory / "provenance.json", provenance(label))
    temp_root = Path(tempfile.gettempdir())
    write_json(
        directory / "before.json",
        {"temp_root": str(temp_root), "profiles": profiles(temp_root)},
    )
    (directory / "cpuinfo.txt").write_text(
        Path("/proc/cpuinfo").read_text(), encoding="utf-8"
    )
    binary = Path("target/codspeed/analysis/uv-bench/workspace_discovery")
    write_json(
        directory / "binary.json",
        {"path": str(binary.resolve()), "sha256": sha256(binary)},
    )


def benchmark_order(profile):
    found = []
    for path in sorted(profile.glob("*.out")):
        order = []
        with path.open(encoding="utf-8", errors="replace") as stream:
            for line in stream:
                if not line.startswith("desc: Trigger: Client Request:"):
                    continue
                for name in BENCHMARKS:
                    if line.rstrip().endswith(f"::{name}::{name}"):
                        order.append(name)
        if order:
            found.append({"file": path.name, "order": order})
    return found


def finish(root, label):
    directory = root / label
    before = json.loads((directory / "before.json").read_text())
    profile = new_profile(before["profiles"], Path(before["temp_root"]))
    runner_log = (profile / "runner.log").read_text()
    uploads_disabled = "skip_upload: true" in runner_log
    runner_pinned = "codspeed v5.2.1" in runner_log
    order = benchmark_order(profile)
    files = []
    for path in sorted(profile.rglob("*")):
        if path.is_symlink():
            raise ValueError("Unexpected symlink in CodSpeed profile")
        if path.is_file():
            files.append(
                {
                    "path": str(path.relative_to(profile)),
                    "size": path.stat().st_size,
                    "sha256": sha256(path),
                }
            )
    archive = directory / "profile.tar.gz"
    with tarfile.open(archive, "w:gz") as output:
        output.add(profile, arcname=".", recursive=True)
    shutil.copyfile(profile / "runner.log", directory / "runner.log")
    write_json(
        directory / "manifest.json",
        {
            "profile_directory": str(profile),
            "archive_sha256": sha256(archive),
            "archive_size": archive.stat().st_size,
            "runner_version_verified": runner_pinned,
            "runner_skip_upload_verified": uploads_disabled,
            "workspace_benchmark_order": order,
            "files": files,
        },
    )
    write_json(directory / "after.json", provenance(label))
    if not uploads_disabled or not runner_pinned:
        raise ValueError("Expected runner 5.2.1 with upload disabled")
    if len(order) != 1 or order[0]["order"] != BENCHMARKS:
        raise ValueError("Expected the standard workspace benchmark order exactly once")
    if not any(item["path"].startswith("environment-") for item in files):
        raise ValueError("Missing per-process benchmark environment files")


def compare(root):
    records = {
        label: json.loads((root / label / "provenance.json").read_text())
        for label in SOURCES
    }
    equal_fields = (
        "workflow_sha",
        "run_id",
        "run_attempt",
        "event",
        "ref",
        "cwd",
        "hostname",
        "kernel",
        "cpu_affinity",
        "rustc",
        "cargo",
        "libc",
        "environment",
        "input_sha256",
    )
    checks = {
        key: records["parent"][key] == records["child"][key] for key in equal_fields
    }
    cpu_keys = {
        "Architecture:",
        "CPU(s):",
        "Vendor ID:",
        "Model name:",
        "CPU family:",
        "Model:",
        "Stepping:",
        "Flags:",
        "Hypervisor vendor:",
    }
    cpu = {
        label: {
            item["field"]: item["data"]
            for item in record["cpu"]["lscpu"]
            if item["field"] in cpu_keys
        }
        for label, record in records.items()
    }
    checks["cpu_identity"] = cpu["parent"] == cpu["child"]
    write_json(
        root / "pair.json",
        {"source_order": list(SOURCES), "same_runner_checks": checks},
    )
    if not all(checks.values()):
        raise ValueError("Paired measurements have different recorded environments")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "finish", "compare"))
    parser.add_argument("root", type=Path)
    parser.add_argument("label", choices=SOURCES, nargs="?")
    args = parser.parse_args()
    if args.command == "compare":
        compare(args.root)
    elif args.label is None:
        parser.error("prepare and finish require a source label")
    elif args.command == "prepare":
        prepare(args.root, args.label)
    else:
        finish(args.root, args.label)


if __name__ == "__main__":
    main()
