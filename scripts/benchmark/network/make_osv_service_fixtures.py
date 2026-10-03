"""Create repeatable OSV service-switching cache workloads."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


def audit_command(service: str, ignored: list[str]) -> list[str]:
    return [
        "audit",
        "--preview-features",
        "audit,json-output",
        "--frozen",
        "--service-url",
        "{base}/" + service + "/",
        "--output-format",
        "json",
        *ignored,
    ]


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
    prefixes = ["/first", "/second"]
    profiles = json.loads((args.directory / "osv-record-profiles.json").read_text())
    profiles["record-flaky"]["path_failures"] = {
        "/v1/vulns/OSV-BENCH-0000-0": {"status": 503, "count": 1}
    }
    for profile in profiles.values():
        profile["osv_path_prefixes"] = prefixes
        for field in ("path_latency_ms", "path_failures"):
            routes = profile.get(field, {})
            profile[field] = routes | {
                prefix + route: value
                for prefix in prefixes
                for route, value in routes.items()
            }
    (args.directory / "osv-service-profiles.json").write_text(
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

        for strategy, services in {
            "cold": (),
            "single": ("first",),
            "alternate": ("first", "second"),
            "reverse": ("second", "first"),
            "return": ("first", "second", "first"),
        }.items():
            (args.directory / f"setup-{route}-{strategy}.json").write_text(
                json.dumps(
                    [audit_command(service, ignored) for service in services], indent=2
                )
                + "\n"
            )
    print(json.dumps({"directory": str(args.directory), "services": len(prefixes)}))


if __name__ == "__main__":
    main()
