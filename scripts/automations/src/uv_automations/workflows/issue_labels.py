"""Prepare and publish additive labels for verified GitHub issues."""

import hashlib
import json
import re
from collections.abc import Collection
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Protocol

from uv_automations.json import as_object, as_string
from uv_automations.models import (
    Issue,
    IssueDetails,
    IssueRef,
    IssueState,
    Label,
    ManagedRepository,
    RepositoryName,
)
from uv_automations.workflows.labels import (
    LabelApplyOutcome,
    LabelPlan,
    LabelRecommendation,
    SkippedLabels,
    plan_labels,
)


class IssueType(StrEnum):
    BUG = "bug"
    ENHANCEMENT = "enhancement"
    DUPLICATE = "duplicate"
    QUESTION = "question"


@dataclass(frozen=True, slots=True)
class IssueRevision:
    value: str

    def __post_init__(self) -> None:
        if re.fullmatch(r"[0-9a-f]{64}", self.value) is None:
            raise ValueError("Expected a SHA-256 issue revision")

    @classmethod
    def for_issue(cls, issue: Issue) -> IssueRevision:
        encoded = json.dumps(
            issue.to_payload(), sort_keys=True, separators=(",", ":"), allow_nan=False
        ).encode()
        return cls(hashlib.sha256(encoded).hexdigest())


@dataclass(frozen=True, slots=True)
class PreparedIssueLabels:
    revision: IssueRevision
    existing: tuple[str, ...]
    issue_type: IssueType | None


class IssueLabelReader(Protocol):
    def get_issue_details(self, reference: IssueRef) -> IssueDetails: ...
    def list_labels(self, repository: RepositoryName) -> tuple[Label, ...]: ...


class IssueLabelPublisher(IssueLabelReader, Protocol):
    def add_labels(self, reference: IssueRef, labels: tuple[str, ...]) -> None: ...


def triage_type(value: object | None) -> IssueType | None:
    if value is None:
        return None
    return IssueType(as_string(as_object(value)["type"]))


def _require_uv(reference: IssueRef) -> None:
    if reference.repository.full_name != ManagedRepository.UV.value:
        raise ValueError("Automatic issue labels are restricted to uv")


def _write_context(path: Path, value: object) -> None:
    # These reserved paths must not overwrite files or follow repository symlinks.
    with path.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, separators=(",", ":"), allow_nan=False)
        output.write("\n")


def prepare_issue_labels(
    github: IssueLabelReader,
    reference: IssueRef,
    checkout: Path,
    allowed: Collection[str],
    *,
    triage: object | None,
    triaged_issue: Issue | None,
) -> PreparedIssueLabels | SkippedLabels:
    _require_uv(reference)
    if (triage is None) != (triaged_issue is None):
        raise ValueError(
            "The triage result and its issue snapshot must be provided together"
        )
    issue_type = triage_type(triage)
    details = github.get_issue_details(reference)
    if details.issue.reference != reference:
        raise ValueError("The collected issue does not match the requested issue")
    if details.state is IssueState.CLOSED:
        return SkippedLabels("The issue is closed")
    if triaged_issue is not None and details.issue != triaged_issue:
        return SkippedLabels("The issue changed after triage")
    labels = [
        {"name": label.name, "description": label.description}
        for label in github.list_labels(reference.repository)
        if label.name in allowed
    ]
    _write_context(
        checkout / ".issue-labels-event.json",
        {**details.issue.to_payload(), "labels": details.labels},
    )
    _write_context(checkout / ".issue-labels.json", labels)
    _write_context(checkout / ".issue-labels-triage.json", triage)
    return PreparedIssueLabels(
        IssueRevision.for_issue(details.issue), details.labels, issue_type
    )


def _validate_issue_labels(
    recommendation: LabelRecommendation,
    allowed: Collection[str],
    *,
    issue_type: IssueType | None,
) -> LabelPlan:
    labels = plan_labels(recommendation, allowed).labels
    primary = set(labels).intersection(IssueType)
    if len(primary) > 1 or (
        issue_type is not None and primary.difference({issue_type})
    ):
        raise ValueError("Recommended labels conflict with the issue classification")
    return LabelPlan(labels)


def plan_issue_labels(
    recommendation: LabelRecommendation,
    allowed: Collection[str],
    *,
    existing: Collection[str],
    issue_type: IssueType | None,
) -> LabelPlan:
    labels = _validate_issue_labels(
        recommendation, allowed, issue_type=issue_type
    ).labels
    existing_primary = set(existing).intersection(IssueType)
    missing = tuple(
        label
        for label in labels
        if label not in existing and not (existing_primary and label in IssueType)
    )
    # The triage stage owns the primary classification; labeling fills it in only
    # when a maintainer has not already classified the issue.
    if issue_type is not None and not existing_primary and issue_type not in missing:
        missing = (issue_type.value, *missing)
    return plan_labels(LabelRecommendation(missing, recommendation.summary), allowed)


def apply_issue_labels(
    github: IssueLabelPublisher,
    reference: IssueRef,
    plan: LabelPlan,
    allowed: Collection[str],
    *,
    expected_revision: IssueRevision,
    issue_type: IssueType | None,
) -> LabelApplyOutcome:
    _require_uv(reference)
    # Validate again before making any API calls in the write-capable job.
    plan = _validate_issue_labels(
        LabelRecommendation(plan.labels, ""),
        allowed,
        issue_type=issue_type,
    )
    details = github.get_issue_details(reference)
    if details.issue.reference != reference:
        raise ValueError("The collected issue does not match the requested issue")
    if (
        details.state is IssueState.CLOSED
        or IssueRevision.for_issue(details.issue) != expected_revision
    ):
        return LabelApplyOutcome.STALE
    plan = plan_issue_labels(
        LabelRecommendation(plan.labels, ""),
        allowed,
        existing=details.labels,
        issue_type=issue_type,
    )
    if not plan.labels:
        return LabelApplyOutcome.EMPTY
    available = {label.name for label in github.list_labels(reference.repository)}
    if missing := set(plan.labels).difference(available):
        raise ValueError(f"Repository labels no longer exist: {sorted(missing)!r}")
    github.add_labels(reference, plan.labels)
    return LabelApplyOutcome.APPLIED
