"""Create direct wheels and registry/build controls for no-dependency commands."""

from __future__ import annotations

import argparse
import importlib.util
import json
import shutil
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
    wheel = scheduling.wheel(
        args.directory, "uv-bench-direct-wheel", 1, "uv-bench-runtime==1.0"
    )
    replacement = scheduling.wheel(args.directory, "uv-bench-direct-wheel", 2, None)
    runtime = scheduling.wheel(args.directory, "uv-bench-runtime", 1, None)
    build = scheduling.wheel(args.directory, "uv-bench-build-dependency", 1, None)
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
    for filename, data in (
        ("direct-wheel-fixtures.json", [wheel, replacement, runtime, build]),
        ("direct-wheel-profiles.json", profiles),
    ):
        (args.directory / filename).write_text(json.dumps(data, indent=2) + "\n")
    (args.directory / "overrides.txt").write_text("uv-bench-direct-wheel==2.0\n")
    source = args.directory / "source-project"
    source.mkdir(exist_ok=True)
    shutil.copyfile(args.directory / wheel["filename"], source / wheel["filename"])
    (source / "pyproject.toml").write_text(
        '[project]\nname = "uv-bench-direct-wheel"\nversion = "1.0"\n'
        '[build-system]\nrequires = ["uv-bench-build-dependency==1.0"]\n'
        'build-backend = "backend"\nbackend-path = ["."]\n'
    )
    (source / "backend.py").write_text(
        "import shutil\nfrom pathlib import Path\nimport uv_bench_build_dependency\n\n"
        "def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):\n"
        f"    wheel = Path(__file__).with_name({wheel['filename']!r})\n"
        "    shutil.copyfile(wheel, Path(wheel_directory) / wheel.name)\n"
        "    return wheel.name\n"
    )
    print(json.dumps({"wheel": wheel["filename"], "sha256": wheel["sha256"]}))


if __name__ == "__main__":
    main()
