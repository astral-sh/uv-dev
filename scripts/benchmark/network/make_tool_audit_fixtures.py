"""Create locked local tools and pinned OSV responses for connection benchmarks."""

from __future__ import annotations

import argparse
import hashlib
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
    if not 1 <= args.packages <= 1000:
        parser.error("--packages must be between 1 and 1000")
    args.directory.mkdir(parents=True, exist_ok=True)
    names = [f"uv-bench-audit-tool-{number:03d}" for number in range(args.packages)]
    wheels = [
        scheduling.wheel(args.directory, name, 1, None, console_script=True)
        for name in names
    ]
    setup = [
        [
            "tool",
            "install",
            "--preview-features",
            "tool-install-locks",
            "--no-index",
            "--python",
            "{python}",
            "{fixtures}/" + wheel["filename"],
        ]
        for wheel in wheels
    ]
    for route, pages in (("plain", 1), ("paginated", 2)):
        configuration = {
            "dependencies": {name: {"version": "1.0", "pages": pages} for name in names}
        }
        path = args.directory / f"tool-audit-{route}.json"
        path.write_text(json.dumps(configuration, indent=2) + "\n")
        entry = {
            "filename": path.name,
            "url": "https://example.invalid/" + path.name,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "kind": "osv",
        }
        (args.directory / f"tool-audit-{route}-fixtures.json").write_text(
            json.dumps([entry, *wheels], indent=2) + "\n"
        )
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
            "osv_failures": {f"{names[0]}:0": {"status": 503, "count": 1}},
        },
    }
    for name, value in (
        ("tool-audit-profiles.json", profiles),
        ("tool-audit-setup.json", setup),
        ("tool-audit-one-setup.json", setup[:1]),
        ("tool-audit-empty-setup.json", []),
    ):
        (args.directory / name).write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps({"tools": args.packages, "directory": str(args.directory)}))


if __name__ == "__main__":
    main()
