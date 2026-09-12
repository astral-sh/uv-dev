"""Export immutable source trees for offline build and discovery benchmarks."""

from __future__ import annotations

import argparse
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path


def main() -> None:
    if sys.version_info < (3, 12, 11):
        raise SystemExit("Source fixtures require Python 3.12.11 or newer")
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=root / ".cache/bench-sources")
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    git_fixtures = {
        item["name"]: item
        for item in json.loads(Path(__file__).with_name("git.json").read_text())
    }
    archives = {
        item["filename"]: item
        for item in json.loads(Path(__file__).with_name("fixtures.json").read_text())
    }
    environment = os.environ.copy()
    for name in (
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
    ):
        environment.pop(name, None)
    environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    for source in json.loads(Path(__file__).with_name("sources.json").read_text()):
        name = source["name"]
        if Path(name).name != name:
            raise ValueError(f"Invalid source fixture: {name}")
        destination = args.directory / name
        marker = args.directory / f".{name}.json"
        provenance = source | {
            "input": git_fixtures[source["git"]]
            if "git" in source
            else archives[source["archive"]]
        }
        if (
            destination.is_dir()
            and marker.is_file()
            and json.loads(marker.read_text()) == provenance
        ):
            continue
        with tempfile.TemporaryDirectory(dir=args.directory) as temporary:
            extracted = Path(temporary) / "source"
            extracted.mkdir()
            if "git" in source:
                fixture = git_fixtures[source["git"]]
                archive = subprocess.check_output(
                    [
                        "git",
                        "-C",
                        str(root / ".cache/bench-git" / f"{source['git']}.git"),
                        "archive",
                        "--format=tar",
                        fixture["commit"],
                    ],
                    env=environment,
                )
                with tarfile.open(fileobj=io.BytesIO(archive)) as archive:
                    archive.extractall(extracted, filter="data")
                tree = extracted
            else:
                with tarfile.open(
                    root / ".cache/bench-fixtures" / source["archive"]
                ) as archive:
                    archive.extractall(extracted, filter="data")
                tree = extracted / source["prefix"]
            if not any(
                (tree / filename).is_file()
                for filename in ("pyproject.toml", "setup.py")
            ):
                raise ValueError(f"Missing project metadata for {name}")
            if destination.exists():
                shutil.rmtree(destination)
            tree.replace(destination)
        marker.write_text(json.dumps(provenance, sort_keys=True) + "\n")
        files = [path for path in destination.rglob("*") if path.is_file()]
        print(
            f"{name}: {len(files)} files, {sum(p.stat().st_size for p in files):,} bytes"
        )


if __name__ == "__main__":
    main()
