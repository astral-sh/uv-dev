"""Create local console-script releases for outdated-tool connection benchmarks."""

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
    parser.add_argument("--packages", type=int, default=12)
    args = parser.parse_args()
    if args.packages < 1:
        parser.error("--packages must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(
            args.directory,
            f"uv-bench-tool-{number:03d}",
            version,
            None,
            console_script=True,
        )
        for number in range(args.packages)
        for version in (1, 2)
    ]
    setup = [
        [
            "tool",
            "install",
            "--python",
            "{python}",
            "--default-index",
            "{index}",
            "{fixtures}/" + item["filename"],
        ]
        for item in manifest[::2]
    ]
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "connection": {
            "latency_ms": 20,
            "bytes_per_second": 1250000,
            "connection_latency_ms": 300,
        },
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {
                "/simple/uv-bench-tool-000/": {"status": 503, "count": 1}
            },
        },
    }
    for name, value in (
        ("tool-list-fixtures.json", manifest),
        ("tool-list-profiles.json", profiles),
        ("tool-list-setup.json", setup),
        ("tool-list-one-setup.json", setup[:1]),
        ("tool-list-empty-setup.json", []),
    ):
        (args.directory / name).write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps({"packages": args.packages, "wheels": len(manifest)}))


if __name__ == "__main__":
    main()
