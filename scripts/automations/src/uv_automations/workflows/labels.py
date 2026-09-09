"""Prepare, validate, and publish pull-request label recommendations."""

import json
from collections.abc import Collection
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Protocol

from uv_automations import git
from uv_automations.github import PullRequestLabelContext
from uv_automations.json import as_array, as_object, as_string, loads
from uv_automations.models import (
    CommitSha,
    Label,
    ManagedRepository,
    PullRequestDetails,
    PullRequestRef,
    RepositoryName,
)

MAX_LABELS = 3


@dataclass(frozen=True, slots=True)
class LabelRecommendation:
    labels: tuple[str, ...]
    summary: str

    def __post_init__(self) -> None:
        if len(self.labels) > MAX_LABELS:
            raise ValueError(f"Expected at most {MAX_LABELS} labels")

    @classmethod
    def from_json(cls, value: object) -> LabelRecommendation:
        data = as_object(value)
        if data.keys() != {"labels", "summary"}:
            raise ValueError("Expected exactly 'labels' and 'summary'")
        labels = tuple(as_string(label) for label in as_array(data["labels"]))
        return cls(labels, as_string(data["summary"]))


@dataclass(frozen=True, slots=True)
class LabelPlan:
    labels: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class PreparedLabels:
    head_sha: CommitSha


@dataclass(frozen=True, slots=True)
class SkippedLabels:
    reason: str


type LabelPreparation = PreparedLabels | SkippedLabels


class LabelApplyOutcome(StrEnum):
    APPLIED = "applied"
    STALE = "stale"
    EMPTY = "empty"


class LabelContextReader(Protocol):
    def get_label_context(
        self, reference: PullRequestRef
    ) -> PullRequestLabelContext: ...
    def list_labels(self, repository: RepositoryName) -> tuple[Label, ...]: ...


class LabelPublisher(Protocol):
    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails: ...
    def add_labels(
        self, reference: PullRequestRef, labels: tuple[str, ...]
    ) -> None: ...


def load_allowed_labels(path: Path) -> frozenset[str]:
    return frozenset(
        as_string(label) for label in as_array(loads(path.read_text(encoding="utf-8")))
    )


def plan_labels(
    recommendation: LabelRecommendation, allowed: Collection[str]
) -> LabelPlan:
    if len(set(recommendation.labels)) != len(recommendation.labels):
        raise ValueError("Recommended labels must be unique")
    if disallowed := set(recommendation.labels).difference(allowed):
        raise ValueError(f"Disallowed pull request labels: {sorted(disallowed)!r}")
    return LabelPlan(recommendation.labels)


def validate_label_plan(value: object, allowed: Collection[str]) -> LabelPlan:
    labels = tuple(as_string(label) for label in as_array(value))
    return plan_labels(LabelRecommendation(labels, summary=""), allowed)


def _replace_context_file(path: Path, content: str) -> None:
    # The pull request can contain a symlink at either reserved path.
    path.unlink(missing_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as output:
        output.write(content)
        output.write("\n")


def prepare_labels(
    github: LabelContextReader,
    reference: PullRequestRef,
    checkout: Path,
    allowed: Collection[str],
    *,
    expected_head: CommitSha | None,
) -> LabelPreparation:
    checked_out_head = git.head(checkout)
    if expected_head is not None and checked_out_head != expected_head:
        return SkippedLabels("The checkout does not match the dispatched head")
    context = github.get_label_context(reference)
    if context.head_sha != checked_out_head:
        return SkippedLabels("The pull request head changed before labeling")
    labels = [
        {"name": label.name, "description": label.description}
        for label in github.list_labels(reference.repository)
        if label.name in allowed
    ]
    _replace_context_file(
        checkout / ".pull-request-labels-event.json", context.event_json
    )
    _replace_context_file(
        checkout / ".pull-request-labels.json", json.dumps(labels, allow_nan=False)
    )
    return PreparedLabels(checked_out_head)


def apply_labels(
    github: LabelPublisher,
    reference: PullRequestRef,
    plan: LabelPlan,
    *,
    expected_head: CommitSha,
) -> LabelApplyOutcome:
    if reference.repository.full_name != ManagedRepository.UV_DEV.value:
        raise ValueError("Automatic pull request labels are restricted to uv-dev")
    if not plan.labels:
        return LabelApplyOutcome.EMPTY
    pull_request = github.get_pull_request(reference)
    if not pull_request.is_open or pull_request.head.sha != expected_head:
        return LabelApplyOutcome.STALE
    github.add_labels(reference, plan.labels)
    return LabelApplyOutcome.APPLIED


def recommendation_summary(recommendation: LabelRecommendation) -> str:
    encoded = json.dumps(
        {"labels": recommendation.labels, "summary": recommendation.summary}, indent=2
    )
    return f"### Pull request labels\n\n```json\n{encoded}\n```\n"
