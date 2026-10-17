"""Create pinned OSV identifiers and full records for pipelined audit benchmarks."""

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
    names = [f"uv-bench-osv-{index:04d}" for index in range(args.packages)]
    all_identifiers = set()
    for route in ("empty", "skewed", "dense", "shared", "paginated", "single"):
        dependencies = {}
        records = {}
        for index, name in enumerate(names):
            pages = [[]]
            if route == "skewed" and 1000 <= index < 1008:
                pages = [[f"OSV-BENCH-{index:04d}-0"]]
            elif route == "dense" and index < 24:
                pages = [[f"OSV-BENCH-{index:04d}-{page}"] for page in range(2)]
            elif route == "shared" and index % 1000 == 0:
                pages = [
                    ["OSV-BENCH-SHARED-0", "OSV-BENCH-SHARED-1"],
                    ["OSV-BENCH-SHARED-1", "OSV-BENCH-SHARED-2"],
                ]
            elif route == "paginated" and 1000 <= index < 1008:
                pages = [[f"OSV-BENCH-{index:04d}-{page}"] for page in range(3)]
            elif route == "single" and index == 0:
                pages = [["OSV-BENCH-0000-0"]]
            dependencies[name] = {
                "version": "1.0",
                "pages": len(pages),
                "vulns_by_page": pages,
            }
            for identifiers in pages:
                for identifier in identifiers:
                    records.setdefault(
                        identifier,
                        {
                            "id": identifier,
                            "schema_version": "1.7.0",
                            "modified": "2026-01-01T00:00:00Z",
                            "published": "2025-01-01T00:00:00Z",
                            "summary": "Synthetic benchmark advisory " + identifier,
                            "details": "Deterministic advisory description. " * 32,
                        },
                    )
        all_identifiers.update(records)
        configuration = {
            "record_query_keys": True,
            "dependencies": dependencies,
            "vulnerabilities": records,
        }
        filename = f"osv-record-{route}.json"
        payload = (json.dumps(configuration, indent=2) + "\n").encode()
        (args.directory / filename).write_bytes(payload)
        entry = {
            "filename": filename,
            "url": "https://example.invalid/" + filename,
            "sha256": hashlib.sha256(payload).hexdigest(),
            "kind": "osv",
        }
        (args.directory / f"osv-record-{route}-fixtures.json").write_text(
            json.dumps([entry, *wheels], indent=2) + "\n"
        )
    profiles = json.loads((args.directory / "osv-profiles.json").read_text())
    record_paths = {
        "/v1/vulns/" + identifier: 450 for identifier in sorted(all_identifiers)
    }
    profiles["uneven"]["path_latency_ms"] = record_paths
    profiles["batch-flaky"] = profiles.pop("flaky")
    profiles["record-flaky"] = {
        **profiles["uneven"],
        "path_failures": {"/v1/vulns/OSV-BENCH-1000-0": {"status": 503, "count": 1}},
    }
    (args.directory / "osv-record-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    print(json.dumps({"packages": args.packages, "routes": 6}))


if __name__ == "__main__":
    main()
