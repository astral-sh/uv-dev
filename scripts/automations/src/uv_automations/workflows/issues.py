"""Prepare verified GitHub issue context for read-only automation jobs."""

import errno
import json
import os
from contextlib import ExitStack
from dataclasses import dataclass
from pathlib import Path

from uv_automations.github import IssueReader
from uv_automations.models import Issue, IssueRef


@dataclass(frozen=True, slots=True)
class PreparedIssue:
    issue: Issue
    path: Path


@dataclass(frozen=True, slots=True)
class _Destination:
    root: Path
    relative: Path

    @property
    def path(self) -> Path:
        return self.root / self.relative


def _open_flags() -> tuple[int, int]:
    if (
        os.open not in os.supports_dir_fd
        or not getattr(os, "O_DIRECTORY", 0)
        or not getattr(os, "O_NOFOLLOW", 0)
    ):
        raise OSError(
            errno.ENOTSUP,
            "Secure issue context creation requires no-follow directory descriptors",
        )
    return (
        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
    )


def _resolve_destination(
    destination: Path, *, workspace: Path, runner_temp: Path
) -> _Destination:
    if any(character in str(destination) for character in "\r\n\0"):
        raise ValueError("The issue destination must be a single-line path")
    if destination.name in {"", ".", ".."}:
        raise ValueError("The issue destination must name a file")
    if ".." in destination.parts:
        raise ValueError("The issue destination must not traverse parent directories")

    # Only the caller-provided roots are trusted enough to resolve. Keep every
    # destination component intact so descriptor traversal can reject symlinks.
    roots = tuple(
        (Path(os.path.abspath(root)), root.resolve(strict=True))
        for root in (workspace, runner_temp)
    )
    if not all(canonical.is_dir() for _, canonical in roots):
        raise ValueError("The issue destination roots must be directories")

    if not destination.is_absolute():
        selected = _Destination(roots[0][1], destination)
    else:
        matches = [
            (len(alias.parts), _Destination(canonical, destination.relative_to(alias)))
            for configured, canonical in roots
            for alias in (configured, canonical)
            if destination.is_relative_to(alias)
        ]
        if not matches:
            raise ValueError(
                "The issue destination must be inside the runner or workspace"
            )
        _, selected = max(matches, key=lambda match: match[0])
    if not selected.relative.parts:
        raise ValueError("The issue destination must name a file")
    if any(character in str(selected.path) for character in "\r\n\0"):
        raise ValueError("The issue destination must be a single-line path")
    return selected


def _open_directory(
    name: str, flags: int, descriptors: ExitStack, *, parent: int | None = None
) -> int:
    try:
        descriptor = os.open(name, flags, dir_fd=parent)
    except OSError as error:
        if error.errno in {errno.ELOOP, errno.ENOTDIR}:
            raise ValueError(
                "The issue destination must not have symbolic-link parents"
            ) from error
        raise
    descriptors.callback(os.close, descriptor)
    return descriptor


def _open_parent(destination: _Destination, flags: int, descriptors: ExitStack) -> int:
    # Start at the filesystem root, then anchor every subsequent component to
    # an already-open directory. Even a swapped root ancestor cannot redirect
    # the final exclusive create through a symlink.
    descriptor = _open_directory(destination.root.anchor, flags, descriptors)
    for component in (*destination.root.parts[1:], *destination.relative.parts[:-1]):
        descriptor = _open_directory(component, flags, descriptors, parent=descriptor)
    return descriptor


def prepare_issue(
    reader: IssueReader,
    reference: IssueRef,
    destination: Path,
    *,
    workspace: Path,
    runner_temp: Path,
) -> PreparedIssue:
    directory_flags, file_flags = _open_flags()
    target = _resolve_destination(
        destination, workspace=workspace, runner_temp=runner_temp
    )
    issue = reader.get_issue(reference)
    if issue.reference != reference:
        raise ValueError("The collected issue does not match the requested issue")

    with ExitStack() as descriptors:
        parent = _open_parent(target, directory_flags, descriptors)
        descriptor = os.open(target.relative.name, file_flags, 0o666, dir_fd=parent)
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as output:
            json.dump(
                issue.to_payload(), output, separators=(",", ":"), allow_nan=False
            )
            output.write("\n")
    return PreparedIssue(issue=issue, path=target.path)
