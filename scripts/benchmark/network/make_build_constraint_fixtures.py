"""Create workspace builds that share a pinned public constraints file."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import shutil
from pathlib import Path

AIRFLOW_COMMIT = "3675eaba7aaedde3eb68bc947a8ec8958fbb0050"
CONSTRAINTS_NAME = "airflow-3.1.0-constraints-3.12.txt"
CONSTRAINTS_SHA256 = "83607112d6b852e294f405811378db66aa7471982daa6e1ecdcd16a78d78c0a9"
CONSTRAINTS_SIZE = 17974

spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", Path(__file__).with_name("make_scheduling_fixtures.py")
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--constraints", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=16)
    args = parser.parse_args()
    if not 1 <= args.packages <= 100:
        parser.error("--packages must be between 1 and 100")
    constraints = args.constraints.read_bytes()
    if (
        len(constraints) != CONSTRAINTS_SIZE
        or hashlib.sha256(constraints).hexdigest() != CONSTRAINTS_SHA256
    ):
        parser.error("--constraints must contain the pinned Airflow 3.1.0 constraints")
    args.directory.mkdir(parents=True, exist_ok=True)
    (args.directory / CONSTRAINTS_NAME).write_bytes(constraints)
    workspace = args.directory / "workspace"
    workspace.mkdir(exist_ok=True)
    (workspace / "pyproject.toml").write_text(
        '[tool.uv.workspace]\nmembers = ["packages/*"]\n'
    )
    manifest = [
        {
            "kind": "raw",
            "filename": CONSTRAINTS_NAME,
            "url": f"https://raw.githubusercontent.com/apache/airflow/{AIRFLOW_COMMIT}/constraints-3.12.txt",
            "sha256": CONSTRAINTS_SHA256,
            "size": CONSTRAINTS_SIZE,
        }
    ]
    outputs = {}
    for index in range(1, args.packages + 1):
        name = f"uv-bench-build-{index:02}"
        wheel = scheduling.wheel(args.directory, name, 1, None)
        package = workspace / "packages" / name
        package.mkdir(parents=True, exist_ok=True)
        (package / "pyproject.toml").write_text(
            f'[project]\nname = "{name}"\nversion = "1.0"\n'
            'requires-python = ">=3.12"\n'
            '[build-system]\nrequires = []\nbuild-backend = "backend"\n'
            'backend-path = ["."]\n'
        )
        (package / "backend.py").write_text(
            "import shutil\nfrom pathlib import Path\n\n"
            "def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):\n"
            f"    wheel = Path(__file__).with_name({wheel['filename']!r})\n"
            "    shutil.copyfile(wheel, Path(wheel_directory) / wheel.name)\n"
            "    return wheel.name\n"
        )
        shutil.copyfile(args.directory / wheel["filename"], package / wheel["filename"])
        outputs[wheel["filename"]] = wheel["sha256"]
        manifest.append(wheel)
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "constrained": {"latency_ms": 150, "bytes_per_second": 125000},
        "high-latency": {"latency_ms": 600, "bytes_per_second": 1250000},
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 125000,
            "path_failures": {
                "/files/" + CONSTRAINTS_NAME: {"status": 503, "count": 1}
            },
        },
    }
    for profile in profiles.values():
        profile["cache_control"] = "no-store"
    for filename, value in (
        ("build-constraint-fixtures.json", manifest),
        ("build-constraint-profiles.json", profiles),
        ("build-constraint-outputs.json", outputs),
    ):
        (args.directory / filename).write_text(json.dumps(value, indent=2) + "\n")
    print(
        json.dumps(
            {
                "packages": args.packages,
                "constraints_sha256": CONSTRAINTS_SHA256,
                "constraints_bytes": CONSTRAINTS_SIZE,
                "expected_wheels": len(outputs),
            }
        )
    )


if __name__ == "__main__":
    main()
