"""Partition all built walltime suites into bounded, independently uploaded runs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import BinaryIO

MAX_SHARDS = 8
PLAN_VERSION = 2


def stream_digest(stream: BinaryIO) -> str:
    hasher = hashlib.sha256()
    while chunk := stream.read(1024 * 1024):
        hasher.update(chunk)
    return hasher.hexdigest()


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return stream_digest(stream)


def output(root: Path, *command: str) -> str:
    return subprocess.check_output(command, cwd=root, text=True).strip()


def cargo_codspeed_version(
    root: Path,
    *,
    cargo: str = "cargo",
    environment: Mapping[str, str] | None = None,
) -> str:
    result = subprocess.run(
        [cargo, "codspeed", "--version"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    # cargo-codspeed 5.0.1 sends Clap's display-version result through its error
    # handler. Other nonzero exits are not version responses.
    if (
        result.returncode == 1
        and result.stdout == ""
        and result.stderr.rstrip("\r\n") == "cargo-codspeed 5.0.1"
    ):
        return "cargo-codspeed 5.0.1"
    result.check_returncode()
    version = result.stdout.strip()
    if (
        result.stderr
        or re.fullmatch(
            r"cargo-codspeed [0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?",
            version,
        )
        is None
    ):
        raise ValueError("Unexpected cargo-codspeed version response")
    return version


def source_identity(root: Path) -> dict:
    """Identify tracked source without treating generated fixture files as source edits."""
    difference = subprocess.check_output(
        ["git", "diff", "--binary", "--no-ext-diff", "--no-textconv", "HEAD", "--"],
        cwd=root,
    )
    return {
        "commit": output(root, "git", "rev-parse", "HEAD"),
        "tree": output(root, "git", "rev-parse", "HEAD^{tree}"),
        "tracked_working_tree_dirty": bool(difference),
        "tracked_diff_sha256": hashlib.sha256(difference).hexdigest(),
        "cargo_lock_sha256": digest(root / "Cargo.lock"),
        "rust_toolchain_sha256": digest(root / "rust-toolchain.toml"),
    }


def producer_metadata(root: Path) -> dict:
    return {
        "rustc": output(root, "rustc", "-Vv"),
        "cargo_codspeed": cargo_codspeed_version(root),
        "working_tree_status": output(
            root, "git", "status", "--porcelain=v1", "--untracked-files=normal"
        ).splitlines(),
        "github": {
            key: os.environ[key]
            for key in (
                "GITHUB_REPOSITORY",
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_JOB",
                "GITHUB_WORKFLOW_REF",
                "GITHUB_WORKFLOW_SHA",
            )
            if key in os.environ
        },
    }


def partition(names: list[str]) -> list[dict]:
    if not names or len(names) != len(set(names)):
        raise ValueError("Expected a non-empty set of unique benchmark suites")
    if any(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]*", name) is None for name in names):
        raise ValueError("Unexpected benchmark suite name")
    names = sorted(names)
    count = min(MAX_SHARDS, len(names))
    return [
        {"index": index + 1, "total": count, "benches": names[index::count]}
        for index in range(count)
    ]


def built_artifacts(root: Path) -> dict[str, Path]:
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--locked",
                "--no-deps",
                "--format-version",
                "1",
            ],
            cwd=root,
            text=True,
        )
    )
    package = next(item for item in metadata["packages"] if item["name"] == "uv-bench")
    declared = {
        target["name"] for target in package["targets"] if "bench" in target["kind"]
    }
    directory = Path(metadata["target_directory"]) / "codspeed/walltime/uv-bench"
    artifacts = {
        path.name: path for path in sorted(directory.iterdir()) if path.is_file()
    }
    if unknown := set(artifacts) - declared:
        raise ValueError(f"Unexpected walltime build artifacts: {sorted(unknown)}")
    return artifacts


def artifact_identity(path: Path) -> dict:
    with path.open("rb") as stream:
        return {
            "sha256": stream_digest(stream),
            "size": os.fstat(stream.fileno()).st_size,
        }


def prepare_plan(source: dict, producer: dict, artifacts: dict[str, Path]) -> dict:
    return {
        "version": PLAN_VERSION,
        "source": source,
        "producer": producer,
        "artifacts": {
            name: artifact_identity(path) for name, path in sorted(artifacts.items())
        },
        "shards": partition(list(artifacts)),
    }


def verify_plan(
    plan: dict, source: dict, artifacts: dict[str, Path], shard: int
) -> dict:
    expected = partition(list(artifacts))
    if plan.get("version") != PLAN_VERSION or plan.get("shards") != expected:
        raise ValueError("The walltime shard plan does not match the built suites")
    if plan.get("source") != source:
        raise ValueError("The walltime shard plan belongs to different tracked source")
    if set(plan.get("artifacts", {})) != set(artifacts):
        raise ValueError("The walltime shard plan has a different artifact inventory")
    if not 1 <= shard <= len(expected):
        raise ValueError("Shard index is outside the prepared plan")
    selected = expected[shard - 1]
    # Other shards verify their own binaries; hashing the whole build in every job
    # would add unrelated filesystem work before each measurement.
    for name in selected["benches"]:
        if plan["artifacts"][name] != artifact_identity(artifacts[name]):
            raise ValueError(
                f"Walltime benchmark artifact does not match its plan: {name}"
            )
    return selected


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan")
    run = commands.add_parser("run")
    run.add_argument("shard", type=int)
    args = parser.parse_args()
    plan = root / ".cache/bench-walltime-shards.json"
    artifacts = built_artifacts(root)
    source = source_identity(root)
    if args.command == "plan":
        expected = prepare_plan(source, producer_metadata(root), artifacts)
        plan.parent.mkdir(parents=True, exist_ok=True)
        plan.write_text(json.dumps(expected, indent=2) + "\n")
        matrix = {
            "include": [
                {"index": item["index"], "total": item["total"]}
                for item in expected["shards"]
            ]
        }
        if output := os.environ.get("GITHUB_OUTPUT"):
            with Path(output).open("a") as stream:
                print(
                    "matrix=" + json.dumps(matrix, separators=(",", ":")), file=stream
                )
        print(json.dumps(expected, indent=2))
        return
    selected = verify_plan(json.loads(plan.read_text()), source, artifacts, args.shard)
    print(
        f"Running walltime shard {args.shard}/{selected['total']}: {', '.join(selected['benches'])}",
        flush=True,
    )
    command = ["cargo", "codspeed", "run", "-m", "walltime", "-p", "uv-bench"]
    for name in selected["benches"]:
        command.extend(["--bench", name])
    subprocess.run(command, cwd=root, check=True)


if __name__ == "__main__":
    main()
