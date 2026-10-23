"""Create OSV page graphs with dense, sparse, and independent slow batches."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=3001)
    args = parser.parse_args()
    if not 1 <= args.packages <= 10000:
        parser.error("--packages must be between 1 and 10000")
    subprocess.run(
        [
            "uv",
            "--no-config",
            "run",
            "--no-project",
            "--offline",
            "--python",
            sys.executable,
            "python",
            "-S",
            str(Path(__file__).with_name("make_osv_fixtures.py")),
            "--directory",
            str(args.directory),
            "--packages",
            str(args.packages),
        ],
        check=True,
    )
    original = json.loads((args.directory / "osv-plain-fixtures.json").read_text())
    wheels = [entry for entry in original if entry.get("kind") != "osv"]
    assert len(wheels) == args.packages
    names = [f"uv-bench-osv-{index:04d}" for index in range(args.packages)]
    for route in ("plain", "skewed", "dense", "sparse"):
        dependencies = {}
        for index, name in enumerate(names):
            pages = 1
            if route == "skewed" and 1000 <= index < 2000:
                pages = 4
            elif route == "dense":
                pages = 2
            elif route == "sparse" and index % 1000 == 999:
                pages = 3
            dependencies[name] = {"version": "1.0", "pages": pages}
        configuration = {
            "record_query_keys": True,
            "dependencies": dependencies,
        }
        filename = f"osv-pagination-{route}.json"
        payload = (json.dumps(configuration, indent=2) + "\n").encode()
        (args.directory / filename).write_bytes(payload)
        entry = {
            "filename": filename,
            "url": "https://example.invalid/" + filename,
            "sha256": hashlib.sha256(payload).hexdigest(),
            "kind": "osv",
        }
        (args.directory / f"osv-pagination-{route}-fixtures.json").write_text(
            json.dumps([entry, *wheels], indent=2) + "\n"
        )
    profiles = json.loads((args.directory / "osv-profiles.json").read_text())
    profiles["flaky-page"] = {
        "latency_ms": 100,
        "bytes_per_second": 1250000,
        "osv_query_latency_ms": {"uv-bench-osv-0000:0": 600},
        "osv_failures": {"uv-bench-osv-1000:1": {"status": 503, "count": 1}},
    }
    (args.directory / "osv-pagination-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    print(json.dumps({"packages": args.packages, "routes": 4}))


if __name__ == "__main__":
    main()
