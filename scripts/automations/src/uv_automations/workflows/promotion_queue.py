"""Durable, exact-revision queueing for public pull-request promotions."""

import json
from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, assert_never

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.github_promotion import (
    PromotedParentReader,
    PromotionComparisonReader,
    PromotionRevisionReader,
    verified_promoted_parent,
)
from uv_automations.json import as_object, as_string, loads, require_keys
from uv_automations.models import CommitSha, RepositoryIdentity
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    ClosedPromotedParent,
    MergedPromotedParent,
    OpenPromotedParent,
    PromotionApproval,
    PromotionApprovalClaim,
    PromotionApprovalKind,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    PullRequestSelection,
    UneditedPromotionComment,
    current_ready_approval,
)

QUEUE_MARKER = "<!-- uv-automations:promotion-queue:v1 "
MAX_QUEUE_JSON_BYTES = 4096


class QueueParentKind(StrEnum):
    SOURCE = "source"
    SYNC = "sync"


@dataclass(frozen=True, slots=True)
class PendingSourceParent:
    source: PromotionScope

    def to_json(self) -> dict[str, object]:
        return {"kind": QueueParentKind.SOURCE.value, "source": self.source.to_json()}


@dataclass(frozen=True, slots=True)
class PendingParentSync:
    source: PromotionScope
    upstream: PromotionScope
    merge: CommitSha

    def __post_init__(self) -> None:
        if self.upstream.repository != UV_REPOSITORY:
            raise ValueError("Queued parent has an unexpected upstream repository")

    def to_json(self) -> dict[str, object]:
        return {
            "kind": QueueParentKind.SYNC.value,
            "source": self.source.to_json(),
            "upstream": self.upstream.to_json(),
            "merge": str(self.merge),
        }


type QueuedParent = PendingSourceParent | PendingParentSync


def _parent_from_json(value: object) -> QueuedParent:
    data = as_object(value)
    kind = QueueParentKind(as_string(data["kind"]))
    match kind:
        case QueueParentKind.SOURCE:
            require_keys(data, {"kind", "source"})
            return PendingSourceParent(PromotionScope.from_json(data["source"]))
        case QueueParentKind.SYNC:
            require_keys(data, {"kind", "source", "upstream", "merge"})
            return PendingParentSync(
                PromotionScope.from_json(data["source"]),
                PromotionScope.from_json(data["upstream"]),
                CommitSha(as_string(data["merge"])),
            )
    assert_never(kind)


@dataclass(frozen=True, slots=True)
class QueuedPromotion:
    approval: PromotionApprovalClaim
    base: BranchRevision
    head: BranchRevision
    parent: QueuedParent

    def __post_init__(self) -> None:
        source = self.approval.source
        if (
            source.repository != UV_DEV_REPOSITORY
            or self.approval.kind != PromotionApprovalKind.READY_FOR_REVIEW
            or self.approval.ready_event_id != self.approval.event_id
            or self.base.repository != source.repository
            or self.head.repository != source.repository
            or self.head.sha != self.approval.head
            or self.parent.source.repository != source.repository
            or self.parent.source == source
        ):
            raise ValueError("Inconsistent public promotion queue identity")

    @property
    def source(self) -> PromotionScope:
        return self.approval.source

    def to_json(self) -> dict[str, object]:
        return {
            "approval": self.approval.to_json(),
            "base": self.base.to_json(),
            "head": self.head.to_json(),
            "parent": self.parent.to_json(),
        }

    @classmethod
    def from_json(cls, value: object) -> QueuedPromotion:
        data = as_object(value)
        require_keys(data, {"approval", "base", "head", "parent"})
        return cls(
            PromotionApprovalClaim.from_json(data["approval"]),
            BranchRevision.from_json(data["base"]),
            BranchRevision.from_json(data["head"]),
            _parent_from_json(data["parent"]),
        )

    @classmethod
    def waiting_for_parent(
        cls,
        source: PromotionPullRequest,
        approval: PromotionApproval,
        parent: PromotionPullRequest,
    ) -> QueuedPromotion:
        return cls(
            approval.claim,
            BranchRevision(
                source.scope.repository,
                source.details.base.ref,
                source.details.base.sha,
            ),
            BranchRevision(
                source.scope.repository,
                source.details.head.ref,
                source.details.head.sha,
            ),
            PendingSourceParent(parent.scope),
        )

    @classmethod
    def waiting_for_sync(
        cls,
        source: PromotionPullRequest,
        approval: PromotionApproval,
        parent: MergedPromotedParent,
    ) -> QueuedPromotion:
        return cls(
            approval.claim,
            BranchRevision(
                source.scope.repository,
                source.details.base.ref,
                source.details.base.sha,
            ),
            BranchRevision(
                source.scope.repository,
                source.details.head.ref,
                source.details.head.sha,
            ),
            PendingParentSync(
                parent.source.scope, parent.upstream.scope, parent.merge.sha
            ),
        )

    def comment(self) -> str:
        match self.parent:
            case PendingSourceParent(source=parent):
                reason = (
                    f"Waiting for promotion of [{parent.repository.name}#{parent.number}]"
                    f"(https://github.com/{parent.repository.name}/pull/{parent.number})."
                )
            case PendingParentSync(upstream=parent):
                reason = (
                    f"Waiting for [{parent.repository.name}#{parent.number}]"
                    f"(https://github.com/{parent.repository.name}/pull/{parent.number}) "
                    "to reach `uv-dev/main`."
                )
            case _:
                assert_never(self.parent)
        encoded = json.dumps(
            self.to_json(), separators=(",", ":"), sort_keys=True, allow_nan=False
        )
        if len(encoded.encode()) > MAX_QUEUE_JSON_BYTES:
            raise ValueError("Promotion queue record exceeds the size limit")
        return f"{reason}\n\n{QUEUE_MARKER}{encoded} -->"


def queued_promotion(comment: PromotionComment) -> QueuedPromotion | None:
    """Recognize only the complete canonical record, never quoted marker text."""
    if not comment.is_automation or comment.body.count(QUEUE_MARKER) != 1:
        return None
    _, _, marker = comment.body.partition(QUEUE_MARKER)
    if not marker.endswith(" -->"):
        return None
    encoded = marker.removesuffix(" -->")
    if len(encoded.encode()) > MAX_QUEUE_JSON_BYTES:
        return None
    try:
        queue = QueuedPromotion.from_json(loads(encoded))
    except KeyError, TypeError, ValueError:
        return None
    if queue.source != comment.scope or queue.comment() != comment.body:
        return None
    return queue


class QueueRecordReader(Protocol):
    def get_promotion_pull_request(
        self, scope: PromotionScope
    ) -> PromotionPullRequest: ...

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]: ...

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]: ...

    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None: ...


class QueueReader(
    QueueRecordReader, PromotedParentReader, PromotionRevisionReader, Protocol
):
    pass


class ReplayUpstreamReader(PromotedParentReader, PromotionRevisionReader, Protocol):
    pass


class QueueDiscoveryReader(QueueReader, Protocol):
    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]: ...


class QueueWriter(Protocol):
    def create_queue_comment(self, queued: QueuedPromotion) -> None: ...


class PromotionDispatcher(Protocol):
    def dispatch_promotion(
        self, approval: PromotionApprovalClaim
    ) -> WorkflowDispatch: ...


class QueueRecordOutcome(StrEnum):
    RECORDED = "recorded"
    UNCHANGED = "unchanged"
    STALE = "stale"


class ReplaySkipReason(StrEnum):
    NO_RECORD = "no-record"
    AMBIGUOUS_RECORD = "ambiguous-record"
    STALE_SOURCE = "stale-source"
    STALE_APPROVAL = "stale-approval"
    PARENT_CHANGED = "parent-changed"
    PARENT_NOT_PROMOTED = "parent-not-promoted"
    PARENT_NOT_MERGED = "parent-not-merged"
    PARENT_NOT_SYNCED = "parent-not-synced"
    MAIN_NOT_PUBLIC = "main-not-public"
    MAIN_CHANGED = "main-changed"


@dataclass(frozen=True, slots=True)
class SkippedReplay:
    reason: ReplaySkipReason


@dataclass(frozen=True, slots=True)
class ReadyReplay:
    queued: QueuedPromotion
    parent: MergedPromotedParent
    main: BranchRevision


type ReplayPlan = ReadyReplay | SkippedReplay


@dataclass(frozen=True, slots=True)
class DispatchedReplay:
    queued: QueuedPromotion
    run: WorkflowDispatch


type ReplayOutcome = DispatchedReplay | SkippedReplay


def _source_matches(source: PromotionPullRequest, queued: QueuedPromotion) -> bool:
    return (
        source.scope == queued.source
        and source.is_open
        and not source.draft
        and source.same_repository
        and source.details.base.repository == queued.base.repository
        and source.details.base.ref == queued.base.ref
        and source.details.base.sha == queued.base.sha
        and source.details.head.repository == queued.head.repository
        and source.details.head.ref == queued.head.ref
        and source.details.head.sha == queued.head.sha
    )


def _current_queue_approval(
    reader: QueueRecordReader, source: PromotionScope, head: CommitSha
) -> PromotionApproval | None:
    return current_ready_approval(source, head, reader.list_promotion_events(source))


def _approval_matches(reader: QueueRecordReader, queued: QueuedPromotion) -> bool:
    approval = _current_queue_approval(reader, queued.source, queued.approval.head)
    return approval is not None and queued.approval.matches(approval)


def _same_authority(left: QueuedPromotion, right: QueuedPromotion) -> bool:
    return (
        left.approval == right.approval
        and left.base == right.base
        and left.head == right.head
        and left.parent.source == right.parent.source
    )


def _covers(existing: QueuedPromotion, queued: QueuedPromotion) -> bool:
    return existing == queued or (
        _same_authority(existing, queued)
        and isinstance(existing.parent, PendingParentSync)
        and isinstance(queued.parent, PendingSourceParent)
    )


def _queue_record_for_approval(
    reader: QueueRecordReader,
    source: PromotionScope,
    approval: PromotionApproval | None,
) -> QueuedPromotion | SkippedReplay:
    comments = reader.list_promotion_comments(source)
    current: QueuedPromotion | None = None
    found_record = False
    identifiers: set[int] = set()
    for comment in sorted(
        comments, key=lambda value: (value.created_at, value.identifier)
    ):
        if comment.scope != source:
            raise ValueError("Queue comments belong to another pull request")
        if comment.identifier in identifiers:
            raise ValueError("Queue comments contain duplicate identities")
        identifiers.add(comment.identifier)
        queued = queued_promotion(comment)
        if queued is None:
            continue
        original = reader.get_unedited_promotion_comment(source, comment.identifier)
        if original is None or original.comment != comment:
            continue
        found_record = True
        if approval is None or queued.approval.event_id != approval.event_id:
            continue
        if current is not None:
            if not _same_authority(current, queued) or (
                isinstance(current.parent, PendingParentSync)
                and isinstance(queued.parent, PendingParentSync)
                and current.parent != queued.parent
            ):
                return SkippedReplay(ReplaySkipReason.AMBIGUOUS_RECORD)
            if _covers(current, queued):
                continue
        current = queued
    if current is None:
        return SkippedReplay(
            ReplaySkipReason.STALE_APPROVAL
            if found_record
            else ReplaySkipReason.NO_RECORD
        )
    return current


def current_queued_promotion(
    reader: QueueRecordReader, source: PromotionPullRequest
) -> QueuedPromotion | SkippedReplay:
    approval = _current_queue_approval(reader, source.scope, source.details.head.sha)
    current = _queue_record_for_approval(reader, source.scope, approval)
    if isinstance(current, SkippedReplay):
        return current
    if not _source_matches(source, current):
        return SkippedReplay(ReplaySkipReason.STALE_SOURCE)
    if not _approval_matches(reader, current):
        return SkippedReplay(ReplaySkipReason.STALE_APPROVAL)
    return current


def record_queue(
    reader: QueueRecordReader, writer: QueueWriter, queued: QueuedPromotion
) -> QueueRecordOutcome:
    source = reader.get_promotion_pull_request(queued.source)
    approval = _current_queue_approval(reader, queued.source, queued.approval.head)
    if (
        not _source_matches(source, queued)
        or approval is None
        or not queued.approval.matches(approval)
    ):
        return QueueRecordOutcome.STALE
    existing = _queue_record_for_approval(reader, queued.source, approval)
    if isinstance(existing, QueuedPromotion):
        if _covers(existing, queued):
            return QueueRecordOutcome.UNCHANGED
        if not _same_authority(existing, queued) or (
            isinstance(existing.parent, PendingParentSync)
            and isinstance(queued.parent, PendingParentSync)
        ):
            # The same readiness event cannot authorize a different revision
            # or parent. A fresh human approval can create a new record.
            return QueueRecordOutcome.STALE
    if (
        isinstance(existing, SkippedReplay)
        and existing.reason == ReplaySkipReason.AMBIGUOUS_RECORD
    ):
        raise ValueError("Conflicting promotion queue records")
    # This writer never edits old records. Each changed approval retains its own
    # exact head, and a retry of the same state does not create another comment.
    if not _source_matches(
        reader.get_promotion_pull_request(queued.source), queued
    ) or not _approval_matches(reader, queued):
        return QueueRecordOutcome.STALE
    writer.create_queue_comment(queued)
    return QueueRecordOutcome.RECORDED


def _main_contains(
    reader: PromotionComparisonReader,
    repository: RepositoryIdentity,
    main: CommitSha,
    commit: CommitSha,
) -> bool:
    comparison = reader.compare_commits(repository, commit, main)
    if (
        comparison.repository != repository
        or comparison.base != commit
        or comparison.head != main
    ):
        raise ValueError("Promotion reader compared different main commits")
    return comparison.is_ancestor


def is_public_main_revision(reader: PromotionRevisionReader, sha: CommitSha) -> bool:
    """Whether a revision is reachable from the current public uv main."""
    public_main = reader.get_ref(UV_REPOSITORY, "main")
    return public_main is not None and _main_contains(
        reader, UV_REPOSITORY, public_main, sha
    )


def plan_replay(
    reader: QueueReader,
    upstream_reader: ReplayUpstreamReader,
    queued: QueuedPromotion,
    main: BranchRevision,
) -> ReplayPlan:
    if main.repository != queued.source.repository or main.ref != "main":
        raise ValueError("Replay requires the synchronized source main revision")
    if not is_public_main_revision(upstream_reader, main.sha):
        return SkippedReplay(ReplaySkipReason.MAIN_NOT_PUBLIC)
    source = reader.get_promotion_pull_request(queued.source)
    if not _source_matches(source, queued):
        return SkippedReplay(ReplaySkipReason.STALE_SOURCE)
    if not _approval_matches(reader, queued):
        return SkippedReplay(ReplaySkipReason.STALE_APPROVAL)
    source_parent = reader.get_promotion_pull_request(queued.parent.source)
    if (
        source_parent.is_open
        or not source_parent.same_repository
        or source_parent.details.head.ref != queued.base.ref
        or source_parent.details.head.sha != queued.base.sha
    ):
        return SkippedReplay(ReplaySkipReason.PARENT_CHANGED)
    parent = verified_promoted_parent(
        reader, source_parent, upstream_reader=upstream_reader
    )
    match parent:
        case None:
            return SkippedReplay(ReplaySkipReason.PARENT_NOT_PROMOTED)
        case OpenPromotedParent() | ClosedPromotedParent():
            return SkippedReplay(ReplaySkipReason.PARENT_NOT_MERGED)
        case MergedPromotedParent():
            pass
        case _:
            assert_never(parent)
    match queued.parent:
        case PendingSourceParent():
            pass
        case PendingParentSync(upstream=expected, merge=merge):
            if parent.upstream.scope != expected or parent.merge.sha != merge:
                return SkippedReplay(ReplaySkipReason.PARENT_CHANGED)
        case _:
            assert_never(queued.parent)
    if not _main_contains(reader, main.repository, main.sha, parent.merge.sha):
        return SkippedReplay(ReplaySkipReason.PARENT_NOT_SYNCED)
    current_main = reader.get_ref(main.repository, "main")
    if current_main is None or not _main_contains(
        reader, main.repository, current_main, main.sha
    ):
        return SkippedReplay(ReplaySkipReason.MAIN_CHANGED)
    if not is_public_main_revision(upstream_reader, current_main):
        return SkippedReplay(ReplaySkipReason.MAIN_NOT_PUBLIC)
    if not _source_matches(reader.get_promotion_pull_request(queued.source), queued):
        return SkippedReplay(ReplaySkipReason.STALE_SOURCE)
    if not _approval_matches(reader, queued):
        return SkippedReplay(ReplaySkipReason.STALE_APPROVAL)
    return ReadyReplay(queued, parent, main)


def replay_one(
    reader: QueueReader,
    upstream_reader: ReplayUpstreamReader,
    writer: PromotionDispatcher,
    source: PromotionScope,
    main: BranchRevision,
    *,
    expected: QueuedPromotion | None = None,
    expected_parent: PromotionScope | None = None,
) -> ReplayOutcome:
    if source.repository != UV_DEV_REPOSITORY:
        raise ValueError("Automatic promotion replay is limited to uv-dev")
    current = current_queued_promotion(
        reader, reader.get_promotion_pull_request(source)
    )
    if isinstance(current, SkippedReplay):
        return current
    if expected is not None and not _covers(current, expected):
        return SkippedReplay(ReplaySkipReason.STALE_APPROVAL)
    if expected_parent is not None and current.parent.source != expected_parent:
        return SkippedReplay(ReplaySkipReason.PARENT_CHANGED)
    plan = plan_replay(reader, upstream_reader, current, main)
    match plan:
        case SkippedReplay():
            return plan
        case ReadyReplay():
            # The worker checks this exact event again; a newer readiness event
            # must never upgrade an already queued dispatch to a different head.
            run = writer.dispatch_promotion(plan.queued.approval)
            return DispatchedReplay(plan.queued, run)
    assert_never(plan)


def replay_queued_promotions(
    reader: QueueDiscoveryReader,
    upstream_reader: ReplayUpstreamReader,
    writer: PromotionDispatcher,
    main: BranchRevision,
    *,
    parent: PromotionScope | None = None,
) -> tuple[ReplayOutcome, ...]:
    if main.repository != UV_DEV_REPOSITORY or main.ref != "main":
        raise ValueError("Automatic promotion replay requires uv-dev/main")
    base = None
    if parent is not None:
        if parent.repository != UV_DEV_REPOSITORY:
            raise ValueError("Queued promotion parent must belong to uv-dev")
        source_parent = reader.get_promotion_pull_request(parent)
        if source_parent.is_open or not source_parent.same_repository:
            return ()
        base = source_parent.details.head.ref
    outcomes: list[ReplayOutcome] = []
    for source in reader.list_pull_requests(UV_DEV_REPOSITORY, base=base):
        if (
            source.draft
            or not source.same_repository
            or source.details.base.ref == "main"
        ):
            continue
        outcomes.append(
            replay_one(
                reader,
                upstream_reader,
                writer,
                source.scope,
                main,
                expected_parent=parent,
            )
        )
    return tuple(outcomes)
