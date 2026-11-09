"""Prepare checksum-verified Windows Python archive bundles for shard timing trials."""

import hashlib
import json
import os
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# Hash partitions of the consumers on the experiment's pinned main revision.
PARTITIONS = {
    "3.10.16": (2,),
    "3.10.17": (1, 2, 3),
    "3.11.8": (1,),
    "3.12.4": (1, 3),
    "3.12.5": (1, 3),
    "3.12.6": (1, 2, 3),
    "3.12.8": (1, 2, 3),
    "3.12.9": (1, 2, 3),
    "3.12.11": (1, 2, 3),
    "3.13.1": (1,),
    "3.13.1+freethreaded": (1,),
    "3.14.5rc1": (3,),
    "3.15.0a2": (3,),
    "3.15.0rc3": (3,),
}


def main() -> None:
    catalogue = json.loads(Path("crates/uv-python-managed/download-metadata.json").read_text())
    root = Path(os.environ["RUNNER_TEMP"]) / "windows-python"
    original = root / "original"
    original.mkdir(parents=True)
    for partition in (1, 2, 3):
        (root / str(partition)).mkdir()

    def download(version: str) -> tuple[str, str, str]:
        entry = catalogue[f"cpython-{version}-windows-x86_64-none"]
        url = entry["url"]
        digest = entry["sha256"]
        # Match the archive name used by ManagedPythonDownload in UV_PYTHON_CACHE_DIR.
        filename = digest[:9] + "-" + url.rsplit("/", 1)[1].replace("%2B", "-")
        archive = original / filename
        prefix = "https://github.com/astral-sh/python-build-standalone/releases/download/"
        if not url.startswith(prefix):
            raise ValueError(f"Unexpected archive URL: {url}")
        mirror = "https://releases.astral.sh/github/python-build-standalone/releases/download/"
        subprocess.run(
            ["curl", "--fail", "--silent", "--show-error", "--location", "--retry", "3",
             "--max-time", "180", "--output", str(archive), mirror + url.removeprefix(prefix)],
            check=True,
        )
        with archive.open("rb") as source:
            actual = hashlib.file_digest(source, "sha256").hexdigest()
        if actual != digest:
            raise ValueError(f"Unexpected SHA-256 for {filename}: {actual}")
        print(f"{version} {archive.stat().st_size} bytes, partitions {PARTITIONS[version]}")
        return version, filename, digest

    checksums = {partition: [] for partition in (1, 2, 3)}
    with ThreadPoolExecutor(max_workers=4) as workers:
        for version, filename, digest in workers.map(download, PARTITIONS):
            for partition in PARTITIONS[version]:
                (root / str(partition) / filename).hardlink_to(original / filename)
                checksums[partition].append(f"{digest}  {filename}\n")
    for partition, lines in checksums.items():
        (root / str(partition) / "checksums.sha256").write_text("".join(lines))

    jobs = []
    for trial in range(1, 9):
        arms = ("main", "preload") if trial % 2 else ("preload", "main")
        for partition in (1, 2, 3):
            for arm in arms:
                jobs.append({"trial": trial, "partition": partition, "arm": arm})
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        print("matrix=" + json.dumps({"include": jobs}), file=output)


if __name__ == "__main__":
    main()
