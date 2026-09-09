"""Git operations against an explicit automation checkout."""

import os
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from uv_automations.models import CommitSha


def _environment() -> dict[str, str]:
    # An explicit checkout must not be redirected by inherited Git state. The
    # caller still owns authentication (for example, GH_TOKEN).
    excluded = {
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    }
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in excluded and not key.startswith("GIT_CONFIG")
    }
    environment.update(
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_NOSYSTEM="1",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_TERMINAL_PROMPT="0",
    )
    return environment


@dataclass(frozen=True, slots=True)
class Git:
    """Run Git without hooks, replacement objects, or inherited repository state."""

    path: Path

    def command(
        self,
        arguments: Sequence[str],
        *,
        check: bool = True,
        input: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "git",
                "-C",
                str(self.path),
                "-c",
                f"core.hooksPath={os.devnull}",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                *arguments,
            ],
            input=input,
            check=check,
            text=True,
            capture_output=True,
            env=_environment(),
            timeout=120,
        )

    def output(self, *arguments: str) -> str:
        return self.command(arguments).stdout.rstrip("\r\n")

    def resolve_commit(self, revision: str) -> CommitSha:
        return CommitSha(
            self.output(
                "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"
            )
        )

    def is_ancestor(self, ancestor: CommitSha, descendant: CommitSha) -> bool:
        result = self.command(
            ("merge-base", "--is-ancestor", str(ancestor), str(descendant)),
            check=False,
        )
        if result.returncode not in (0, 1):
            result.check_returncode()
        return result.returncode == 0


def head(repository: Path) -> CommitSha:
    return Git(repository).resolve_commit("HEAD")


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
