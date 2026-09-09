"""Prepare verified GitHub issue context for read-only automation jobs."""

import json
from dataclasses import dataclass
from pathlib import Path

from uv_automations.github import IssueReader
from uv_automations.models import Issue, IssueRef


@dataclass(frozen=True, slots=True)
class PreparedIssue:
    issue: Issue
    path: Path


def _resolve_destination(
    destination: Path, *, workspace: Path, runner_temp: Path
) -> Path:
    if any(character in str(destination) for character in "\r\n\0"):
        raise ValueError("The issue destination must be a single-line path")
    if destination.name in {"", ".", ".."}:
        raise ValueError("The issue destination must name a file")

    roots = (workspace.resolve(strict=True), runner_temp.resolve(strict=True))
    if not all(root.is_dir() for root in roots):
        raise ValueError("The issue destination roots must be directories")

    if not destination.is_absolute():
        destination = roots[0] / destination
    # Resolve the parent, not the leaf: exclusive creation must reject an existing
    # leaf symlink even when that symlink's target does not exist.
    resolved = destination.parent.resolve(strict=True) / destination.name
    if not any(resolved != root and resolved.is_relative_to(root) for root in roots):
        raise ValueError("The issue destination must be inside the runner or workspace")
    if any(character in str(resolved) for character in "\r\n\0"):
        raise ValueError("The issue destination must be a single-line path")
    return resolved


def prepare_issue(
    reader: IssueReader,
    reference: IssueRef,
    destination: Path,
    *,
    workspace: Path,
    runner_temp: Path,
) -> PreparedIssue:
    path = _resolve_destination(
        destination, workspace=workspace, runner_temp=runner_temp
    )
    issue = reader.get_issue(reference)
    if issue.reference != reference:
        raise ValueError("The collected issue does not match the requested issue")

    with path.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(issue.to_payload(), output, separators=(",", ":"), allow_nan=False)
        output.write("\n")
    return PreparedIssue(issue=issue, path=path)
