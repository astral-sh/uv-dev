"""Prepare pinned release-tag histories for source-cache-key benchmarks."""

from __future__ import annotations

import json
import os
import re
import subprocess
from pathlib import Path


def git(directory: Path, *args: str, input: str | None = None) -> str:
    environment = os.environ.copy()
    for key in (
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
    ):
        environment.pop(key, None)
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_TERMINAL_PROMPT="0",
    )
    return subprocess.check_output(
        ["git", "-C", str(directory), *args], text=True, env=environment, input=input
    ).strip()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    output = root / ".cache/bench-git-tags"
    output.mkdir(parents=True, exist_ok=True)
    fixtures = json.loads(Path(__file__).with_name("git-tags.json").read_text())
    for fixture in fixtures:
        name, commit, tags = fixture["name"], fixture["commit"], fixture["tags"]
        if Path(name).name != name or not re.fullmatch("[0-9a-f]{40}", commit):
            raise ValueError(f"Invalid Git fixture: {name}")
        seed = root / ".cache/bench-git" / f"{name}.git"
        directory = output / f"{name}.git"
        if not directory.exists():
            git(
                output,
                "clone",
                "--quiet",
                "--bare",
                "--shared",
                str(seed),
                str(directory),
            )
        # Keep the archive relocatable when the walltime runner extracts `.cache`.
        (directory / "objects/info/alternates").write_text(
            os.path.relpath(seed / "objects", directory / "objects") + "\n"
        )
        for reference, oid in tags.items():
            if not reference.startswith("refs/tags/") or not re.fullmatch(
                "[0-9a-f]{40}", oid
            ):
                raise ValueError(f"Invalid pinned tag: {reference}")
            git(directory, "check-ref-format", reference)
        objects = (
            git(
                directory,
                "cat-file",
                "--batch-check",
                input="\n".join(tags.values()) + "\n",
            )
            if tags
            else ""
        )
        missing = [
            line.split()[0]
            for line in objects.splitlines()
            if line.endswith(" missing")
        ]
        if missing:
            git(
                directory,
                "fetch",
                "--quiet",
                "--no-tags",
                fixture["repository"],
                *missing,
            )
        changes = []
        for reference in git(
            directory, "for-each-ref", "--format=%(refname)", "refs/tags"
        ).splitlines():
            if reference not in tags:
                changes.append(f"delete {reference}")
        for reference, oid in tags.items():
            changes.append(f"update {reference} {oid}")
        if changes:
            git(directory, "update-ref", "--stdin", input="\n".join(changes) + "\n")
        git(directory, "update-ref", "--no-deref", "HEAD", commit)
        git(
            directory,
            "-c",
            "fsck.zeroPaddedFilemode=ignore",
            "fsck",
            "--no-reflogs",
            "--connectivity-only",
        )
        print(f"{name}: {len(tags)} pinned release tags")


if __name__ == "__main__":
    main()
