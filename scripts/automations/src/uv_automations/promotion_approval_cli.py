"""Read-only command-line adapters for private promotion approval."""

import argparse
import json
import re
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import append_summary, write_output
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.promotion_models import PromotionApprovalKind, PromotionScope
from uv_automations.workflows.promotion_approval import (
    PRIVATE_APPROVAL_DEPENDENCY,
    InspectedPrivatePromotion,
    PrivateApprovalReference,
    SkippedPrivatePromotionRequest,
    inspect_private_promotion,
    verify_private_approval,
)


class ApprovalCommandKind(StrEnum):
    INSPECT = "promotions.request.inspect"
    VERIFY = "promotions.private-approval"


@dataclass(frozen=True, slots=True, kw_only=True)
class InspectPrivatePromotion:
    source: PromotionScope
    head: CommitSha
    kind: PromotionApprovalKind
    github_output: Path | None
    summary: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class VerifyPrivateApproval:
    reference: PrivateApprovalReference
    github_output: Path | None
    summary: Path | None


type ApprovalCommand = InspectPrivatePromotion | VerifyPrivateApproval


def _positive_integer(value: str) -> int:
    if re.fullmatch(r"[1-9][0-9]*", value) is None:
        raise ValueError("Expected a positive integer")
    return int(value)


def _add_source(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", dest="repository", type=RepositoryName, required=True)
    parser.add_argument("--repository-id", type=_positive_integer, required=True)
    parser.add_argument("--pull-request", type=_positive_integer, required=True)
    parser.add_argument("--expected-head", type=CommitSha, required=True)


def _add_outputs(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--summary", type=Path)


def add_request_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    inspect = commands.add_parser("inspect")
    inspect.set_defaults(command=ApprovalCommandKind.INSPECT)
    _add_source(inspect)
    inspect.add_argument("--kind", type=PromotionApprovalKind, required=True)
    _add_outputs(inspect)


def add_private_approval_command(parser: argparse.ArgumentParser) -> None:
    parser.set_defaults(command=ApprovalCommandKind.VERIFY)
    _add_source(parser)
    parser.add_argument("--approval-kind", type=PromotionApprovalKind, required=True)
    parser.add_argument("--approval-id", type=_positive_integer, required=True)
    parser.add_argument("--ready-event-id", type=_positive_integer, required=True)
    parser.add_argument("--approval-receipt-id", type=_positive_integer, required=True)
    _add_outputs(parser)


def _scope(parsed: argparse.Namespace) -> PromotionScope:
    return PromotionScope(
        RepositoryIdentity(parsed.repository, parsed.repository_id), parsed.pull_request
    )


def _reference(parsed: argparse.Namespace) -> PrivateApprovalReference:
    if parsed.approval_kind != PromotionApprovalKind.LABELED:
        raise ValueError("Private promotion requires a label approval")
    return PrivateApprovalReference(
        _scope(parsed),
        parsed.expected_head,
        parsed.approval_id,
        parsed.ready_event_id,
        parsed.approval_receipt_id,
    )


def parse_command(parsed: argparse.Namespace) -> ApprovalCommand:
    kind = ApprovalCommandKind(parsed.command)
    match kind:
        case ApprovalCommandKind.INSPECT:
            return InspectPrivatePromotion(
                source=_scope(parsed),
                head=parsed.expected_head,
                kind=parsed.kind,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case ApprovalCommandKind.VERIFY:
            return VerifyPrivateApproval(
                reference=_reference(parsed),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
    assert_never(kind)


def _execute(command: ApprovalCommand) -> tuple[dict[str, str], str]:
    reader = PromotionGitHub(token_variable="GH_READ_TOKEN")
    match command:
        case InspectPrivatePromotion():
            result = inspect_private_promotion(
                reader, command.source, command.head, command.kind
            )
            match result:
                case InspectedPrivatePromotion():
                    return (
                        {
                            "state": "blocked",
                            "reason": "dispatcher_event_metadata_unavailable",
                            "current_kind": result.kind.value,
                            "current_event_id": str(result.event_id),
                            "current_ready_event_id": str(result.ready_event_id),
                        },
                        PRIVATE_APPROVAL_DEPENDENCY,
                    )
                case SkippedPrivatePromotionRequest(reason=reason):
                    return (
                        {"state": "unavailable", "reason": reason.value},
                        f"{reason.message} {PRIVATE_APPROVAL_DEPENDENCY}",
                    )
            assert_never(result)
        case VerifyPrivateApproval():
            approval = verify_private_approval(reader, command.reference)
            return (
                {
                    **command.reference.dispatch_inputs(),
                    "promoter": approval.actor.login,
                    "promoter_id": str(approval.actor.database_id),
                },
                (
                    "Verified the exact private promotion approval receipt. "
                    "No workflow was dispatched."
                ),
            )
    assert_never(command)


def run(command: ApprovalCommand) -> None:
    outputs, summary = _execute(command)
    if command.github_output is None:
        print(json.dumps(outputs, separators=(",", ":")))
    else:
        for name, value in outputs.items():
            write_output(command.github_output, name, value)
    if command.summary is not None:
        append_summary(command.summary, summary)
