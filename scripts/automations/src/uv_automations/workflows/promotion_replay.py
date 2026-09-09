"""Read-only worker plans for an exact, previously queued public promotion."""

from typing import assert_never

from uv_automations.github_promotion import parent_sync
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    BranchRevision,
    PromotionApprovalKind,
    PromotionPullRequest,
    SynchronizedParent,
    UnsynchronizedParent,
    current_ready_approval,
)
from uv_automations.workflows.promotion import (
    PromotionPlan,
    PromotionRequest,
    Rebase,
    Stale,
)
from uv_automations.workflows.promotion_queue import (
    QueuedPromotion,
    QueueReader,
    ReadyReplay,
    ReplayUpstreamReader,
    SkippedReplay,
    current_queued_promotion,
    plan_replay,
)


def _same_source_revision(
    expected: PromotionPullRequest, current: PromotionPullRequest
) -> bool:
    return (
        current.scope == expected.scope
        and current.is_open
        and not current.draft
        and current.same_repository
        and current.details.base == expected.details.base
        and current.details.head == expected.details.head
    )


def plan_queued_promotion(
    reader: QueueReader,
    request: PromotionRequest,
    *,
    upstream_reader: ReplayUpstreamReader | None = None,
) -> PromotionPlan:
    """Revalidate queued authority and plan its merged-parent update onto main.

    A surviving or recreated upstream parent branch is not a replay destination.
    Manual promotion keeps its existing-branch behavior in ``plan_promotion``.
    """
    if (
        request.source.repository != UV_DEV_REPOSITORY
        or request.kind != PromotionApprovalKind.READY_FOR_REVIEW
        or request.approval_id is None
    ):
        raise ValueError("Queued promotion requires an exact public readiness event")

    upstream_reader = upstream_reader or reader
    source = reader.get_promotion_pull_request(request.source)
    if source.scope != request.source:
        raise ValueError("Promotion reader returned a different source pull request")
    queued = current_queued_promotion(reader, source)
    match queued:
        case SkippedReplay():
            return Stale(
                source, request.head, "The queued promotion is no longer current"
            )
        case QueuedPromotion():
            if (
                queued.approval.head != request.head
                or queued.approval.event_id != request.approval_id
                or queued.approval.kind != request.kind
                or (
                    request.ready_event_id is not None
                    and queued.approval.ready_event_id != request.ready_event_id
                )
            ):
                return Stale(source, request.head, "The queued approval changed")
        case _:
            assert_never(queued)

    main_sha = reader.get_ref(UV_DEV_REPOSITORY, "main")
    if main_sha is None:
        return Stale(source, request.head, "The source main branch is unavailable")
    replay = plan_replay(
        reader,
        upstream_reader,
        queued,
        BranchRevision(UV_DEV_REPOSITORY, "main", main_sha),
    )
    match replay:
        case SkippedReplay():
            return Stale(
                source, request.head, "The queued parent is no longer replayable"
            )
        case ReadyReplay():
            pass
        case _:
            assert_never(replay)

    synchronized = parent_sync(reader, replay.parent, replay.main)
    match synchronized:
        case UnsynchronizedParent():
            return Stale(
                source, request.head, "The queued parent is no longer synchronized"
            )
        case SynchronizedParent():
            pass
        case _:
            assert_never(synchronized)

    current_source = reader.get_promotion_pull_request(request.source)
    if current_source.scope != request.source:
        raise ValueError("Promotion reader returned a different source pull request")
    if not _same_source_revision(source, current_source):
        return Stale(current_source, request.head, "The queued source changed")
    approval = current_ready_approval(
        request.source, request.head, reader.list_promotion_events(request.source)
    )
    if approval is None or not queued.approval.matches(approval):
        return Stale(current_source, request.head, "The queued approval changed")
    return Rebase(
        current_source,
        approval,
        synchronized.parent,
        synchronized.main,
        queued.base.sha,
    )
