"""Prepare pinned Git repositories for offline benchmark workloads."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


def git(directory: Path, *args: str) -> str:
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
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_TERMINAL_PROMPT="0",
    )
    return subprocess.check_output(
        ["git", "-C", str(directory), *args], text=True, env=environment
    ).strip()


def configure_upload_pack(directory: Path) -> None:
    # The local transport must advertise the same filtering capability as the
    # upstream hosts, including requests for trees and blobs by object ID.
    git(directory, "config", "uploadpack.allowFilter", "true")
    git(directory, "config", "uploadpack.allowAnySHA1InWant", "true")


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=root / ".cache/bench-git")
    parser.add_argument(
        "--manifest", type=Path, default=Path(__file__).with_name("git.json")
    )
    args = parser.parse_args()
    for fixture in json.loads(args.manifest.read_text()):
        name, commit, reference = (
            fixture["name"],
            fixture["commit"],
            fixture["reference"],
        )
        if Path(name).name != name or not re.fullmatch("[0-9a-f]{40}", commit):
            raise ValueError(f"Invalid Git fixture: {name}")
        directory = args.directory / f"{name}.git"
        directory.mkdir(parents=True, exist_ok=True)
        if not (directory / "HEAD").is_file():
            git(directory, "-c", "init.templateDir=", "init", "--bare", "--quiet")
        configure_upload_pack(directory)
        # Some upstream histories contain known malformed legacy objects. Keep
        # fsck enabled while exempting only the exact objects in the manifest.
        fsck_options = []
        if skipped := fixture.get("fsck-skip", []):
            if any(not re.fullmatch("[0-9a-f]{40}", item) for item in skipped):
                raise ValueError(f"Invalid fsck skip list for {name}")
            skip_list = directory / "info/bench-fsck-skip-list"
            skip_list.write_text("\n".join(skipped) + "\n")
            fsck_options = [
                "-c",
                f"fetch.fsck.skipList={skip_list.resolve()}",
                "-c",
                f"fsck.skipList={skip_list.resolve()}",
            ]
        try:
            present = git(directory, "rev-parse", "--verify", "--quiet", reference)
        except subprocess.CalledProcessError:
            present = None
        shallow = git(directory, "rev-parse", "--is-shallow-repository") == "true"
        if present != commit or shallow:
            git(
                directory,
                "-c",
                "fetch.fsckObjects=true",
                # Early Git versions wrote this legacy tree encoding in Flask's history.
                "-c",
                "fetch.fsck.zeroPaddedFilemode=ignore",
                *fsck_options,
                "fetch",
                "--quiet",
                "--no-tags",
                *(["--unshallow"] if shallow else []),
                fixture["repository"],
                commit,
            )
            actual = git(directory, "rev-parse", "FETCH_HEAD^{commit}")
            if actual != commit:
                raise ValueError(f"Unexpected Git commit for {name}: {actual}")
            # Capture the real source tree at an immutable point in the upstream ref.
            git(directory, "update-ref", reference, commit)
        git(directory, "update-ref", "--no-deref", "HEAD", commit)
        for revision in fixture.get("revisions", []):
            if not re.fullmatch("[0-9a-f]{40}", revision["commit"]):
                raise ValueError(f"Invalid Git revision: {revision}")
            # Historical benchmark revisions must be part of the pinned history.
            git(directory, "merge-base", "--is-ancestor", revision["commit"], commit)
        git(
            directory,
            "-c",
            "fsck.zeroPaddedFilemode=ignore",
            *fsck_options,
            "fsck",
            "--no-reflogs",
            "--connectivity-only",
        )
        entries = git(directory, "ls-tree", "-rl", commit).splitlines()
        sizes = [
            int(entry.split()[3]) for entry in entries if entry.split()[1] == "blob"
        ]
        for package in fixture.get("packages", []):
            subdirectory = Path(package["subdirectory"])
            if subdirectory.is_absolute() or ".." in subdirectory.parts:
                raise ValueError(f"Invalid Git package: {package}")
            git(directory, "cat-file", "-e", f"{commit}:{subdirectory}/pyproject.toml")
        print(f"{name}: {commit}, {len(sizes)} files, {sum(sizes):,} bytes")


if __name__ == "__main__":
    main()
