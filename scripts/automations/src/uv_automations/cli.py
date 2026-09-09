"""Command-line adapters for GitHub Actions."""

import argparse
import json
import logging
import re
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations import commits_cli, promotions_cli
from uv_automations.actions import append_summary, write_json_output, write_output
from uv_automations.github import GitHub
from uv_automations.github_promotion import PromotionReadError
from uv_automations.json import loads
from uv_automations.models import (
    CommitSha,
    PullRequestRef,
    RepositoryIdentity,
    RepositoryName,
)
from uv_automations.workflows.conflicts import (
    RebaseDispatch,
    conflict_payload,
    find_conflicted_pull_requests,
    identify_conflicts,
    parse_dispatch,
    remove_rebase_label,
)
from uv_automations.workflows.labels import (
    LabelApplyOutcome,
    LabelRecommendation,
    PreparedLabels,
    SkippedLabels,
    apply_labels,
    load_allowed_labels,
    plan_labels,
    prepare_labels,
    recommendation_summary,
    validate_label_plan,
)

logger = logging.getLogger(__name__)


class CommandGroup(StrEnum):
    LABELS = "labels"
    PULL_REQUESTS = "pull-requests"
    COMMITS = "commits"
    PROMOTIONS = "promotions"


class CommandKind(StrEnum):
    PREPARE_LABELS = "labels.prepare"
    VALIDATE_LABELS = "labels.validate"
    REPORT_LABELS = "labels.report"
    APPLY_LABELS = "labels.apply"
    FIND_CONFLICTS = "pull-requests.conflicts"
    IDENTIFY_CONFLICTS = "pull-requests.identify"
    REMOVE_REBASE_LABEL = "pull-requests.remove-rebase-label"


@dataclass(frozen=True, slots=True, kw_only=True)
class PrepareLabels:
    reference: PullRequestRef
    checkout: Path
    allowed: Path
    expected_head: CommitSha | None
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ValidateLabels:
    allowed: Path
    github_output: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class ReportLabels:
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ApplyLabels:
    reference: PullRequestRef
    allowed: Path
    expected_head: CommitSha
    summary: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class FindConflicts:
    repository: RepositoryName
    author: str | None


@dataclass(frozen=True, slots=True, kw_only=True)
class IdentifyConflicts:
    repository: RepositoryIdentity
    dispatch: RebaseDispatch | None
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class RemoveRebaseLabel:
    reference: PullRequestRef


type CoreCommand = (
    PrepareLabels
    | ValidateLabels
    | ReportLabels
    | ApplyLabels
    | FindConflicts
    | IdentifyConflicts
    | RemoveRebaseLabel
)

type Command = CoreCommand | commits_cli.CommitCommand | promotions_cli.PromotionCommand


def _positive_integer(value: str) -> int:
    if re.fullmatch(r"[1-9][0-9]*", value) is None:
        raise ValueError("Expected a positive integer")
    return int(value)


def _optional_number(value: str) -> int | None:
    return _positive_integer(value) if value else None


def _optional_commit(value: str) -> CommitSha | None:
    return CommitSha(value) if value else None


def _optional_string(value: str) -> str | None:
    return value or None


def _add_pull_request(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", dest="repository", type=RepositoryName, required=True)
    parser.add_argument("--pull-request", type=_positive_integer, required=True)


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="uv-automations", description=__doc__)
    commands = parser.add_subparsers(dest="command_group", required=True)
    labels = commands.add_parser("labels").add_subparsers(required=True)

    prepare = labels.add_parser("prepare")
    prepare.set_defaults(command=CommandKind.PREPARE_LABELS)
    _add_pull_request(prepare)
    prepare.add_argument("--checkout", type=Path, required=True)
    prepare.add_argument("--allowed", type=Path, required=True)
    prepare.add_argument("--expected-head", type=_optional_commit)
    prepare.add_argument("--github-output", type=Path, required=True)

    validate = labels.add_parser("validate")
    validate.set_defaults(command=CommandKind.VALIDATE_LABELS)
    validate.add_argument("--allowed", type=Path, required=True)
    validate.add_argument("--github-output", type=Path)

    report = labels.add_parser("report")
    report.set_defaults(command=CommandKind.REPORT_LABELS)
    report.add_argument("--summary", type=Path, required=True)

    apply = labels.add_parser("apply")
    apply.set_defaults(command=CommandKind.APPLY_LABELS)
    _add_pull_request(apply)
    apply.add_argument("--allowed", type=Path, required=True)
    apply.add_argument("--expected-head", type=CommitSha, required=True)
    apply.add_argument("--summary", type=Path)

    pull_requests = commands.add_parser("pull-requests").add_subparsers(required=True)
    conflicts = pull_requests.add_parser("conflicts")
    conflicts.set_defaults(command=CommandKind.FIND_CONFLICTS)
    conflicts.add_argument(
        "--repo", dest="repository", type=RepositoryName, required=True
    )
    conflicts.add_argument("--author")

    identify = pull_requests.add_parser("identify")
    identify.set_defaults(command=CommandKind.IDENTIFY_CONFLICTS)
    identify.add_argument(
        "--repo", dest="repository", type=RepositoryName, required=True
    )
    identify.add_argument("--repository-id", type=_positive_integer, required=True)
    identify.add_argument("--pull-request", type=_optional_number)
    identify.add_argument("--expected-head", type=_optional_commit)
    identify.add_argument("--previous-base-ref", type=_optional_string)
    identify.add_argument("--previous-base-sha", type=_optional_commit)
    identify.add_argument("--github-output", type=Path, required=True)
    identify.add_argument("--summary", type=Path, required=True)

    remove = pull_requests.add_parser("remove-rebase-label")
    remove.set_defaults(command=CommandKind.REMOVE_REBASE_LABEL)
    _add_pull_request(remove)
    commits_cli.add_commands(commands.add_parser("commits"))
    promotions_cli.add_commands(commands.add_parser("promotions"))
    return parser


def parse_command(
    parser: argparse.ArgumentParser, arguments: Sequence[str] | None
) -> Command:
    parsed = parser.parse_args(arguments)
    group = CommandGroup(parsed.command_group)
    match group:
        case CommandGroup.LABELS | CommandGroup.PULL_REQUESTS:
            return _parse_core_command(parsed)
        case CommandGroup.COMMITS:
            return commits_cli.parse_command(parsed)
        case CommandGroup.PROMOTIONS:
            return promotions_cli.parse_command(parsed)
    assert_never(group)


def _parse_core_command(parsed: argparse.Namespace) -> CoreCommand:
    kind = CommandKind(parsed.command)
    match kind:
        case CommandKind.PREPARE_LABELS:
            return PrepareLabels(
                reference=PullRequestRef(parsed.repository, parsed.pull_request),
                checkout=parsed.checkout,
                allowed=parsed.allowed,
                expected_head=parsed.expected_head,
                github_output=parsed.github_output,
            )
        case CommandKind.VALIDATE_LABELS:
            return ValidateLabels(
                allowed=parsed.allowed, github_output=parsed.github_output
            )
        case CommandKind.REPORT_LABELS:
            return ReportLabels(summary=parsed.summary)
        case CommandKind.APPLY_LABELS:
            return ApplyLabels(
                reference=PullRequestRef(parsed.repository, parsed.pull_request),
                allowed=parsed.allowed,
                expected_head=parsed.expected_head,
                summary=parsed.summary,
            )
        case CommandKind.FIND_CONFLICTS:
            return FindConflicts(repository=parsed.repository, author=parsed.author)
        case CommandKind.IDENTIFY_CONFLICTS:
            return IdentifyConflicts(
                repository=RepositoryIdentity(parsed.repository, parsed.repository_id),
                dispatch=parse_dispatch(
                    parsed.repository,
                    parsed.pull_request,
                    parsed.expected_head,
                    parsed.previous_base_ref,
                    parsed.previous_base_sha,
                ),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommandKind.REMOVE_REBASE_LABEL:
            return RemoveRebaseLabel(
                reference=PullRequestRef(parsed.repository, parsed.pull_request)
            )
    assert_never(kind)


def _label_apply_message(outcome: LabelApplyOutcome) -> str:
    match outcome:
        case LabelApplyOutcome.APPLIED:
            return "Applied the recommended pull request labels."
        case LabelApplyOutcome.STALE:
            return (
                "Skipped labels because the pull request is closed or its head changed."
            )
        case LabelApplyOutcome.EMPTY:
            return "No pull request labels were recommended."
    assert_never(outcome)


def run(command: Command) -> None:
    github = GitHub()
    match command:
        case PrepareLabels():
            preparation = prepare_labels(
                github,
                command.reference,
                command.checkout,
                load_allowed_labels(command.allowed),
                expected_head=command.expected_head,
            )
            match preparation:
                case PreparedLabels():
                    write_json_output(command.github_output, "ready", True)
                    write_output(
                        command.github_output, "head-sha", str(preparation.head_sha)
                    )
                    return
                case SkippedLabels():
                    logger.info("%s", preparation.reason)
                    write_json_output(command.github_output, "ready", False)
                    return
            assert_never(preparation)
        case ValidateLabels():
            recommendation = LabelRecommendation.from_json(loads(sys.stdin.read()))
            plan = plan_labels(recommendation, load_allowed_labels(command.allowed))
            if command.github_output is not None:
                write_json_output(command.github_output, "labels", plan.labels)
            else:
                print(json.dumps(plan.labels, separators=(",", ":")))
            return
        case ReportLabels():
            recommendation = LabelRecommendation.from_json(loads(sys.stdin.read()))
            append_summary(command.summary, recommendation_summary(recommendation))
            return
        case ApplyLabels():
            plan = validate_label_plan(
                loads(sys.stdin.read()), load_allowed_labels(command.allowed)
            )
            outcome = apply_labels(
                github, command.reference, plan, expected_head=command.expected_head
            )
            message = _label_apply_message(outcome)
            if command.summary is not None:
                append_summary(command.summary, message)
            else:
                print(message)
            return
        case FindConflicts():
            found = find_conflicted_pull_requests(
                github, command.repository, author=command.author
            )
            print(
                json.dumps(
                    [conflict_payload(pull_request) for pull_request in found],
                    separators=(",", ":"),
                )
            )
            return
        case IdentifyConflicts():
            plan = identify_conflicts(github, command.repository, command.dispatch)
            write_json_output(command.github_output, "matrix", plan.matrix())
            write_json_output(command.github_output, "rebasable", len(plan.rebasable))
            append_summary(command.summary, plan.summary())
            return
        case RemoveRebaseLabel():
            remove_rebase_label(github, command.reference)
            return
        case commits_cli.PersistCommit() | commits_cli.LoadCommit():
            commits_cli.run(command)
            return
        case (
            promotions_cli.PreparePromotion()
            | promotions_cli.ReadPromotionApproval()
            | promotions_cli.RecordPromotionQueue()
            | promotions_cli.ReplayPromotions()
            | promotions_cli.ReplayOnePromotion()
            | promotions_cli.ReplayPromotedChildren()
            | promotions_cli.SyncPromotionSource()
            | promotions_cli.EnsurePromotionBase()
        ):
            promotions_cli.run(command)
            return
    assert_never(command)


def main(arguments: Sequence[str] | None = None) -> None:
    parser = create_parser()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    try:
        run(parse_command(parser, arguments))
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(2, f"{parser.prog}: {error}\n")
    except subprocess.CalledProcessError as error:
        parser.exit(
            1, f"{parser.prog}: command failed with status {error.returncode}\n"
        )
    except subprocess.TimeoutExpired as error:
        parser.exit(
            1, f"{parser.prog}: command timed out after {error.timeout} seconds\n"
        )
    except PromotionReadError as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
