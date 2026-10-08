"""Refresh reviewed package graphs and their archive identities."""

import argparse
import csv
import json
import re
import subprocess
import tempfile
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from package_fixtures import FIXTURES, load_profiles, normalize_name


def resolve(uv, requirements, profile):
    with tempfile.TemporaryDirectory() as temporary:
        source = Path(temporary) / "requirements.in"
        destination = Path(temporary) / "requirements.txt"
        source.write_text(
            "".join(
                f"{name}=={version}\n" for name, version in sorted(requirements.items())
            )
        )
        subprocess.run(
            [
                uv,
                "--no-config",
                "--no-python-downloads",
                "pip",
                "compile",
                str(source),
                "--python-version",
                profile["resolution_python"],
                "--python-platform",
                profile["resolution_platform"],
                "--no-header",
                "--no-annotate",
                "--output-file",
                str(destination),
            ],
            check=True,
        )
        result = {}
        for line in destination.read_text().splitlines():
            if not line or line.startswith("#"):
                continue
            match = re.fullmatch(r"([A-Za-z0-9_.-]+)==([^\s;]+)", line)
            if match is None:
                raise ValueError("Expected an exact unmarked requirement: " + line)
            result[normalize_name(match[1])] = match[2]
        return result


def artifacts(package):
    name, version = package
    with urllib.request.urlopen(
        f"https://pypi.org/pypi/{name}/{version}/json", timeout=60
    ) as response:
        release = json.load(response)
    return [
        {
            "name": name,
            "version": version,
            "filename": artifact["filename"],
            "url": artifact["url"],
            "sha256": artifact["digests"]["sha256"],
            "requires_python": artifact.get("requires_python") or "",
            "yanked": "true" if artifact.get("yanked") else "",
            "yanked_reason": artifact.get("yanked_reason") or "",
        }
        for artifact in release["urls"]
        if artifact["packagetype"] in ("sdist", "bdist_wheel")
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", default="uv", help="uv binary used to refresh graphs")
    parser.add_argument(
        "--resolve",
        action="append",
        default=[],
        metavar="PROFILE",
        help="Refresh this profile's graphs from its reviewed exact root requirements",
    )
    args = parser.parse_args()
    profiles = load_profiles()
    for name in args.resolve:
        profile = profiles[name]
        profile["requirements"] = resolve(args.uv, profile["roots"], profile)
        profile["build_requirements"] = resolve(
            args.uv, profile["build_roots"], profile
        )
    packages = set()
    for profile in profiles.values():
        packages.update(profile["requirements"].items())
        packages.update(profile["build_requirements"].items())
    rows = []
    with ThreadPoolExecutor(max_workers=4) as executor:
        for result in executor.map(artifacts, sorted(packages)):
            if not result:
                raise ValueError("A selected release has no supported archives")
            rows.extend(result)
    fields = (
        "name",
        "version",
        "filename",
        "url",
        "sha256",
        "requires_python",
        "yanked",
        "yanked_reason",
    )
    with (FIXTURES / "artifacts.tsv").open("w", newline="", encoding="utf-8") as target:
        writer = csv.DictWriter(
            target, fieldnames=fields, delimiter="\t", lineterminator="\n"
        )
        writer.writeheader()
        writer.writerows(
            sorted(rows, key=lambda row: (row["name"], row["version"], row["filename"]))
        )
    if args.resolve:
        (FIXTURES / "profiles.json").write_text(
            json.dumps(profiles, indent=2, sort_keys=True) + "\n"
        )
    print(f"Recorded {len(rows)} archives for {len(packages)} exact releases")


if __name__ == "__main__":
    main()
