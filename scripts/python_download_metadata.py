"""Export bundled catalogs for uv versions that require a JSON download manifest.

The NDJSON catalogs are the source of truth. This adapter is used by CI to install
test interpreters with a released uv before building the checkout.
"""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Iterator
from pathlib import Path

CATALOGS = (
    Path(__file__).parent.parent / "crates" / "uv-python-managed" / "download-metadata"
)
PROVIDERS = ("python-build-standalone", "pypy", "graalpy", "pyodide")


def iter_downloads(provider: str, content: str) -> Iterator[dict]:
    """Adapt selected provider records to the public JSON manifest format."""
    implementation = (
        "cpython" if provider in {"python-build-standalone", "pyodide"} else provider
    )
    for line in content.splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        version, _, build = record["version"].partition("+")
        match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)((?:a|b|rc)\d+)?", version)
        if match is None:
            raise ValueError(f"Invalid Python version: {version}")
        major, minor, patch = map(int, match.group(1, 2, 3))
        prerelease = match.group(4) or ""
        for artifact in record["artifacts"]:
            platform = artifact["platform"]
            build_options = []
            for part in artifact["variant"].split("+"):
                if part in {
                    "shared-freethreaded",
                    "shared-noopt",
                    "shared-pgo",
                    "static-noopt",
                }:
                    build_options.extend(part.split("-"))
                else:
                    build_options.append(part)
            while "-" in platform:
                prefix, _, option = platform.rpartition("-")
                if option not in {"static", "shared", "debug", "freethreaded"}:
                    break
                platform = prefix
                build_options.append(option)
            if (
                "static" in build_options
                or {"gil", "freethreaded"}.issubset(build_options)
                or len(platform.split("-")) > 4
            ):
                continue
            arch, _, operating_system, *environment = platform.split("-")
            arch = {"ppc64": "powerpc64", "ppc64le": "powerpc64le"}.get(arch, arch)
            arch_family, separator, arch_variant = arch.rpartition("_v")
            if separator:
                arch_variant = "v" + arch_variant
            else:
                arch_family, arch_variant = arch, None
            libc = (
                environment[0]
                if operating_system in {"linux", "emscripten"} and environment
                else "none"
            )
            variant = (
                "+".join(
                    option
                    for option in ("freethreaded", "debug")
                    if option in build_options
                )
                or None
            )
            yield {
                "name": implementation,
                "arch": {"family": arch_family, "variant": arch_variant},
                "os": operating_system,
                "libc": libc,
                "major": major,
                "minor": minor,
                "patch": patch,
                "prerelease": prerelease,
                "url": artifact["url"],
                "sha256": artifact.get("sha256"),
                "variant": variant,
                "build": build or None,
            }


def download_key(download: dict) -> str:
    version = f"{download['major']}.{download['minor']}.{download['patch']}{download['prerelease']}"
    if download["variant"]:
        version += "+" + download["variant"]
    arch = download["arch"]["family"]
    if download["arch"]["variant"]:
        arch += "_" + download["arch"]["variant"]
    return f"{download['name']}-{version}-{download['os']}-{arch}-{download['libc']}"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    downloads = {
        download_key(download): download
        for provider in PROVIDERS
        for download in iter_downloads(
            provider, (CATALOGS / f"{provider}.ndjson").read_text()
        )
    }
    args.output.write_text(json.dumps(downloads, indent=2) + "\n", newline="\n")


if __name__ == "__main__":
    main()
