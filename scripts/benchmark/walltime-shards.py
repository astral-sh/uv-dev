"""Partition all built walltime suites into bounded, independently uploaded runs."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path

MAX_SHARDS = 8


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
    command = ["cargo", "codspeed", "run", "-m", "walltime", "-p", "uv-bench"]
    for name in selected["benches"]:
        command.extend(["--bench", name])
    subprocess.run(command, cwd=root, check=True)


if __name__ == "__main__":
    main()
