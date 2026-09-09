"""Freshness checks for the existing promotion publisher's base-copy step."""

from enum import StrEnum
from typing import Protocol, assert_never

from uv_automations.github_promotion import PromotionReader
from uv_automations.promotion_models import BranchRevision, PromotionPullRequest
from uv_automations.workflows.promotion import (
    AlreadyPublished,
    CopyUpstreamBaseClaim,
    PromotionRequest,
    Publish,
    Rebase,
    Rejected,
    Stale,
    WaitForParent,
    WaitForSync,
    plan_promotion,
)


class UpstreamBaseWriter(Protocol):
    def create_upstream_base(self, destination: BranchRevision) -> None: ...


class BaseCopyOutcome(StrEnum):
    CREATED = "created"
    UNCHANGED = "unchanged"
    STALE = "stale"


def _source_matches(source: PromotionPullRequest, claim: CopyUpstreamBaseClaim) -> bool:
    return (
        source.scope == claim.approval.source
        and source.details.base.repository == claim.source_base.repository
        and source.details.base.ref == claim.source_base.ref
        and source.details.base.sha == claim.source_base.sha
        and source.details.head.repository == claim.source_head.repository
        and source.details.head.ref == claim.source_head.ref
        and source.details.head.sha == claim.source_head.sha
    )


def ensure_upstream_base(
    reader: PromotionReader,
    upstream_reader: PromotionReader,
    writer: UpstreamBaseWriter,
    claim: CopyUpstreamBaseClaim,
) -> BaseCopyOutcome:
    approval = claim.approval
    request = PromotionRequest(
        approval.source,
        approval.head,
        approval.event_id,
        approval.kind,
        approval.ready_event_id,
    )
    plan = plan_promotion(reader, request, upstream_reader=upstream_reader)
    match plan:
        case Publish() | AlreadyPublished():
            if (
                not _source_matches(plan.source, claim)
                or not claim.approval.matches(plan.approval)
                or plan.base != claim.destination
            ):
                return BaseCopyOutcome.STALE
            if isinstance(plan, AlreadyPublished) or plan.copy_base is None:
                return BaseCopyOutcome.UNCHANGED
            if not claim.matches(plan.copy_base):
                return BaseCopyOutcome.STALE
            writer.create_upstream_base(plan.copy_base.destination)
            return BaseCopyOutcome.CREATED
        case Rebase() | WaitForParent() | WaitForSync() | Stale() | Rejected():
            return BaseCopyOutcome.STALE
    assert_never(plan)
