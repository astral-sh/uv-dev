"""Read-only recognition of an exact, directly promoted public revision."""

from dataclasses import dataclass
from typing import Protocol

from uv_automations.github_promotion import PromotionReceiptReader
from uv_automations.models import ActorKind, CommitSha, PullRequestState
from uv_automations.promotion_models import (
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    AmbiguousPromotionRecord,
    PromotionComment,
    PromotionPullRequest,
    PromotionRecord,
    PromotionScope,
    UneditedPromotionComment,
    promotion_record,
    unique_promotion_record,
)
from uv_automations.workflows.promotion import PromotionRequest


class ClosedPromotionReader(PromotionReceiptReader, Protocol):
    def get_promotion_pull_request(
        self, scope: PromotionScope
    ) -> PromotionPullRequest: ...

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]: ...


def _source_matches(
    source: PromotionPullRequest, scope: PromotionScope, head: CommitSha
) -> bool:
    return (
        scope.repository == UV_DEV_REPOSITORY
        and source.scope == scope
        and source.details.state == PullRequestState.CLOSED
        and source.merge is None
        and source.same_repository
        and source.details.head.sha == head
    )


def _upstream_matches(
    source: PromotionPullRequest,
    upstream: PromotionPullRequest,
    record: PromotionRecord,
) -> bool:
    author = upstream.author
    return (
        record.source == source.scope
        and record.upstream == upstream.scope
        and upstream.scope.repository == UV_REPOSITORY
        and upstream.same_repository
        and (upstream.is_open or upstream.merge is not None)
        and upstream.details.base.ref == source.details.base.ref
        and upstream.details.head.ref == source.details.head.ref
        and upstream.details.head.sha == source.details.head.sha
        and author is not None
        and author.database_id == AUTOMATIONS_BOT_ID
        and author.kind == ActorKind.BOT
    )


def _same_observation(
    expected: PromotionPullRequest, current: PromotionPullRequest
) -> bool:
    # Base tips can advance while the source and publication still target the
    # same branch. This observation grants no authority to update either base.
    return (
        current.scope == expected.scope
        and current.details.state == expected.details.state
        and current.details.base.repository == expected.details.base.repository
        and current.details.base.ref == expected.details.base.ref
        and current.details.head == expected.details.head
        and current.merge == expected.merge
    )


@dataclass(frozen=True, slots=True)
class ObservedClosedPromotion:
    """An observed publication and closure, not approval for another write."""

    source: PromotionPullRequest
    head: CommitSha
    upstream: PromotionPullRequest
    receipt: UneditedPromotionComment

    def __post_init__(self) -> None:
        record = promotion_record(self.receipt.comment)
        if (
            record is None
            or not _source_matches(self.source, self.source.scope, self.head)
            or not _upstream_matches(self.source, self.upstream, record)
        ):
            raise ValueError("Inconsistent closed promotion observation")


def inspect_completed_promotion(
    reader: ClosedPromotionReader, request: PromotionRequest
) -> ObservedClosedPromotion | None:
    """Recognize only an already-published exact direct uv-dev revision.

    A historical receipt does not bind a rebased candidate or an approval event.
    Queued and private promotions therefore retain their separate planners.
    """
    if (
        request.source.repository != UV_DEV_REPOSITORY
        or request.approval_id is not None
    ):
        return None
    source = reader.get_promotion_pull_request(request.source)
    if source.scope != request.source:
        raise ValueError("Promotion reader returned a different source pull request")
    if not _source_matches(source, request.source, request.head):
        return None
    try:
        record = unique_promotion_record(
            source.scope, reader.list_promotion_comments(source.scope)
        )
    except AmbiguousPromotionRecord:
        return None
    if record is None:
        return None
    receipt = reader.get_unedited_promotion_comment(source.scope, record.comment_id)
    if receipt is None or promotion_record(receipt.comment) != record:
        return None
    upstream = reader.get_promotion_pull_request(record.upstream)
    if not _upstream_matches(source, upstream, record):
        return None

    # Receipt verification performs several API reads. Observe both PRs again
    # before reporting success; a changed publication remains an ordinary no-op.
    current_source = reader.get_promotion_pull_request(request.source)
    current_upstream = reader.get_promotion_pull_request(record.upstream)
    if (
        not _same_observation(source, current_source)
        or not _same_observation(upstream, current_upstream)
        or not _source_matches(current_source, request.source, request.head)
        or not _upstream_matches(current_source, current_upstream, record)
    ):
        return None
    return ObservedClosedPromotion(
        current_source, request.head, current_upstream, receipt
    )
