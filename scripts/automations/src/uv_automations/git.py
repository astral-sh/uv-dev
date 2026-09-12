"""The Git operations needed by the automation workflows."""

import subprocess
from pathlib import Path

from uv_automations.models import CommitSha


def head(repository: Path) -> CommitSha:
    result = subprocess.run(
        ["git", "-C", str(repository), "rev-parse", "--verify", "HEAD^{commit}"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        timeout=30,
    )
    return CommitSha(result.stdout.strip())


def check_branch(name: str) -> None:
    result = subprocess.run(
        ["git", "check-ref-format", f"refs/heads/{name}"],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=30,
    )
    if result.returncode != 0:
        raise ValueError(f"Invalid Git branch name: {name!r}")
