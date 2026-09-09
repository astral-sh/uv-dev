"""Transfer verified commit ranges without checking out received code."""

import os
import stat
from dataclasses import dataclass
from pathlib import Path
from tempfile import TemporaryDirectory

from uv_automations.git import Git
from uv_automations.models import CommitSha


@dataclass(frozen=True, slots=True)
class CommitRange:
    base: CommitSha
    head: CommitSha

    def __post_init__(self) -> None:
        if self.base == self.head:
            raise ValueError("A commit artifact must extend its base")

    @property
    def reference(self) -> str:
        return f"refs/uv-automations/commits/{self.head}"


@dataclass(frozen=True, slots=True)
class CommitBundle:
    path: Path
    commits: CommitRange


def _require_commit(repository: Git, commit: CommitSha) -> None:
    if repository.output("cat-file", "-t", str(commit)) != "commit":
        raise ValueError(f"Expected a Git commit: {commit}")


def _require_ancestry(repository: Git, commits: CommitRange) -> None:
    _require_commit(repository, commits.base)
    _require_commit(repository, commits.head)
    if not repository.is_ancestor(commits.base, commits.head):
        raise ValueError("The artifact commit does not extend its trusted base")


def _verify_bundle(repository: Git, path: Path, commits: CommitRange) -> None:
    repository.command(("bundle", "verify", str(path)))
    if repository.output("bundle", "list-heads", str(path)) != (
        f"{commits.head} {commits.reference}"
    ):
        raise ValueError("The bundle does not advertise exactly the expected commit")


def persist_commit(
    repository: Git, commits: CommitRange, destination: Path
) -> CommitBundle:
    """Create a new bundle containing a strict extension of a known commit."""
    _require_ancestry(repository, commits)
    destination = destination.absolute()
    # A private staging directory plus an exclusive link avoids following an
    # existing destination symlink or replacing another producer's artifact.
    with TemporaryDirectory(prefix=".persist-commit-", dir=destination.parent) as root:
        staged = Path(root) / "commit.bundle"
        repository.command(("update-ref", commits.reference, str(commits.head), ""))
        try:
            repository.command(
                (
                    "bundle",
                    "create",
                    str(staged),
                    f"{commits.base}..{commits.reference}",
                )
            )
            _verify_bundle(repository, staged, commits)
            os.link(staged, destination)
        finally:
            # Delete only the exact temporary ref this call created.
            repository.command(
                ("update-ref", "-d", commits.reference, str(commits.head))
            )
    return CommitBundle(destination, commits)


def load_commit(repository: Git, bundle: Path, commits: CommitRange) -> CommitSha:
    """Import only objects, preserving the consumer's worktree, refs, and FETCH_HEAD."""
    bundle = bundle.absolute()
    try:
        is_regular = stat.S_ISREG(bundle.lstat().st_mode)
    except FileNotFoundError:
        is_regular = False
    if not is_regular:
        raise ValueError("The commit artifact must contain a regular Git bundle")

    _require_commit(repository, commits.base)
    _verify_bundle(repository, bundle, commits)
    repository.command(
        (
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            str(bundle),
            commits.reference,
        )
    )
    _require_ancestry(repository, commits)
    return commits.head
