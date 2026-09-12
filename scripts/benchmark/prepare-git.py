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


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=root / ".cache/bench-git")
    args = parser.parse_args()
    for fixture in json.loads(Path(__file__).with_name("git.json").read_text()):
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
        git(
            directory,
            "-c",
            "fsck.zeroPaddedFilemode=ignore",
            "fsck",
            "--no-reflogs",
            "--connectivity-only",
        )
        entries = git(directory, "ls-tree", "-rl", commit).splitlines()
        sizes = [int(entry.split()[3]) for entry in entries]
        print(f"{name}: {commit}, {len(entries)} files, {sum(sizes):,} bytes")


if __name__ == "__main__":
    main()
