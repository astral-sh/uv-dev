"""Partition all built walltime suites into bounded, independently uploaded runs."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

MAX_SHARDS = 8
# Approximate five-minute runtime units on the Linux walltime runners. New
# suites start at one unit; long-running suites need a larger scheduling weight.
DEFAULT_WEIGHT = 1
SUITE_WEIGHTS = {
    "build_frontend": 2,
    "cache_management": 2,
    "concurrent_environments": 3,
    "download_hashing": 2,
    "git_fetch": 3,
    "git_fetch_popular": 6,
    "git_monorepo": 5,
    "github_metadata": 3,
    "local_wheel_cache": 2,
    "lockfile": 3,
    "native_source_install": 2,
    "python_install": 2,
    "source_build_reuse": 11,
    "source_metadata": 5,
    "uv": 2,
}
PINNED_GIT_SUITES = {
    "git_fetch",
    "git_fetch_popular",
    "git_fetch_large",
    "git_monorepo",
}


def partition(names: list[str]) -> list[dict]:
    if not names or len(names) != len(set(names)):
        raise ValueError("Expected a non-empty set of unique benchmark suites")
    if any(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]*", name) is None for name in names):
        raise ValueError("Unexpected benchmark suite name")
    count = min(MAX_SHARDS, len(names))
    groups: list[list[str]] = [[] for _ in range(count)]
    loads = [0] * count
    for name in sorted(
        names, key=lambda name: (-SUITE_WEIGHTS.get(name, DEFAULT_WEIGHT), name)
    ):
        index = min(range(count), key=lambda index: (loads[index], index))
        groups[index].append(name)
        loads[index] += SUITE_WEIGHTS.get(name, DEFAULT_WEIGHT)
    return [
        {"index": index + 1, "total": count, "benches": sorted(group)}
        for index, group in enumerate(groups)
    ]


def built_suites(root: Path) -> list[str]:
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
    names = sorted(path.name for path in directory.iterdir() if path.is_file())
    if unknown := set(names) - declared:
        raise ValueError(f"Unexpected walltime build artifacts: {sorted(unknown)}")
    return names


def run_suites(root: Path, names: list[str]) -> None:
    command = ["cargo", "codspeed", "run", "-m", "walltime", "-p", "uv-bench"]
    # Git fetch measurements need the same modern Git on the build and runner hosts.
    # Keep the local-transport-only runtime scoped to Git transport suites.
    selected = [name for name in names if name in PINNED_GIT_SUITES]
    if selected:
        arguments = [argument for name in selected for argument in ("--bench", name)]
        subprocess.run(
            [
                sys.executable,
                str(root / "scripts/benchmark/git-runtime.py"),
                "run",
                "--",
                *command,
                *arguments,
            ],
            cwd=root,
            check=True,
        )
    remaining = [name for name in names if name not in PINNED_GIT_SUITES]
    if not remaining:
        return
    for name in remaining:
        command.extend(["--bench", name])
    subprocess.run(command, cwd=root, check=True)


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan")
    run = commands.add_parser("run")
    run.add_argument("shard", type=int)
    args = parser.parse_args()
    plan = root / ".cache/bench-walltime-shards.json"
    expected = {"version": 1, "shards": partition(built_suites(root))}
    if args.command == "plan":
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
    if json.loads(plan.read_text()) != expected:
        raise ValueError("The walltime shard plan does not match the built suites")
    if not 1 <= args.shard <= len(expected["shards"]):
        parser.error("Shard index is outside the prepared plan")
    selected = expected["shards"][args.shard - 1]
    print(
        f"Running walltime shard {args.shard}/{selected['total']}: {', '.join(selected['benches'])}",
        flush=True,
    )
    run_suites(root, selected["benches"])


if __name__ == "__main__":
    main()
