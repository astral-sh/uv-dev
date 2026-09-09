"""Git operations against an explicit automation checkout."""

import os
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Self

from uv_automations.models import CommitSha


def _environment(
    token_variable: str | None = None, *, directory: Path | None = None
) -> dict[str, str]:
    # An explicit checkout must not be redirected by inherited Git state. The
    # caller still owns authentication (for example, GH_TOKEN).
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("GIT_")
    }
    environment.update(
        GIT_ATTR_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_CONFIG_NOSYSTEM="1",
        GIT_GRAFT_FILE=os.devnull,
        GIT_NO_LAZY_FETCH="1",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_OPTIONAL_LOCKS="0",
        GIT_TERMINAL_PROMPT="0",
    )
    if directory is not None:
        # `Git` names the repository itself, not a directory from which Git
        # should discover some other repository in its ancestors.
        environment["GIT_CEILING_DIRECTORIES"] = str(directory.absolute().parent)
    if token_variable is not None:
        token = os.environ.get(token_variable)
        if not token:
            raise ValueError(f"Missing Git token environment: {token_variable}")
        environment["GH_TOKEN"] = token
    return environment


@dataclass(frozen=True, slots=True)
class Git:
    """Run Git against trusted metadata with explicit repository and credentials.

    Configuration-defined hooks and filters are not sandboxed by this adapter.
    Inspect an agent-owned checkout through `checkouts.inspect_candidate` first.
    """

    path: Path
    token_variable: str | None = field(default=None, kw_only=True)

    def with_token(self, variable: str | None) -> Self:
        return replace(self, token_variable=variable)

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
                "core.fsmonitor=false",
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
            env=_environment(self.token_variable, directory=self.path),
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
        env=_environment(),
        timeout=30,
    )
    if result.returncode != 0:
        raise ValueError(f"Invalid Git branch name: {name!r}")
