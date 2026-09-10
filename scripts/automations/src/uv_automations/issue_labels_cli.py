"""Command-line adapters for issue-label recommendations and publication."""

import argparse
import json
import logging
import sys
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import append_summary, write_json_output, write_output
from uv_automations.github import GitHub, decode_issue
from uv_automations.json import as_array, as_string, loads
from uv_automations.models import Issue, IssueRef, RepositoryName
from uv_automations.workflows.issue_labels import (
    IssueRevision,
    IssueType,
    PreparedIssueLabels,
    apply_issue_labels,
    plan_issue_labels,
    prepare_issue_labels,
)
from uv_automations.workflows.labels import (
    LabelApplyOutcome,
    LabelRecommendation,
    SkippedLabels,
    load_allowed_labels,
    recommendation_summary,
    validate_label_plan,
)

logger = logging.getLogger(__name__)


class IssueLabelCommandKind(StrEnum):
    PREPARE = "issue-labels.prepare"
    VALIDATE = "issue-labels.validate"
    REPORT = "issue-labels.report"
    APPLY = "issue-labels.apply"


@dataclass(frozen=True, slots=True, kw_only=True)
class PrepareIssueLabels:
    reference: IssueRef
    checkout: Path
    allowed: Path
    triage: object | None
    triaged_issue: Issue | None
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ValidateIssueLabels:
    allowed: Path
    existing: tuple[str, ...]
    issue_type: IssueType | None
    github_output: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class ReportIssueLabels:
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ApplyIssueLabels:
    reference: IssueRef
    allowed: Path
    expected_revision: IssueRevision
    issue_type: IssueType | None
    summary: Path | None


type IssueLabelCommand = (
    PrepareIssueLabels | ValidateIssueLabels | ReportIssueLabels | ApplyIssueLabels
)


def _add_issue(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", dest="repository", type=RepositoryName, required=True)
    parser.add_argument("--issue", required=True)


def _optional_type(value: str) -> IssueType | None:
    return IssueType(value) if value else None


def _label_names(value: str) -> tuple[str, ...]:
    return tuple(as_string(label) for label in as_array(loads(value)))


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    prepare = commands.add_parser("prepare")
    prepare.set_defaults(command=IssueLabelCommandKind.PREPARE)
    _add_issue(prepare)
    prepare.add_argument("--checkout", type=Path, required=True)
    prepare.add_argument("--allowed", type=Path, required=True)
    prepare.add_argument("--triage-result", default="")
    prepare.add_argument("--triage-issue", default="")
    prepare.add_argument("--github-output", type=Path, required=True)

    validate = commands.add_parser("validate")
    validate.set_defaults(command=IssueLabelCommandKind.VALIDATE)
    validate.add_argument("--allowed", type=Path, required=True)
    validate.add_argument("--existing", type=_label_names, required=True)
    validate.add_argument("--issue-type", type=_optional_type)
    validate.add_argument("--github-output", type=Path)

    report = commands.add_parser("report")
    report.set_defaults(command=IssueLabelCommandKind.REPORT)
    report.add_argument("--summary", type=Path, required=True)

    apply = commands.add_parser("apply")
    apply.set_defaults(command=IssueLabelCommandKind.APPLY)
    _add_issue(apply)
    apply.add_argument("--allowed", type=Path, required=True)
    apply.add_argument("--expected-revision", type=IssueRevision, required=True)
    apply.add_argument("--issue-type", type=_optional_type)
    apply.add_argument("--summary", type=Path)


def parse_command(parsed: argparse.Namespace) -> IssueLabelCommand:
    kind = IssueLabelCommandKind(parsed.command)
    match kind:
        case IssueLabelCommandKind.PREPARE:
            reference = IssueRef.from_input(parsed.repository, parsed.issue)
            return PrepareIssueLabels(
                reference=reference,
                checkout=parsed.checkout,
                allowed=parsed.allowed,
                triage=loads(parsed.triage_result) if parsed.triage_result else None,
                triaged_issue=(
                    decode_issue(loads(parsed.triage_issue), reference)
                    if parsed.triage_issue
                    else None
                ),
                github_output=parsed.github_output,
            )
        case IssueLabelCommandKind.VALIDATE:
            return ValidateIssueLabels(
                allowed=parsed.allowed,
                existing=parsed.existing,
                issue_type=parsed.issue_type,
                github_output=parsed.github_output,
            )
        case IssueLabelCommandKind.REPORT:
            return ReportIssueLabels(summary=parsed.summary)
        case IssueLabelCommandKind.APPLY:
            return ApplyIssueLabels(
                reference=IssueRef.from_input(parsed.repository, parsed.issue),
                allowed=parsed.allowed,
                expected_revision=parsed.expected_revision,
                issue_type=parsed.issue_type,
                summary=parsed.summary,
            )
    assert_never(kind)


def _apply_message(outcome: LabelApplyOutcome) -> str:
    match outcome:
        case LabelApplyOutcome.APPLIED:
            return "Applied the recommended issue labels."
        case LabelApplyOutcome.STALE:
            return "Skipped labels because the issue is closed or its contents changed."
        case LabelApplyOutcome.EMPTY:
            return "No additional issue labels are needed."
    assert_never(outcome)


def run(command: IssueLabelCommand) -> None:
    match command:
        case PrepareIssueLabels():
            preparation = prepare_issue_labels(
                GitHub(),
                command.reference,
                command.checkout,
                load_allowed_labels(command.allowed),
                triage=command.triage,
                triaged_issue=command.triaged_issue,
            )
            match preparation:
                case PreparedIssueLabels():
                    write_json_output(command.github_output, "ready", True)
                    write_json_output(
                        command.github_output, "issue-number", command.reference.number
                    )
                    write_output(
                        command.github_output, "revision", preparation.revision.value
                    )
                    write_json_output(
                        command.github_output, "existing-labels", preparation.existing
                    )
                    write_output(
                        command.github_output,
                        "issue-type",
                        preparation.issue_type or "",
                    )
                    return
                case SkippedLabels():
                    logger.info("%s", preparation.reason)
                    write_json_output(command.github_output, "ready", False)
                    return
            assert_never(preparation)
        case ValidateIssueLabels():
            recommendation = LabelRecommendation.from_json(loads(sys.stdin.read()))
            plan = plan_issue_labels(
                recommendation,
                load_allowed_labels(command.allowed),
                existing=command.existing,
                issue_type=command.issue_type,
            )
            if command.github_output is not None:
                write_json_output(command.github_output, "labels", plan.labels)
            else:
                print(json.dumps(plan.labels, separators=(",", ":")))
            return
        case ReportIssueLabels():
            recommendation = LabelRecommendation.from_json(loads(sys.stdin.read()))
            append_summary(
                command.summary,
                recommendation_summary(recommendation, title="Issue labels"),
            )
            return
        case ApplyIssueLabels():
            allowed = load_allowed_labels(command.allowed)
            plan = validate_label_plan(loads(sys.stdin.read()), allowed)
            outcome = apply_issue_labels(
                GitHub(),
                command.reference,
                plan,
                allowed,
                expected_revision=command.expected_revision,
                issue_type=command.issue_type,
            )
            message = _apply_message(outcome)
            if command.summary is not None:
                append_summary(command.summary, message)
            else:
                print(message)
            return
    assert_never(command)
