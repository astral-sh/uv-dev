"""Command-line stages for promotion planning and durable replay."""

import argparse
import re
import sys
from collections import Counter
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations import promotion_retarget_cli
from uv_automations.actions import append_summary, write_json_output, write_output
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.github_promotion_queue import PromotionQueueGitHub
from uv_automations.json import loads
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    BranchRevision,
    MergedPromotedParent,
    PromotionScope,
    UnrecordedMergedParent,
    current_ready_approval,
    ready_approval,
)
from uv_automations.promotion_retarget_cli import ApplyRetargets, IdentifyRetargets
from uv_automations.workflows.promotion import (
    AlreadyPublished,
    CopyUpstreamBaseClaim,
    PromotionPlan,
    PromotionRequest,
    Publish,
    Rebase,
    Rejected,
    Stale,
    WaitForParent,
    WaitForSync,
    plan_promotion,
)
from uv_automations.workflows.promotion_publish import (
    BaseCopyOutcome,
    ensure_upstream_base,
)
from uv_automations.workflows.promotion_queue import (
    MAX_QUEUE_JSON_BYTES,
    DispatchedReplay,
    QueuedPromotion,
    QueueRecordOutcome,
    ReplayOutcome,
    SkippedReplay,
    record_queue,
    replay_one,
    replay_queued_promotions,
)
from uv_automations.workflows.promotion_replay import plan_queued_promotion

MAX_BASE_COPY_JSON_BYTES = 16 * 1024


class PromotionCommandKind(StrEnum):
    PREPARE = "promotions.prepare"
    CURRENT_APPROVAL = "promotions.current-approval"
    RECORD_QUEUE = "promotions.record-queue"
    REPLAY = "promotions.replay"
    REPLAY_ONE = "promotions.replay-one"
    REPLAY_CHILDREN = "promotions.replay-children"
    SYNC = "promotions.sync"
    ENSURE_BASE = "promotions.ensure-base"


@dataclass(frozen=True, slots=True, kw_only=True)
class PreparePromotion:
    request: PromotionRequest
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ReadPromotionApproval:
    request: PromotionRequest


@dataclass(frozen=True, slots=True, kw_only=True)
class RecordPromotionQueue:
    source: PromotionScope
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ReplayPromotions:
    main: BranchRevision
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ReplayOnePromotion:
    source: PromotionScope
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ReplayPromotedChildren:
    parent: PromotionScope
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class SyncPromotionSource:
    repository: RepositoryIdentity
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class EnsurePromotionBase:
    source: PromotionScope
    github_output: Path
    summary: Path


type PromotionCommand = (
    PreparePromotion
    | ReadPromotionApproval
    | RecordPromotionQueue
    | ReplayPromotions
    | ReplayOnePromotion
    | ReplayPromotedChildren
    | SyncPromotionSource
    | EnsurePromotionBase
    | promotion_retarget_cli.PromotionRetargetCommand
)


def _positive_integer(value: str) -> int:
    if re.fullmatch(r"[1-9][0-9]*", value) is None:
        raise ValueError("Expected a positive integer")
    return int(value)


def _optional_number(value: str) -> int | None:
    return _positive_integer(value) if value else None


def _add_repository(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", dest="repository", type=RepositoryName, required=True)
    parser.add_argument("--repository-id", type=_positive_integer, required=True)


def _add_source(parser: argparse.ArgumentParser) -> None:
    _add_repository(parser)
    parser.add_argument("--pull-request", type=_positive_integer, required=True)


def _scope(parsed: argparse.Namespace) -> PromotionScope:
    return PromotionScope(
        RepositoryIdentity(parsed.repository, parsed.repository_id), parsed.pull_request
    )


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    prepare = commands.add_parser("prepare")
    prepare.set_defaults(command=PromotionCommandKind.PREPARE)
    _add_source(prepare)
    prepare.add_argument("--expected-head", type=CommitSha, required=True)
    prepare.add_argument("--approval-id", type=_optional_number)
    prepare.add_argument("--github-output", type=Path, required=True)
    prepare.add_argument("--summary", type=Path, required=True)

    approval = commands.add_parser("current-approval")
    approval.set_defaults(command=PromotionCommandKind.CURRENT_APPROVAL)
    _add_source(approval)
    approval.add_argument("--expected-head", type=CommitSha, required=True)
    approval.add_argument("--approval-id", type=_optional_number)

    record = commands.add_parser("record-queue")
    record.set_defaults(command=PromotionCommandKind.RECORD_QUEUE)
    _add_source(record)
    record.add_argument("--github-output", type=Path, required=True)
    record.add_argument("--summary", type=Path, required=True)

    replay = commands.add_parser("replay")
    replay.set_defaults(command=PromotionCommandKind.REPLAY)
    _add_repository(replay)
    replay.add_argument("--main-sha", type=CommitSha, required=True)
    replay.add_argument("--summary", type=Path, required=True)

    replay_single = commands.add_parser("replay-one")
    replay_single.set_defaults(command=PromotionCommandKind.REPLAY_ONE)
    _add_source(replay_single)
    replay_single.add_argument("--summary", type=Path, required=True)

    children = commands.add_parser("replay-children")
    children.set_defaults(command=PromotionCommandKind.REPLAY_CHILDREN)
    _add_repository(children)
    children.add_argument("--parent", type=_positive_integer, required=True)
    children.add_argument("--summary", type=Path, required=True)

    sync = commands.add_parser("sync")
    sync.set_defaults(command=PromotionCommandKind.SYNC)
    _add_repository(sync)
    sync.add_argument("--github-output", type=Path, required=True)

    ensure = commands.add_parser("ensure-base")
    ensure.set_defaults(command=PromotionCommandKind.ENSURE_BASE)
    _add_source(ensure)
    ensure.add_argument("--github-output", type=Path, required=True)
    ensure.add_argument("--summary", type=Path, required=True)

    promotion_retarget_cli.add_commands(commands.add_parser("retarget"))


def parse_command(parsed: argparse.Namespace) -> PromotionCommand:
    if isinstance(parsed.command, promotion_retarget_cli.PromotionRetargetCommandKind):
        return promotion_retarget_cli.parse_command(parsed)
    kind = PromotionCommandKind(parsed.command)
    match kind:
        case PromotionCommandKind.PREPARE:
            return PreparePromotion(
                request=PromotionRequest(
                    _scope(parsed), parsed.expected_head, parsed.approval_id
                ),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case PromotionCommandKind.CURRENT_APPROVAL:
            return ReadPromotionApproval(
                request=PromotionRequest(
                    _scope(parsed), parsed.expected_head, parsed.approval_id
                ),
            )
        case PromotionCommandKind.RECORD_QUEUE:
            return RecordPromotionQueue(
                source=_scope(parsed),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case PromotionCommandKind.REPLAY:
            return ReplayPromotions(
                main=BranchRevision(
                    RepositoryIdentity(parsed.repository, parsed.repository_id),
                    "main",
                    parsed.main_sha,
                ),
                summary=parsed.summary,
            )
        case PromotionCommandKind.REPLAY_ONE:
            return ReplayOnePromotion(source=_scope(parsed), summary=parsed.summary)
        case PromotionCommandKind.REPLAY_CHILDREN:
            return ReplayPromotedChildren(
                parent=PromotionScope(
                    RepositoryIdentity(parsed.repository, parsed.repository_id),
                    parsed.parent,
                ),
                summary=parsed.summary,
            )
        case PromotionCommandKind.SYNC:
            return SyncPromotionSource(
                repository=RepositoryIdentity(parsed.repository, parsed.repository_id),
                github_output=parsed.github_output,
            )
        case PromotionCommandKind.ENSURE_BASE:
            return EnsurePromotionBase(
                source=_scope(parsed),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
    assert_never(kind)


def _read_json(limit: int) -> object:
    text = sys.stdin.read(limit + 1)
    if len(text.encode()) > limit:
        raise ValueError("Promotion input exceeds the size limit")
    return loads(text)


def _read_queue(source: PromotionScope) -> QueuedPromotion:
    queued = QueuedPromotion.from_json(_read_json(MAX_QUEUE_JSON_BYTES))
    if queued.source != source:
        raise ValueError("Promotion queue belongs to another source")
    return queued


def _write_plan(plan: PromotionPlan, output: Path, summary: Path) -> None:
    source = plan.source
    for name, value in {
        "source_base_ref": source.details.base.ref,
        "source_base_sha": str(source.details.base.sha),
        "head_ref": source.details.head.ref,
    }.items():
        write_output(output, name, value)
    if plan.approval is not None:
        write_output(output, "approval_id", str(plan.approval.event_id))
        write_output(output, "promoter", plan.approval.actor.login)

    match plan:
        case Publish() | AlreadyPublished():
            write_output(output, "action", "promote")
            write_output(output, "base_ref", plan.base.ref)
            write_output(output, "base_sha", str(plan.base.sha))
            if isinstance(plan, Publish) and plan.copy_base is not None:
                write_json_output(output, "copy_base", plan.copy_base.to_json())
            append_summary(
                summary, "The approved pull request is ready for publication."
            )
            return
        case Rebase():
            write_output(output, "action", "update-parent")
            write_output(output, "base_ref", plan.base.ref)
            write_output(output, "base_sha", str(plan.base.sha))
            write_output(output, "base_previous_sha", str(plan.previous_base))
            write_output(output, "parent_merge_sha", str(plan.parent.merge.sha))
            append_summary(
                summary,
                "The approved child needs a clean update onto its merged parent.",
            )
            return
        case WaitForParent():
            write_output(output, "action", "queued")
            if source.scope.repository == UV_DEV_REPOSITORY:
                queued = QueuedPromotion.waiting_for_parent(
                    source, plan.approval, plan.parent
                )
                write_json_output(output, "queue", queued.to_json())
                append_summary(summary, queued.comment().split("\n", 1)[0])
            else:
                append_summary(
                    summary, "The private promotion is waiting for its source parent."
                )
            return
        case WaitForSync():
            write_output(output, "action", "queued")
            match plan.parent:
                case MergedPromotedParent():
                    if source.scope.repository == UV_DEV_REPOSITORY:
                        queued = QueuedPromotion.waiting_for_sync(
                            source, plan.approval, plan.parent
                        )
                        write_json_output(output, "queue", queued.to_json())
                        append_summary(summary, queued.comment().split("\n", 1)[0])
                    else:
                        append_summary(
                            summary,
                            "The private promotion is waiting for its parent merge to synchronize.",
                        )
                    return
                case UnrecordedMergedParent():
                    append_summary(
                        summary,
                        "The promotion is waiting for its parent merge to synchronize. Automatic replay requires a verified bot promotion record.",
                    )
                    return
            assert_never(plan.parent)
        case Stale():
            write_output(output, "action", "stale")
            if (
                plan.approval is not None
                and source.details.head.sha != plan.expected_head
            ):
                write_output(output, "changed_head", str(source.details.head.sha))
            append_summary(summary, f"Promotion skipped: {plan.reason}.")
            return
        case Rejected():
            write_output(output, "action", "rejected")
            write_output(output, "rejection_reason", plan.reason)
            append_summary(summary, f"Promotion stopped: {plan.reason}.")
            return
    assert_never(plan)


def _replay_summary(outcomes: tuple[ReplayOutcome, ...]) -> str:
    dispatched: list[str] = []
    skipped: Counter[str] = Counter()
    for outcome in outcomes:
        match outcome:
            case DispatchedReplay():
                dispatched.append(
                    f"- [{outcome.queued.source.repository.name}#{outcome.queued.source.number}]({outcome.run.url})"
                )
            case SkippedReplay():
                skipped[outcome.reason.value] += 1
            case _:
                assert_never(outcome)
    text = f"Replayed {len(dispatched)} queued promotion(s)."
    if dispatched:
        text += "\n\n" + "\n".join(dispatched)
    if skipped:
        text += (
            "\n\nPending or skipped: "
            + ", ".join(
                f"{reason}={count}" for reason, count in sorted(skipped.items())
            )
            + "."
        )
    return text


def run(command: PromotionCommand) -> None:
    match command:
        case PreparePromotion():
            reader = PromotionGitHub()
            plan = (
                plan_queued_promotion(reader, command.request)
                if command.request.source.repository == UV_DEV_REPOSITORY
                and command.request.approval_id is not None
                else plan_promotion(reader, command.request)
            )
            _write_plan(
                plan,
                command.github_output,
                command.summary,
            )
            return
        case ReadPromotionApproval(request=request):
            events = PromotionGitHub().list_promotion_events(request.source)
            approval = (
                current_ready_approval(request.source, request.head, events)
                if request.source.repository == UV_DEV_REPOSITORY
                and request.approval_id is not None
                else ready_approval(request.source, request.head, events)
            )
            if approval is not None and (
                request.approval_id is None or request.approval_id == approval.event_id
            ):
                print(approval.event_id)
            return
        case RecordPromotionQueue():
            queued = _read_queue(command.source)
            outcome = record_queue(
                PromotionGitHub(token_variable="GH_READ_TOKEN"),
                PromotionQueueGitHub(),
                queued,
            )
            match outcome:
                case QueueRecordOutcome.RECORDED | QueueRecordOutcome.UNCHANGED:
                    write_json_output(command.github_output, "recorded", True)
                case QueueRecordOutcome.STALE:
                    write_json_output(command.github_output, "recorded", False)
            append_summary(command.summary, f"Promotion queue: {outcome.value}.")
            return
        case ReplayPromotions():
            outcomes = replay_queued_promotions(
                PromotionGitHub(token_variable="GH_SOURCE_TOKEN"),
                PromotionGitHub(token_variable="GH_UPSTREAM_TOKEN"),
                PromotionQueueGitHub(),
                command.main,
            )
            append_summary(command.summary, _replay_summary(outcomes))
            return
        case ReplayOnePromotion():
            queued = _read_queue(command.source)
            reader = PromotionGitHub(token_variable="GH_SOURCE_TOKEN")
            main = reader.get_ref(command.source.repository, "main")
            if main is None:
                raise ValueError("The promotion source main branch is missing")
            outcome = replay_one(
                reader,
                PromotionGitHub(token_variable="GH_UPSTREAM_TOKEN"),
                PromotionQueueGitHub(),
                command.source,
                BranchRevision(command.source.repository, "main", main),
                expected=queued,
            )
            append_summary(command.summary, _replay_summary((outcome,)))
            return
        case ReplayPromotedChildren():
            reader = PromotionGitHub(token_variable="GH_SOURCE_TOKEN")
            main = reader.get_ref(command.parent.repository, "main")
            if main is None:
                raise ValueError("The promotion source main branch is missing")
            outcomes = replay_queued_promotions(
                reader,
                PromotionGitHub(token_variable="GH_UPSTREAM_TOKEN"),
                PromotionQueueGitHub(),
                BranchRevision(command.parent.repository, "main", main),
                parent=command.parent,
            )
            append_summary(command.summary, _replay_summary(outcomes))
            return
        case SyncPromotionSource():
            if command.repository != UV_DEV_REPOSITORY:
                raise ValueError("This synchronization stage is only for uv-dev")
            main = PromotionQueueGitHub().sync_uv_dev_main(
                PromotionGitHub(token_variable="GH_UPSTREAM_TOKEN")
            )
            write_output(command.github_output, "main-sha", str(main))
            return
        case EnsurePromotionBase():
            claim = CopyUpstreamBaseClaim.from_json(
                _read_json(MAX_BASE_COPY_JSON_BYTES)
            )
            if claim.approval.source != command.source:
                raise ValueError("Promotion base belongs to another source")
            reader = PromotionGitHub(token_variable="GH_READ_TOKEN")
            outcome = ensure_upstream_base(
                reader, reader, PromotionQueueGitHub(), claim
            )
            match outcome:
                case BaseCopyOutcome.CREATED | BaseCopyOutcome.UNCHANGED:
                    write_json_output(command.github_output, "ready", True)
                case BaseCopyOutcome.STALE:
                    write_json_output(command.github_output, "ready", False)
                    write_output(
                        command.github_output,
                        "rejection_reason",
                        "The promotion base or approval changed before publication.",
                    )
            append_summary(command.summary, f"Promotion base: {outcome.value}.")
            return
        case IdentifyRetargets() | ApplyRetargets():
            promotion_retarget_cli.run(command)
            return
    assert_never(command)
