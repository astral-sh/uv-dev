"""Create independent, deterministic wheels for download-concurrency calibration."""

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
    parser.add_argument("--packages", type=int, default=64)
    parser.add_argument("--payload-bytes", type=int, default=65536)
    args = parser.parse_args()
    if args.packages < 1 or args.payload_bytes < 0:
        parser.error("use a positive package count and nonnegative payload size")
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(
            args.directory,
            f"uv-bench-concurrency-{package:03d}",
            1,
            None,
            payload_bytes=args.payload_bytes,
        )
        for package in range(args.packages)
    ]
    path = args.directory / "concurrency-fixtures.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(
        json.dumps(
            {
                "packages": len(manifest),
                "bytes": sum(item["size"] for item in manifest),
                "manifest": str(path),
            }
        )
    )


if __name__ == "__main__":
    main()
