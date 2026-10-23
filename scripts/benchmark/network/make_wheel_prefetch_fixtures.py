"""Create an unused wheel on an index known to lack range support."""

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
    parser.add_argument("--payload-bytes", type=int, default=4 * 1024 * 1024)
    args = parser.parse_args()
    if args.payload_bytes < 0:
        parser.error("--payload-bytes must be nonnegative")
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(
            args.directory,
            "uv-bench-wheel-root",
            version,
            [f"uv-bench-wheel-gate=={gate}.0", f"uv-bench-wheel-choice{choice}"],
        )
        for version, gate, choice in ((1, 1, ""), (2, 1, "==1.0"), (3, 2, "==2.0"))
    ]
    for item in manifest:
        item["pep658"] = False
    manifest.extend(
        scheduling.wheel(
            args.directory,
            "uv-bench-wheel-gate",
            version,
            f"uv-bench-wheel-choice=={version}.0",
        )
        for version in (1, 2)
    )
    for version in (1, 2):
        item = scheduling.wheel(
            args.directory,
            "uv-bench-wheel-choice",
            version,
            None,
            payload_bytes=args.payload_bytes if version == 2 else 0,
        )
        item["pep658"] = False
        manifest.append(item)
    gate_paths = {
        f"/files/uv_bench_wheel_gate-{version}.0-py3-none-any.whl.metadata": 750
        for version in (1, 2)
    }
    unused_path = "/files/uv_bench_wheel_choice-2.0-py3-none-any.whl"
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0, "ranges": False},
        "slow": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "ranges": False,
            "path_latency_ms": gate_paths,
        },
        "constrained": {
            "latency_ms": 250,
            "bytes_per_second": 125000,
            "ranges": False,
            "path_latency_ms": gate_paths,
        },
        "range": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "ranges": True,
            "path_latency_ms": gate_paths,
        },
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "ranges": False,
            "path_latency_ms": gate_paths,
            "path_failures": {unused_path: {"status": 503, "count": 2}},
        },
    }
    sidecars = [
        {key: value for key, value in item.items() if key != "pep658"}
        if item["filename"].startswith("uv_bench_wheel_choice-")
        else item
        for item in manifest
    ]
    for name, value in (
        ("wheel-prefetch-fixtures.json", manifest),
        ("wheel-prefetch-sidecars.json", sidecars),
        ("wheel-prefetch-profiles.json", profiles),
    ):
        (args.directory / name).write_text(json.dumps(value, indent=2) + "\n")
    print(
        json.dumps({"distributions": len(manifest), "directory": str(args.directory)})
    )


if __name__ == "__main__":
    main()
