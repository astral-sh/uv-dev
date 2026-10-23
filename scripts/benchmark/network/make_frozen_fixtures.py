"""Create a locked-wheel installation workload with an unused find-links source."""

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
    manifest = [scheduling.wheel(args.directory, "uv-bench-frozen", 1, None)]
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 300, "bytes_per_second": 1250000},
        "slow-unused": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {"/flat/extra": 1500},
        },
        "flaky-unused": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {"/flat/extra": {"status": 503, "count": 2}},
        },
    }
    setup = [["lock", "--python", "{python}", "--default-index", "{index}"]]
    for name, contents in (
        ("frozen-fixtures.json", manifest),
        ("frozen-profiles.json", profiles),
        ("frozen-setup.json", setup),
    ):
        (args.directory / name).write_text(json.dumps(contents, indent=2) + "\n")
    (args.directory / "frozen-project.toml").write_text(
        '[project]\nname = "frozen-wheel-bench"\nversion = "1.0.0"\n'
        'requires-python = ">=3.12"\ndependencies = ["uv-bench-frozen==1.0"]\n'
    )
    print(
        json.dumps({"wheel": manifest[0]["filename"], "sha256": manifest[0]["sha256"]})
    )


if __name__ == "__main__":
    main()
