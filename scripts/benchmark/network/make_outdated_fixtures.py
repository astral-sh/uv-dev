"""Create two wheel releases and indexes for outdated-version lookup benchmarks."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", Path(__file__).with_name("make_scheduling_fixtures.py")
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(args.directory, "uv-bench-outdated", version, None)
        for version in (1, 2)
    ]
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "slow-early-late": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {"/flat/00": 1500, "/flat/08": 1500},
        },
        "both-sources": {"latency_ms": 300, "bytes_per_second": 1250000},
        "slow-simple": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {"/simple/uv-bench-outdated/": 1500},
        },
        "slow-flat": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {"/flat/releases": 1500},
        },
    }
    setup = [
        ["venv", "--python", "{python}", "{work}/env"],
        [
            "pip",
            "install",
            "--python",
            "{work}/env/bin/python",
            "--no-index",
            "--no-deps",
            "{fixtures}/" + manifest[0]["filename"],
        ],
    ]
    for name, contents in (
        ("outdated-fixtures.json", manifest),
        ("outdated-profiles.json", profiles),
        ("outdated-setup.json", setup),
    ):
        (args.directory / name).write_text(json.dumps(contents, indent=2) + "\n")
    print(
        json.dumps(
            {"installed": manifest[0]["filename"], "latest": manifest[1]["filename"]}
        )
    )


if __name__ == "__main__":
    main()
