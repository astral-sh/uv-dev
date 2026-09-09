"""Read-only plans for the existing direct pull request promotion workflow."""

from dataclasses import dataclass
from typing import assert_never

from uv_automations.github_promotion import PromotionReader, verified_promoted_parent
from uv_automations.json import (
    as_object,
    as_positive_integer,
    require_keys,
)
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    BranchRevision,
    LabelAddedEvent,
    MergedPromotedParent,
    PromotionApproval,
    PromotionApprovalClaim,
    PromotionApprovalKind,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    PullRequestSelection,
    UnrecordedMergedParent,
    latest_label_event,
    latest_ready_event,
    ready_approval,
    require_promotion_source,
)

PRIVATE_PUBLISH_LABEL = "bot:promote"


@dataclass(frozen=True, slots=True)
class PromotionRequest:
    source: PromotionScope
    head: CommitSha
    approval_id: int | None = None
    approval_kind: PromotionApprovalKind | None = None
    ready_event_id: int | None = None

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        if self.approval_id is not None:
            as_positive_integer(self.approval_id)
        if self.ready_event_id is not None:
            as_positive_integer(self.ready_event_id)
        if self.approval_kind is not None and self.approval_id is None:
            raise ValueError("An approval kind requires its exact event ID")
        match self.kind:
            case PromotionApprovalKind.READY_FOR_REVIEW:
                if self.ready_event_id is not None and (
                    self.approval_id is None or self.ready_event_id != self.approval_id
                ):
                    raise ValueError("Readiness request has a different event ID")
                return
            case PromotionApprovalKind.LABELED:
                if (
                    self.source.repository != UV_SECURITY_REPOSITORY
                    or self.approval_id is None
                    or self.ready_event_id is None
                    or self.ready_event_id >= self.approval_id
                ):
                    raise ValueError("Incomplete private publication approval")
                return
        assert_never(self.kind)

    @property
    def kind(self) -> PromotionApprovalKind:
        return self.approval_kind or PromotionApprovalKind.READY_FOR_REVIEW


def _branch(pull_request: PromotionPullRequest, *, head: bool) -> BranchRevision:
    revision = pull_request.details.head if head else pull_request.details.base
    if revision.repository is None:
        raise ValueError("The pull request branch repository no longer exists")
    return BranchRevision(revision.repository, revision.ref, revision.sha)


def _require_ready(source: PromotionPullRequest, approval: PromotionApproval) -> None:
    if (
        source.scope != approval.source
        or not source.is_open
        or source.draft
        or not source.same_repository
        or source.details.head.sha != approval.head
    ):
        raise ValueError("Promotion plan does not describe the approved source")


@dataclass(frozen=True, slots=True)
class CopyUpstreamBaseClaim:
    """Untrusted preconditions for a later narrowly scoped ref-creation stage."""

    approval: PromotionApprovalClaim
    source_base: BranchRevision
    source_head: BranchRevision
    parent: PromotionScope
    parent_base: BranchRevision
    parent_head: BranchRevision
    destination: BranchRevision

    def __post_init__(self) -> None:
        source_repository = self.approval.source.repository
        if (
            self.source_base.repository != source_repository
            or self.source_head.repository != source_repository
            or self.source_head.sha != self.approval.head
            or self.parent.repository != UV_REPOSITORY
            or self.parent_base.repository != UV_REPOSITORY
            or self.parent_head.repository != source_repository
            or self.destination.repository != UV_REPOSITORY
            or self.source_base.ref != self.parent_head.ref
            or self.source_base.ref != self.destination.ref
            or self.source_base.sha != self.parent_head.sha
            or self.source_base.sha != self.destination.sha
        ):
            raise ValueError("Inconsistent upstream-base copy preconditions")

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "approval": self.approval.to_json(),
            "source_base": self.source_base.to_json(),
            "source_head": self.source_head.to_json(),
            "parent": self.parent.to_json(),
            "parent_base": self.parent_base.to_json(),
            "parent_head": self.parent_head.to_json(),
            "destination": self.destination.to_json(),
        }

    @classmethod
    def from_json(cls, value: object) -> CopyUpstreamBaseClaim:
        data = as_object(value)
        require_keys(
            data,
            {
                "version",
                "approval",
                "source_base",
                "source_head",
                "parent",
                "parent_base",
                "parent_head",
                "destination",
            },
        )
        if as_positive_integer(data["version"]) != 1:
            raise ValueError("Unsupported upstream-base copy version")
        return cls(
            PromotionApprovalClaim.from_json(data["approval"]),
            BranchRevision.from_json(data["source_base"]),
            BranchRevision.from_json(data["source_head"]),
            PromotionScope.from_json(data["parent"]),
            BranchRevision.from_json(data["parent_base"]),
            BranchRevision.from_json(data["parent_head"]),
            BranchRevision.from_json(data["destination"]),
        )

    def matches(self, plan: CopyUpstreamBase) -> bool:
        return self == plan.claim


@dataclass(frozen=True, slots=True)
class CopyUpstreamBase:
    source: PromotionPullRequest
    approval: PromotionApproval
    parent: PromotionPullRequest
    destination: BranchRevision

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        if not self.parent.is_open:
            raise ValueError("Upstream-base copy requires current open pull requests")
        # Constructing the untrusted claim checks all ref/identity relationships.
        _ = self.claim

    @property
    def claim(self) -> CopyUpstreamBaseClaim:
        return CopyUpstreamBaseClaim(
            self.approval.claim,
            _branch(self.source, head=False),
            _branch(self.source, head=True),
            self.parent.scope,
            _branch(self.parent, head=False),
            _branch(self.parent, head=True),
            self.destination,
        )

    def to_json(self) -> dict[str, object]:
        return self.claim.to_json()


type MergedParent = MergedPromotedParent | UnrecordedMergedParent


def _require_merged_parent(
    source: PromotionPullRequest, parent: MergedParent, base: BranchRevision
) -> None:
    if (
        parent.original_head != source.details.base.sha
        or parent.upstream.details.head.ref != source.details.base.ref
        or base.repository != source.scope.repository
        or base.ref != parent.upstream.details.base.ref
    ):
        raise ValueError("Promotion plan has different merged-parent preconditions")


@dataclass(frozen=True, slots=True)
class Publish:
    source: PromotionPullRequest
    approval: PromotionApproval
    base: BranchRevision
    copy_base: CopyUpstreamBase | None = None

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        if (
            self.base.repository != UV_REPOSITORY
            or self.base.ref != self.source.details.base.ref
            or (
                self.copy_base is not None
                and (
                    self.copy_base.source != self.source
                    or self.copy_base.approval != self.approval
                    or self.copy_base.destination != self.base
                )
            )
        ):
            raise ValueError("Promotion plan has different upstream-base preconditions")


@dataclass(frozen=True, slots=True)
class Rebase:
    source: PromotionPullRequest
    approval: PromotionApproval
    parent: MergedParent
    base: BranchRevision
    previous_base: CommitSha

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        _require_merged_parent(self.source, self.parent, self.base)
        if self.previous_base != self.source.details.base.sha:
            raise ValueError("Promotion rebase has a different previous base")


@dataclass(frozen=True, slots=True)
class WaitForParent:
    source: PromotionPullRequest
    approval: PromotionApproval
    parent: PromotionPullRequest

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        if not self.parent.is_open or not _exact_source_parent(
            self.source, self.parent
        ):
            raise ValueError("Promotion wait has a different open source parent")


@dataclass(frozen=True, slots=True)
class WaitForSync:
    source: PromotionPullRequest
    approval: PromotionApproval
    parent: MergedParent
    base: BranchRevision

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        _require_merged_parent(self.source, self.parent, self.base)


@dataclass(frozen=True, slots=True)
class AlreadyPublished:
    source: PromotionPullRequest
    approval: PromotionApproval
    upstream: PromotionPullRequest
    base: BranchRevision

    def __post_init__(self) -> None:
        _require_ready(self.source, self.approval)
        if (
            self.base.repository != UV_REPOSITORY
            or self.upstream.scope.repository != UV_REPOSITORY
            or not self.upstream.is_open
            or not self.upstream.same_repository
            or self.upstream.details.base.ref != self.base.ref
            or self.upstream.details.head.ref != self.source.details.head.ref
            or self.upstream.details.head.sha != self.approval.head
        ):
            raise ValueError("Existing publication has different preconditions")


@dataclass(frozen=True, slots=True)
class Stale:
    source: PromotionPullRequest
    expected_head: CommitSha
    reason: str
    approval: PromotionApproval | None = None

    def __post_init__(self) -> None:
        if self.approval is not None and (
            self.approval.source != self.source.scope
            or self.approval.head != self.expected_head
        ):
            raise ValueError("Stale promotion has different approval preconditions")


@dataclass(frozen=True, slots=True)
class Rejected:
    source: PromotionPullRequest
    reason: str
    approval: PromotionApproval | None = None

    def __post_init__(self) -> None:
        if self.approval is not None and self.approval.source != self.source.scope:
            raise ValueError("Rejected promotion has a different approval source")


type PromotionPlan = (
    Publish | Rebase | WaitForParent | WaitForSync | AlreadyPublished | Stale | Rejected
)


def _current_approval(
    source: PromotionPullRequest,
    head: CommitSha,
    events: tuple[PromotionEvent, ...],
    kind: PromotionApprovalKind,
) -> PromotionApproval | None:
    match kind:
        case PromotionApprovalKind.READY_FOR_REVIEW:
            return ready_approval(source.scope, head, events)
        case PromotionApprovalKind.LABELED:
            readiness = latest_ready_event(events)
            label = latest_label_event(events, PRIVATE_PUBLISH_LABEL)
            if (
                source.scope.repository != UV_SECURITY_REPOSITORY
                or PRIVATE_PUBLISH_LABEL not in source.details.labels
                or readiness is None
                or not isinstance(label, LabelAddedEvent)
                or label.actor is None
                or not label.actor.is_human
                or label.identifier <= readiness.identifier
            ):
                return None
            return PromotionApproval(source.scope, head, label, readiness.identifier)
    assert_never(kind)


def _publish_plan(
    upstream_reader: PromotionReader,
    source: PromotionPullRequest,
    approval: PromotionApproval,
    base: BranchRevision,
    copy_base: CopyUpstreamBase | None = None,
) -> Publish | AlreadyPublished | Rejected:
    existing = tuple(
        pull_request
        for pull_request in upstream_reader.list_pull_requests(
            UV_REPOSITORY,
            state=PullRequestSelection.ALL,
            head=source.details.head.ref,
        )
        if pull_request.same_repository
    )
    if len(existing) > 1:
        return Rejected(source, "Found multiple upstream pull requests", approval)
    if existing:
        upstream = existing[0]
        if (
            not upstream.is_open
            or upstream.details.base.ref != base.ref
            or upstream.details.head.ref != source.details.head.ref
            or upstream.details.head.sha != approval.head
        ):
            return Rejected(
                source,
                "An upstream pull request has a different state, base, or head",
                approval,
            )
        return AlreadyPublished(source, approval, upstream, base)
    return Publish(source, approval, base, copy_base)


def _source_parent(
    reader: PromotionReader,
    source: PromotionPullRequest,
    state: PullRequestSelection,
) -> tuple[PromotionPullRequest, ...]:
    return reader.list_pull_requests(
        source.scope.repository, state=state, head=source.details.base.ref
    )


def _exact_source_parent(
    source: PromotionPullRequest, parent: PromotionPullRequest
) -> bool:
    return (
        parent.scope.repository == source.scope.repository
        and parent.same_repository
        and parent.details.head.ref == source.details.base.ref
        and parent.details.head.sha == source.details.base.sha
    )


def _merged_parent(
    source_reader: PromotionReader,
    upstream_reader: PromotionReader,
    source: PromotionPullRequest,
    upstream: PromotionPullRequest,
) -> MergedParent | None:
    closed_sources = tuple(
        parent
        for parent in _source_parent(source_reader, source, PullRequestSelection.CLOSED)
        if _exact_source_parent(source, parent)
    )
    if len(closed_sources) == 1:
        recorded = verified_promoted_parent(
            source_reader, closed_sources[0], upstream_reader=upstream_reader
        )
        if (
            isinstance(recorded, MergedPromotedParent)
            and recorded.upstream.scope == upstream.scope
        ):
            return recorded
    if upstream.details.head.sha == source.details.base.sha:
        return UnrecordedMergedParent(upstream, source.details.base.sha)
    return None


def plan_promotion(
    reader: PromotionReader,
    request: PromotionRequest,
    *,
    upstream_reader: PromotionReader | None = None,
    approval: PromotionApproval | None = None,
) -> PromotionPlan:
    """Read the complete prepare-stage decision without acquiring write authority."""
    upstream_reader = upstream_reader or reader
    source = reader.get_promotion_pull_request(request.source)
    if source.scope != request.source:
        raise ValueError("Promotion reader returned a different source pull request")
    if not source.is_open or source.draft:
        return Stale(source, request.head, "The source is no longer ready")
    if not source.same_repository:
        return Rejected(source, "The source uses a different head repository")
    current_approval = _current_approval(
        source, request.head, reader.list_promotion_events(request.source), request.kind
    )
    if current_approval is None:
        if request.approval_id is not None or approval is not None:
            return Stale(source, request.head, "The promotion approval changed")
        return Rejected(source, "The pull request has no current human approval")
    if (
        (approval is not None and not approval.claim.matches(current_approval))
        or (
            request.approval_id is not None
            and request.approval_id != current_approval.event_id
        )
        or (
            request.ready_event_id is not None
            and request.ready_event_id != current_approval.ready_event_id
        )
    ):
        return Stale(source, request.head, "The promotion approval changed")
    explicit_approval = request.approval_id is not None or approval is not None
    approval = current_approval
    if source.details.head.sha != request.head:
        return Stale(
            source,
            request.head,
            "The approved head changed",
            approval if explicit_approval else None,
        )
    if source.scope.repository == UV_SECURITY_REPOSITORY:
        if PRIVATE_PUBLISH_LABEL not in source.details.labels:
            return Rejected(source, "Private publication approval is missing", approval)
        if not reader.get_repository_permission(
            source.scope.repository, approval.actor
        ).can_write:
            return Rejected(
                source, "The private approver is not a repository writer", approval
            )

    base_ref = source.details.base.ref
    base_head = source.details.base.sha
    upstream_base = upstream_reader.get_ref(UV_REPOSITORY, base_ref)
    if upstream_base is not None:
        return _publish_plan(
            upstream_reader,
            source,
            approval,
            BranchRevision(UV_REPOSITORY, base_ref, upstream_base),
        )

    open_upstream = upstream_reader.list_pull_requests(
        UV_REPOSITORY, state=PullRequestSelection.OPEN, head=base_ref
    )
    if open_upstream:
        if len(open_upstream) != 1:
            return Rejected(source, "Found multiple open upstream parents", approval)
        parent = open_upstream[0]
        if (
            parent.details.head.repository != source.scope.repository
            or parent.details.head.ref != base_ref
            or parent.details.head.sha != base_head
        ):
            return Rejected(source, "The open upstream parent does not match", approval)
        destination = BranchRevision(UV_REPOSITORY, base_ref, base_head)
        copy_base = CopyUpstreamBase(source, approval, parent, destination)
        return _publish_plan(upstream_reader, source, approval, destination, copy_base)

    open_sources = _source_parent(reader, source, PullRequestSelection.OPEN)
    if open_sources:
        if len(open_sources) != 1 or not _exact_source_parent(source, open_sources[0]):
            return Rejected(source, "The open source parent does not match", approval)
        return WaitForParent(source, approval, open_sources[0])

    merged_upstream = tuple(
        parent
        for parent in upstream_reader.list_pull_requests(
            UV_REPOSITORY, state=PullRequestSelection.CLOSED, head=base_ref
        )
        if parent.merge is not None
    )
    if len(merged_upstream) != 1:
        return Rejected(source, "No unique merged upstream parent exists", approval)
    upstream = upstream_reader.get_promotion_pull_request(merged_upstream[0].scope)
    if (
        not upstream.same_repository
        or upstream.details.head.ref != base_ref
        or upstream.merge is None
    ):
        return Rejected(source, "The merged upstream parent changed", approval)
    merged = _merged_parent(reader, upstream_reader, source, upstream)
    if merged is None:
        return Rejected(
            source, "The merged parent provenance could not be verified", approval
        )

    target_ref = merged.upstream.details.base.ref
    target_head = reader.get_ref(source.scope.repository, target_ref)
    if target_head is None:
        return Rejected(
            source, "The merged parent destination is unavailable", approval
        )
    target = BranchRevision(source.scope.repository, target_ref, target_head)
    comparison = reader.compare_commits(
        source.scope.repository, merged.merge.sha, target_head
    )
    if (
        comparison.repository != source.scope.repository
        or comparison.base != merged.merge.sha
        or comparison.head != target_head
    ):
        raise ValueError("Promotion reader compared different parent commits")
    if not comparison.is_ancestor:
        return WaitForSync(source, approval, merged, target)
    return Rebase(source, approval, merged, target, base_head)
