"""Create a batch of wheel builds for repeat-publish index checks."""

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
    parser.add_argument("--builds", type=int, default=12)
    parser.add_argument("--projects", type=int, default=1)
    args = parser.parse_args()
    if args.builds < 1:
        parser.error("--builds must be positive")
    if args.projects < 1:
        parser.error("--projects must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    names = (
        ["uv-bench-publish"]
        if args.projects == 1
        else [f"uv-bench-publish-{index:03d}" for index in range(args.projects)]
    )
    manifest = [
        scheduling.wheel(args.directory, name, 1, None, build_tag=build)
        for name in names
        for build in range(1, args.builds + 1)
    ]
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "constrained": {"latency_ms": 250, "bytes_per_second": 125000},
        "revalidate": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "cache_control": "public, max-age=0",
        },
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {f"/simple/{names[0]}/": {"status": 503, "count": 1}},
        },
    }
    for filename, data in (
        ("publish-fixtures.json", manifest),
        ("publish-profiles.json", profiles),
    ):
        (args.directory / filename).write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps({"wheels": len(manifest), "directory": str(args.directory)}))


if __name__ == "__main__":
    main()
