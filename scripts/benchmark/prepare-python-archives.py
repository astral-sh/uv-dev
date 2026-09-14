"""Prepare immutable host-native Python archives for install and uninstall workloads."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import tempfile
import urllib.request
from pathlib import Path

PREFIX = "https://github.com/astral-sh/python-build-standalone/releases/download/"


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-python-archives"
    )
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    operating_system = {"Darwin": "darwin", "Linux": "linux", "Windows": "windows"}[
        platform.system()
    ]
    architecture = {"arm64": "aarch64", "AMD64": "x86_64"}.get(
        platform.machine(), platform.machine()
    )
    libc = "gnu" if operating_system == "linux" else "none"
    suffix = f"-{operating_system}-{architecture}-{libc}"
    records = []
    for key, distribution in json.loads(
        Path(__file__).with_name("python-archives.json").read_text()
    ).items():
        if not key.endswith(suffix):
            continue
        url = distribution["url"]
        if not url.startswith(PREFIX):
            raise ValueError(f"Unexpected Python archive source: {key}")
        mirror_path = url.removeprefix(PREFIX)
        filename = (
            distribution["sha256"][:9]
            + "-"
            + mirror_path.rsplit("/", 1)[-1].replace("%2B", "-")
        )
        destination = args.directory / filename
        if not destination.is_file() or digest(destination) != distribution["sha256"]:
            with tempfile.TemporaryDirectory(dir=args.directory) as temporary:
                archive = Path(temporary) / filename
                with (
                    urllib.request.urlopen(url, timeout=120) as response,
                    archive.open("wb") as output,
                ):
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
                if digest(archive) != distribution["sha256"]:
                    raise ValueError(f"SHA-256 mismatch for {key}")
                archive.replace(destination)
        records.append(
            {
                "key": key,
                "version": ".".join(
                    str(distribution[field]) for field in ("major", "minor", "patch")
                ),
                "filename": filename,
                "mirror-path": mirror_path,
                "sha256": distribution["sha256"],
            }
        )
        print(filename)
    if not records:
        raise ValueError(f"No pinned Python archives for {suffix}")
    (args.directory / "manifest.json").write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    main()
