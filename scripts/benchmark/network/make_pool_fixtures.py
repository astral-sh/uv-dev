"""Create a wide dependency graph with separated HTTP request bursts."""

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
    parser.add_argument("--width", type=int, default=50)
    args = parser.parse_args()
    if args.width < 1:
        parser.error("--width must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    front = [f"uv-bench-pool-front-{index:02}" for index in range(args.width)]
    back = [f"uv-bench-pool-back-{index:02}" for index in range(args.width)]
    gate = "uv-bench-pool-gate"
    manifest = [
        scheduling.wheel(args.directory, name, 1, f"{gate}==1.0") for name in front
    ]
    manifest.append(
        scheduling.wheel(args.directory, gate, 1, [f"{name}==1.0" for name in back])
    )
    manifest.extend(scheduling.wheel(args.directory, name, 1, None) for name in back)
    (args.directory / "pool-fixtures.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    dependencies = ",\n".join(f'    "{name}==1.0"' for name in front)
    (args.directory / "pool-project.toml").write_text(
        '[project]\nname = "poolbench-root"\nversion = "1.0.0"\n'
        'requires-python = ">=3.12"\ndependencies = [\n' + dependencies + "\n]\n"
    )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "gate": {
            "latency_ms": 0,
            "bytes_per_second": 0,
            "path_latency_ms": {f"/simple/{gate}/": 1000},
        },
        "connection": {
            "latency_ms": 0,
            "bytes_per_second": 0,
            "connection_latency_ms": 300,
            "path_latency_ms": {f"/simple/{gate}/": 1000},
        },
    }
    (args.directory / "pool-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    print(json.dumps({"wheels": len(manifest), "width": args.width}))


if __name__ == "__main__":
    main()
