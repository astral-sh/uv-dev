import unittest
from dataclasses import dataclass, field, replace
from unittest.mock import patch

from uv_automations.models import (
    ActorKind,
    CommitSha,
    PullRequestDetails,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    Timestamp,
)
from uv_automations.promotion_models import (
    AUTOMATIONS_APP,
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    BranchRevision,
    CommitComparison,
    ComparisonStatus,
    ConvertedToDraftEvent,
    HeadForcePush,
    PromotionActor,
    PromotionApproval,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    PullRequestMerge,
    ReadyForReviewEvent,
    UneditedPromotionComment,
)
from uv_automations.workflows.promotion import PromotionRequest, Rebase, Stale
from uv_automations.workflows.promotion_queue import (
    QueuedPromotion,
    ReadyReplay,
    plan_replay,
)
from uv_automations.workflows.promotion_replay import plan_queued_promotion

HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
MERGE = CommitSha("c" * 40)
MAIN = CommitSha("d" * 40)
OLD_MAIN = CommitSha("e" * 40)
OTHER = CommitSha("f" * 40)
TIME = Timestamp.parse("2026-09-09T12:00:00Z")
HUMAN = PromotionActor("zanieb", 101, ActorKind.USER)
BOT = PromotionActor("astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT)
CHILD = PromotionScope(UV_DEV_REPOSITORY, 20)
SOURCE_PARENT = PromotionScope(UV_DEV_REPOSITORY, 10)
UPSTREAM_PARENT = PromotionScope(UV_REPOSITORY, 100)
READY = ReadyForReviewEvent(1000, HUMAN, TIME)
APPROVAL = PromotionApproval(CHILD, HEAD, READY, READY.identifier)
REQUEST = PromotionRequest(CHILD, HEAD, READY.identifier)
MAIN_REVISION = BranchRevision(UV_DEV_REPOSITORY, "main", MAIN)


def pull_request(
    scope: PromotionScope,
    *,
    base_ref: str = "main",
    base_sha: CommitSha = OLD_MAIN,
    head_ref: str = "parent",
    head_sha: CommitSha = BASE,
    state: PullRequestState = PullRequestState.OPEN,
    author: PromotionActor = HUMAN,
    merge: PullRequestMerge | None = None,
) -> PromotionPullRequest:
    return PromotionPullRequest(
        scope,
        PullRequestDetails(
            scope.reference,
            state,
            f"https://github.com/{scope.repository.name}/pull/{scope.number}",
            PullRequestRevision(scope.repository, base_ref, base_sha),
            PullRequestRevision(scope.repository, head_ref, head_sha),
            (),
        ),
        False,
        author,
        "A pull request",
        "",
        merge,
    )


def comment(scope: PromotionScope, body: str, identifier: int) -> PromotionComment:
    return PromotionComment(scope, identifier, BOT, AUTOMATIONS_APP, body, TIME, TIME)


@dataclass
class ReplayReader:
    pull_requests: dict[PromotionScope, PromotionPullRequest]
    events: dict[PromotionScope, tuple[PromotionEvent, ...]]
    comments: dict[PromotionScope, tuple[PromotionComment, ...]]
    refs: dict[tuple[RepositoryIdentity, str], CommitSha]
    ancestors: set[tuple[RepositoryIdentity, CommitSha, CommitSha]]
    histories: dict[PromotionScope, tuple[HeadForcePush, ...]] = field(
        default_factory=dict
    )
    edited: set[int] = field(default_factory=set)
    ref_reads: list[tuple[RepositoryIdentity, str]] = field(default_factory=list)

    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        return self.pull_requests[scope]

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]:
        return self.events.get(scope, ())

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]:
        return self.comments.get(scope, ())

    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None:
        for value in self.comments.get(scope, ()):
            if value.identifier == identifier:
                return (
                    None
                    if identifier in self.edited
                    else UneditedPromotionComment(value)
                )
        return None

    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]:
        return self.histories.get(scope, ())

    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        self.ref_reads.append((repository, ref))
        return self.refs.get((repository, ref))

    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        if base == head:
            return CommitComparison(
                repository, base, head, ComparisonStatus.IDENTICAL, base
            )
        if (repository, base, head) in self.ancestors:
            return CommitComparison(
                repository, base, head, ComparisonStatus.AHEAD, base
            )
        return CommitComparison(
            repository, base, head, ComparisonStatus.DIVERGED, CommitSha("0" * 40)
        )


def replay_reader(*, destination: str = "main") -> tuple[ReplayReader, QueuedPromotion]:
    child = pull_request(
        CHILD, base_ref="parent", base_sha=BASE, head_ref="child", head_sha=HEAD
    )
    open_parent = pull_request(SOURCE_PARENT)
    source_parent = replace(
        open_parent,
        details=replace(open_parent.details, state=PullRequestState.CLOSED),
    )
    upstream = pull_request(
        UPSTREAM_PARENT,
        base_ref=destination,
        base_sha=MAIN,
        state=PullRequestState.CLOSED,
        author=BOT,
        merge=PullRequestMerge(MERGE, TIME),
    )
    queued = QueuedPromotion.waiting_for_parent(child, APPROVAL, open_parent)
    return (
        ReplayReader(
            {CHILD: child, SOURCE_PARENT: source_parent, UPSTREAM_PARENT: upstream},
            {CHILD: (READY,)},
            {
                CHILD: (comment(CHILD, queued.comment(), 2000),),
                SOURCE_PARENT: (
                    comment(
                        SOURCE_PARENT,
                        "Promoted to [#100](https://github.com/astral-sh/uv/pull/100).",
                        1001,
                    ),
                ),
            },
            {
                (UV_DEV_REPOSITORY, "main"): MAIN,
                (UV_REPOSITORY, "main"): MAIN,
            },
            {(UV_DEV_REPOSITORY, MERGE, MAIN)},
        ),
        queued,
    )


class QueuedPromotionPlannerTests(unittest.TestCase):
    def assert_no_authority(self, plan: object) -> None:
        if not isinstance(plan, Stale):
            self.fail(f"Expected a stale replay, got {type(plan).__name__}")
        self.assertIsNone(plan.approval)

    def test_retained_or_recreated_parent_ref_does_not_change_destination(self) -> None:
        for retained in (BASE, OTHER):
            with self.subTest(retained=retained):
                reader, queued = replay_reader()
                reader.refs[(UV_REPOSITORY, "parent")] = retained
                plan = plan_queued_promotion(reader, REQUEST)
                if not isinstance(plan, Rebase):
                    self.fail("Expected the queued child to rebase onto main")
                self.assertEqual(plan.base, MAIN_REVISION)
                self.assertEqual(plan.previous_base, queued.base.sha)
                self.assertEqual(plan.parent.upstream.scope, UPSTREAM_PARENT)
                self.assertEqual(plan.approval.claim, queued.approval)
                self.assertNotIn((UV_REPOSITORY, "parent"), reader.ref_reads)

    def test_nested_parent_with_deleted_old_destination_uses_main(self) -> None:
        reader, queued = replay_reader(destination="grandparent")
        plan = plan_queued_promotion(reader, REQUEST)
        if not isinstance(plan, Rebase):
            self.fail("Expected the nested child to rebase onto synchronized main")
        self.assertEqual(plan.base, MAIN_REVISION)
        self.assertEqual(plan.previous_base, queued.base.sha)
        self.assertNotIn((UV_DEV_REPOSITORY, "grandparent"), reader.ref_reads)
        self.assertNotIn((UV_REPOSITORY, "grandparent"), reader.ref_reads)

    def test_only_public_explicit_readiness_requests_are_accepted(self) -> None:
        reader, _ = replay_reader()
        for request in (
            PromotionRequest(CHILD, HEAD),
            PromotionRequest(
                PromotionScope(UV_SECURITY_REPOSITORY, CHILD.number),
                HEAD,
                READY.identifier,
            ),
        ):
            with self.subTest(request=request), self.assertRaises(ValueError):
                plan_queued_promotion(reader, request)

    def test_missing_or_edited_queue_is_not_worker_authority(self) -> None:
        for edited in (False, True):
            with self.subTest(edited=edited):
                reader, _ = replay_reader()
                if edited:
                    reader.edited.add(2000)
                else:
                    reader.comments[CHILD] = ()
                self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_request_cannot_adopt_another_queued_head_or_event(self) -> None:
        reader, _ = replay_reader()
        for request in (
            replace(REQUEST, head=OTHER),
            replace(REQUEST, approval_id=READY.identifier + 1),
        ):
            with self.subTest(request=request):
                self.assert_no_authority(plan_queued_promotion(reader, request))

    def test_revocation_between_dispatch_and_worker_is_noop(self) -> None:
        reader, queued = replay_reader()
        self.assertIsInstance(
            plan_replay(reader, reader, queued, MAIN_REVISION), ReadyReplay
        )
        reader.events[CHILD] = (
            READY,
            ConvertedToDraftEvent(READY.identifier + 1, HUMAN, TIME),
            ReadyForReviewEvent(READY.identifier + 2, BOT, TIME),
        )
        self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_changed_source_revision_is_not_replayed(self) -> None:
        for change in ("head", "base", "draft", "closed"):
            with self.subTest(change=change):
                reader, _ = replay_reader()
                source = reader.pull_requests[CHILD]
                match change:
                    case "head":
                        source = replace(
                            source,
                            details=replace(
                                source.details,
                                head=replace(source.details.head, sha=OTHER),
                            ),
                        )
                    case "base":
                        source = replace(
                            source,
                            details=replace(
                                source.details,
                                base=replace(source.details.base, sha=OTHER),
                            ),
                        )
                    case "draft":
                        source = replace(source, draft=True)
                    case "closed":
                        source = replace(
                            source,
                            details=replace(
                                source.details, state=PullRequestState.CLOSED
                            ),
                        )
                reader.pull_requests[CHILD] = source
                self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_unrecorded_or_changed_source_parent_is_not_replayed(self) -> None:
        for change in ("record", "head", "open"):
            with self.subTest(change=change):
                reader, _ = replay_reader()
                parent = reader.pull_requests[SOURCE_PARENT]
                match change:
                    case "record":
                        reader.comments[SOURCE_PARENT] = ()
                    case "head":
                        reader.pull_requests[SOURCE_PARENT] = replace(
                            parent,
                            details=replace(
                                parent.details,
                                head=replace(parent.details.head, sha=OTHER),
                            ),
                        )
                    case "open":
                        reader.pull_requests[SOURCE_PARENT] = replace(
                            parent,
                            details=replace(
                                parent.details, state=PullRequestState.OPEN
                            ),
                        )
                self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_rewritten_parent_requires_its_original_head_in_history(self) -> None:
        reader, _ = replay_reader()
        upstream = reader.pull_requests[UPSTREAM_PARENT]
        reader.pull_requests[UPSTREAM_PARENT] = replace(
            upstream,
            details=replace(
                upstream.details, head=replace(upstream.details.head, sha=OTHER)
            ),
        )
        self.assert_no_authority(plan_queued_promotion(reader, REQUEST))
        reader.histories[UPSTREAM_PARENT] = (HeadForcePush("push", BASE, OTHER, TIME),)
        self.assertIsInstance(plan_queued_promotion(reader, REQUEST), Rebase)

    def test_main_must_be_available_synchronized_and_public(self) -> None:
        for change in ("missing", "unsynchronized", "unpublished"):
            with self.subTest(change=change):
                reader, _ = replay_reader()
                match change:
                    case "missing":
                        del reader.refs[(UV_DEV_REPOSITORY, "main")]
                    case "unsynchronized":
                        reader.ancestors.clear()
                    case "unpublished":
                        reader.refs[(UV_REPOSITORY, "main")] = OTHER
                self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_advancing_main_does_not_upgrade_the_pinned_destination(self) -> None:
        reader, _ = replay_reader()
        reader.refs[(UV_REPOSITORY, "main")] = OTHER
        reader.ancestors.update(
            {
                (UV_DEV_REPOSITORY, MAIN, OTHER),
                (UV_REPOSITORY, MAIN, OTHER),
            }
        )
        original_get_ref = reader.get_ref
        source_reads = 0

        def get_ref(repository: RepositoryIdentity, ref: str) -> CommitSha | None:
            nonlocal source_reads
            if repository == UV_DEV_REPOSITORY and ref == "main":
                source_reads += 1
                return MAIN if source_reads == 1 else OTHER
            return original_get_ref(repository, ref)

        with patch.object(reader, "get_ref", side_effect=get_ref):
            plan = plan_queued_promotion(reader, REQUEST)
        if not isinstance(plan, Rebase):
            self.fail("Expected a replay against the original public main revision")
        self.assertEqual(plan.base, MAIN_REVISION)

    def test_source_change_during_destination_verification_is_noop(self) -> None:
        reader, _ = replay_reader()
        original_compare = reader.compare_commits
        merge_comparisons = 0

        def compare(
            repository: RepositoryIdentity, base: CommitSha, head: CommitSha
        ) -> CommitComparison:
            nonlocal merge_comparisons
            result = original_compare(repository, base, head)
            if (repository, base, head) == (UV_DEV_REPOSITORY, MERGE, MAIN):
                merge_comparisons += 1
                if merge_comparisons == 2:
                    source = reader.pull_requests[CHILD]
                    reader.pull_requests[CHILD] = replace(
                        source,
                        details=replace(
                            source.details, head=replace(source.details.head, sha=OTHER)
                        ),
                    )
            return result

        with patch.object(reader, "compare_commits", side_effect=compare):
            self.assert_no_authority(plan_queued_promotion(reader, REQUEST))

    def test_destination_comparison_must_match_the_requested_identity(self) -> None:
        reader, _ = replay_reader()
        original_compare = reader.compare_commits
        merge_comparisons = 0

        def compare(
            repository: RepositoryIdentity, base: CommitSha, head: CommitSha
        ) -> CommitComparison:
            nonlocal merge_comparisons
            if (repository, base, head) == (UV_DEV_REPOSITORY, MERGE, MAIN):
                merge_comparisons += 1
                if merge_comparisons == 2:
                    return CommitComparison(
                        UV_REPOSITORY, base, head, ComparisonStatus.AHEAD, base
                    )
            return original_compare(repository, base, head)

        with (
            patch.object(reader, "compare_commits", side_effect=compare),
            self.assertRaisesRegex(ValueError, "compared different commits"),
        ):
            plan_queued_promotion(reader, REQUEST)


if __name__ == "__main__":
    unittest.main()
