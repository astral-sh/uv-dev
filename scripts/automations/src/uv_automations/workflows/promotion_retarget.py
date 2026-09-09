"""Retarget draft children after a verified promotion reaches source main."""

from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, assert_never

from uv_automations.github_promotion import (
    PromotedParentReader,
    PromotionRevisionReader,
    verified_promoted_parent,
)
from uv_automations.models import CommitSha, PullRequestState, RepositoryIdentity
from uv_automations.promotion_models import (
    UV_REPOSITORY,
    BranchRevision,
    ClosedPromotedParent,
    MergedPromotedParent,
    OpenPromotedParent,
    PromotionPullRequest,
    PromotionScope,
    PullRequestSelection,
    require_promotion_source,
)


class RetargetHistoryReader(PromotedParentReader, PromotionRevisionReader, Protocol):
    pass


class RetargetReader(RetargetHistoryReader, Protocol):
    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]: ...


class RetargetWriter(Protocol):
    def retarget_to_main(self, source: PromotionScope) -> PromotionPullRequest: ...


@dataclass(frozen=True, slots=True)
class RetargetPlan:
    """An in-memory optimistic update, never an Actions output or authority."""

    source: PromotionScope
    base: BranchRevision
    head: BranchRevision
    parent: MergedPromotedParent
    main: BranchRevision

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        if (
            self.main.repository != self.source.repository
            or self.main.ref != "main"
            or self.base.repository != self.source.repository
            or self.head.repository != self.source.repository
            or self.base.ref == "main"
            or self.head.ref == "main"
            or self.parent.source.scope.repository != self.source.repository
            or self.parent.source.scope == self.source
            or self.parent.source.details.head.ref != self.base.ref
            or self.parent.original_head != self.base.sha
        ):
            raise ValueError("Inconsistent promoted-child retarget plan")


@dataclass(frozen=True, slots=True)
class RetargetBatch:
    main: BranchRevision
    plans: tuple[RetargetPlan, ...]
    ready_children: int
    unverified_children: int

    def __post_init__(self) -> None:
        require_promotion_source(self.main.repository)
        if (
            self.main.ref != "main"
            or any(plan.main != self.main for plan in self.plans)
            or len({plan.source for plan in self.plans}) != len(self.plans)
        ):
            raise ValueError("Inconsistent promoted-child retarget batch")


@dataclass(frozen=True, slots=True)
class StaleRetargetSync:
    repository: RepositoryIdentity


type RetargetPreparation = RetargetBatch | StaleRetargetSync


class RetargetSkipReason(StrEnum):
    ALREADY_TARGETED = "already-targeted"
    MAIN_CHANGED = "main-changed"
    PARENT_CHANGED = "parent-changed"
    SOURCE_CHANGED = "source-changed"
    READY_FOR_PROMOTION = "ready-for-promotion"


@dataclass(frozen=True, slots=True)
class Retargeted:
    source: PromotionScope


@dataclass(frozen=True, slots=True)
class SkippedRetarget:
    source: PromotionScope
    reason: RetargetSkipReason


type RetargetOutcome = Retargeted | SkippedRetarget


def _is_source_pull_request(
    pull_request: PromotionPullRequest, repository: RepositoryIdentity
) -> bool:
    return (
        pull_request.scope.repository == repository
        and pull_request.details.base.repository == repository
        and pull_request.details.head.repository == repository
    )


def _public_main_contains(
    reader: RetargetHistoryReader, base: CommitSha, main: CommitSha
) -> bool:
    comparison = reader.compare_commits(UV_REPOSITORY, base, main)
    if (
        comparison.repository != UV_REPOSITORY
        or comparison.base != base
        or comparison.head != main
    ):
        raise ValueError("Retargeting compared different public revisions")
    return comparison.is_ancestor


def _matches_parent(parent: PromotionPullRequest, base: BranchRevision) -> bool:
    return (
        _is_source_pull_request(parent, base.repository)
        and parent.details.state == PullRequestState.CLOSED
        and parent.details.head.ref == base.ref
        and parent.details.head.sha == base.sha
    )


def _merged_parent(
    source_reader: RetargetReader,
    upstream_reader: RetargetHistoryReader,
    base: BranchRevision,
) -> MergedPromotedParent | None:
    parents = tuple(
        parent
        for parent in source_reader.list_pull_requests(
            base.repository, state=PullRequestSelection.CLOSED, head=base.ref
        )
        if _matches_parent(parent, base)
    )
    if len(parents) != 1:
        return None
    source_parent = source_reader.get_promotion_pull_request(parents[0].scope)
    if source_parent.scope != parents[0].scope or not _matches_parent(
        source_parent, base
    ):
        return None
    parent = verified_promoted_parent(
        source_reader, source_parent, upstream_reader=upstream_reader
    )
    match parent:
        case MergedPromotedParent():
            return parent
        case OpenPromotedParent() | ClosedPromotedParent() | None:
            return None
    assert_never(parent)


def plan_retargets(
    source_reader: RetargetReader,
    upstream_reader: RetargetHistoryReader,
    repository: RepositoryIdentity,
    main_sha: CommitSha,
) -> RetargetPreparation:
    """Find eligible source stacks with bounded, read-only discovery."""
    require_promotion_source(repository)
    if source_reader.get_ref(repository, "main") != main_sha:
        return StaleRetargetSync(repository)

    public_main = upstream_reader.get_ref(UV_REPOSITORY, "main")
    if public_main is None or not _public_main_contains(
        upstream_reader, main_sha, public_main
    ):
        raise ValueError("The synchronized revision is not on public main")

    main = BranchRevision(repository, "main", main_sha)
    stacks: dict[BranchRevision, list[PromotionPullRequest]] = {}
    ready_children = 0
    seen: set[PromotionScope] = set()
    for child in source_reader.list_pull_requests(repository):
        if child.scope in seen:
            raise ValueError("Duplicate pull request in retarget discovery")
        seen.add(child.scope)
        if (
            not child.is_open
            or not _is_source_pull_request(child, repository)
            or child.details.base.ref == "main"
            or child.details.head.ref == "main"
        ):
            continue
        if not child.draft:
            # Readiness is publication authority. Retargeting it would change
            # the base under the promotion/replay worker's approved snapshot.
            ready_children += 1
            continue
        base = BranchRevision(
            repository, child.details.base.ref, child.details.base.sha
        )
        stacks.setdefault(base, []).append(child)

    plans: list[RetargetPlan] = []
    unverified_children = 0
    for base, children in stacks.items():
        parent = _merged_parent(source_reader, upstream_reader, base)
        if parent is None or not _public_main_contains(
            upstream_reader, parent.merge.sha, main_sha
        ):
            unverified_children += len(children)
            continue
        plans.extend(
            RetargetPlan(
                source=child.scope,
                base=base,
                head=BranchRevision(
                    repository, child.details.head.ref, child.details.head.sha
                ),
                parent=parent,
                main=main,
            )
            for child in children
        )
    return RetargetBatch(main, tuple(plans), ready_children, unverified_children)


def _same_head(pull_request: PromotionPullRequest, plan: RetargetPlan) -> bool:
    return (
        _is_source_pull_request(pull_request, plan.source.repository)
        and pull_request.scope == plan.source
        and pull_request.details.head.ref == plan.head.ref
        and pull_request.details.head.sha == plan.head.sha
    )


def _same_parent(parent: MergedPromotedParent, expected: MergedPromotedParent) -> bool:
    return (
        parent.record == expected.record
        and parent.source.scope == expected.source.scope
        and parent.source.details.head.ref == expected.source.details.head.ref
        and parent.original_head == expected.original_head
        and parent.merge == expected.merge
    )


def retarget(
    source_reader: RetargetReader,
    upstream_reader: RetargetHistoryReader,
    writer: RetargetWriter,
    plan: RetargetPlan,
) -> RetargetOutcome:
    """Revalidate the plan and perform only a PR-base update.

    GitHub has no expected-head/draft precondition for this mutation. The last
    read is therefore an optimistic freshness check, not an atomic lease. The
    returned PR is checked too; concurrent changes are never silently accepted.
    """
    if source_reader.get_ref(plan.main.repository, "main") != plan.main.sha:
        return SkippedRetarget(plan.source, RetargetSkipReason.MAIN_CHANGED)

    public_main = upstream_reader.get_ref(UV_REPOSITORY, "main")
    if public_main is None or not _public_main_contains(
        upstream_reader, plan.main.sha, public_main
    ):
        return SkippedRetarget(plan.source, RetargetSkipReason.MAIN_CHANGED)

    parent = _merged_parent(source_reader, upstream_reader, plan.base)
    if (
        parent is None
        or not _same_parent(parent, plan.parent)
        or not _public_main_contains(upstream_reader, parent.merge.sha, plan.main.sha)
    ):
        return SkippedRetarget(plan.source, RetargetSkipReason.PARENT_CHANGED)

    # Parent verification can require several API calls. Check the pinned main
    # again, then make the child read the final operation before publication.
    if source_reader.get_ref(plan.main.repository, "main") != plan.main.sha:
        return SkippedRetarget(plan.source, RetargetSkipReason.MAIN_CHANGED)
    current = source_reader.get_promotion_pull_request(plan.source)
    if (
        not current.is_open
        or not _same_head(current, plan)
        or current.details.base.ref not in (plan.base.ref, "main")
    ):
        return SkippedRetarget(plan.source, RetargetSkipReason.SOURCE_CHANGED)
    if current.details.base.ref == "main":
        return SkippedRetarget(plan.source, RetargetSkipReason.ALREADY_TARGETED)
    if current.details.base.sha != plan.base.sha:
        return SkippedRetarget(plan.source, RetargetSkipReason.SOURCE_CHANGED)
    if not current.draft:
        return SkippedRetarget(plan.source, RetargetSkipReason.READY_FOR_PROMOTION)

    updated = writer.retarget_to_main(plan.source)
    if (
        not updated.is_open
        or not updated.draft
        or not _same_head(updated, plan)
        or updated.details.base.ref != "main"
        or updated.details.base.sha != plan.main.sha
    ):
        raise ValueError("The pull request changed while retargeting it")
    return Retargeted(plan.source)


def apply_retargets(
    source_reader: RetargetReader,
    upstream_reader: RetargetHistoryReader,
    writer: RetargetWriter,
    batch: RetargetBatch,
) -> tuple[RetargetOutcome, ...]:
    return tuple(
        retarget(source_reader, upstream_reader, writer, plan) for plan in batch.plans
    )
