"""Create repeatable OSV modification-time cache workloads."""

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
    args = parser.parse_args()
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
            str(Path(__file__).with_name("make_osv_record_fixtures.py")),
            "--directory",
            str(args.directory),
            "--packages",
            "24",
        ],
        check=True,
    )
    profiles = json.loads((args.directory / "osv-record-profiles.json").read_text())
    profiles["record-flaky"]["path_failures"] = {
        "/v1/vulns/OSV-BENCH-0000-0": {"status": 503, "count": 1}
    }
    profiles.update(
        {
            "stale-" + name: {
                **profile,
                "response_date": "Thu, 01 Jan 2026 00:00:00 GMT",
            }
            for name, profile in list(profiles.items())
        }
    )
    (args.directory / "osv-modified-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    for route in ("dense", "single", "shared", "empty"):
        configuration = json.loads(
            (args.directory / f"osv-record-{route}.json").read_text()
        )
        ignored = [
            value
            for identifier in sorted(configuration["vulnerabilities"])
            for value in ("--ignore", identifier)
        ]
        command = [
            "audit",
            "--preview-features",
            "audit,json-output",
            "--frozen",
            "--service-url",
            "{base}",
            "--output-format",
            "json",
            *ignored,
        ]
        (args.directory / f"setup-{route}.json").write_text(
            json.dumps([command], indent=2) + "\n"
        )
        original_manifest = json.loads(
            (args.directory / f"osv-record-{route}-fixtures.json").read_text()
        )
        wheels = [entry for entry in original_manifest if entry.get("kind") != "osv"]
        for validator, value in {
            "matching": "2026-01-01T00:00:00Z",
            "missing": None,
            "malformed": "invalid timestamp",
            "mismatched": "2026-01-02T00:00:00Z",
        }.items():
            fixture = dict(
                configuration,
                summary_modified={
                    identifier: value for identifier in configuration["vulnerabilities"]
                },
            )
            filename = f"osv-modified-{route}-{validator}.json"
            payload = (json.dumps(fixture, indent=2) + "\n").encode()
            (args.directory / filename).write_bytes(payload)
            entry = {
                "filename": filename,
                "url": "https://example.invalid/" + filename,
                "sha256": hashlib.sha256(payload).hexdigest(),
                "kind": "osv",
            }
            (args.directory / filename.replace(".json", "-fixtures.json")).write_text(
                json.dumps([entry, *wheels], indent=2) + "\n"
            )
    print(json.dumps({"directory": str(args.directory), "validators": 4}))


if __name__ == "__main__":
    main()
