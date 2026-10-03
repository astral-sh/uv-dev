"""Create an index large enough to measure repeated find-links downloads."""

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
    parser.add_argument("--versions", type=int, default=128)
    args = parser.parse_args()
    if args.versions < 1:
        parser.error("--versions must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(args.directory, "uv-bench-links", version, None)
        for version in range(1, args.versions + 1)
    ]
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "constrained": {"latency_ms": 250, "bytes_per_second": 125000},
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {"/flat/00": {"status": 503, "count": 1}},
        },
    }
    for filename, data in (
        ("duplicate-links-fixtures.json", manifest),
        ("duplicate-links-profiles.json", profiles),
    ):
        (args.directory / filename).write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps({"wheels": len(manifest), "directory": str(args.directory)}))


if __name__ == "__main__":
    main()
