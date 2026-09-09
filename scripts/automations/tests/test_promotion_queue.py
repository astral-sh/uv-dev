import json
import unittest
from dataclasses import dataclass, field, replace

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.models import (
    ActorKind,
    CommitSha,
    PullRequestDetails,
    PullRequestRef,
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
    GitHubAppIdentity,
    HeadForcePush,
    MergedPromotedParent,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalClaim,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionRecord,
    PromotionScope,
    PullRequestMerge,
    PullRequestSelection,
    ReadyForReviewEvent,
    UneditedPromotionComment,
)
from uv_automations.workflows.promotion_queue import (
    QUEUE_MARKER,
    DispatchedReplay,
    PendingParentSync,
    QueuedPromotion,
    QueueRecordOutcome,
    ReplaySkipReason,
    SkippedReplay,
    current_queued_promotion,
    plan_replay,
    queued_promotion,
    record_queue,
    replay_one,
    replay_queued_promotions,
)

HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
MERGE = CommitSha("c" * 40)
MAIN = CommitSha("d" * 40)
OLD_MAIN = CommitSha("e" * 40)
OTHER = CommitSha("f" * 40)
TIME = Timestamp.parse("2026-09-09T12:00:00Z")
HUMAN = PromotionActor("zanieb", 101, ActorKind.USER)
BOT = PromotionActor("astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT)
CHILD_SCOPE = PromotionScope(UV_DEV_REPOSITORY, 20)
PARENT_SCOPE = PromotionScope(UV_DEV_REPOSITORY, 10)
UPSTREAM_SCOPE = PromotionScope(UV_REPOSITORY, 100)
READY = ReadyForReviewEvent(1000, HUMAN, TIME)
APPROVAL = PromotionApproval(CHILD_SCOPE, HEAD, READY, READY.identifier)


def pull_request(
    scope: PromotionScope,
    *,
    base_ref: str = "main",
    base_sha: CommitSha = OLD_MAIN,
    head_ref: str = "parent",
    head_sha: CommitSha = BASE,
    state: PullRequestState = PullRequestState.OPEN,
    draft: bool = False,
    author: PromotionActor = HUMAN,
    merge: PullRequestMerge | None = None,
) -> PromotionPullRequest:
    return PromotionPullRequest(
        scope,
        PullRequestDetails(
            PullRequestRef(scope.repository.name, scope.number),
            state,
            f"https://github.com/{scope.repository.name}/pull/{scope.number}",
            PullRequestRevision(scope.repository, base_ref, base_sha),
            PullRequestRevision(scope.repository, head_ref, head_sha),
            (),
        ),
        draft,
        author,
        "A pull request",
        "",
        merge,
    )


def child() -> PromotionPullRequest:
    return pull_request(
        CHILD_SCOPE, base_ref="parent", base_sha=BASE, head_ref="child", head_sha=HEAD
    )


def source_parent(*, open: bool = False) -> PromotionPullRequest:
    return pull_request(
        PARENT_SCOPE, state=PullRequestState.OPEN if open else PullRequestState.CLOSED
    )


def upstream_parent(
    *, head: CommitSha = BASE, merged: bool = True
) -> PromotionPullRequest:
    return pull_request(
        UPSTREAM_SCOPE,
        base_sha=MAIN,
        head_sha=head,
        state=PullRequestState.CLOSED if merged else PullRequestState.OPEN,
        author=BOT,
        merge=PullRequestMerge(MERGE, TIME) if merged else None,
    )


def comment(
    scope: PromotionScope, body: str, identifier: int = 2000
) -> PromotionComment:
    return PromotionComment(scope, identifier, BOT, AUTOMATIONS_APP, body, TIME, TIME)


def parent_record() -> PromotionComment:
    return comment(
        PARENT_SCOPE,
        "Promoted to [#100](https://github.com/astral-sh/uv/pull/100).",
        1001,
    )


def source_queue() -> QueuedPromotion:
    return QueuedPromotion.waiting_for_parent(
        child(), APPROVAL, source_parent(open=True)
    )


def sync_queue() -> QueuedPromotion:
    parent = source_parent()
    upstream = upstream_parent()
    proof = MergedPromotedParent(
        parent,
        upstream,
        PromotionRecord(PARENT_SCOPE, UPSTREAM_SCOPE, 1001),
        PullRequestMerge(MERGE, TIME),
    )
    return QueuedPromotion.waiting_for_sync(child(), APPROVAL, proof)


@dataclass
class FakeGitHub:
    pull_requests: dict[PromotionScope, PromotionPullRequest] = field(
        default_factory=lambda: {
            CHILD_SCOPE: child(),
            PARENT_SCOPE: source_parent(),
            UPSTREAM_SCOPE: upstream_parent(),
        }
    )
    events: dict[PromotionScope, tuple[PromotionEvent, ...]] = field(
        default_factory=lambda: {CHILD_SCOPE: (READY,)}
    )
    comments: dict[PromotionScope, tuple[PromotionComment, ...]] = field(
        default_factory=lambda: {PARENT_SCOPE: (parent_record(),)}
    )
    histories: dict[PromotionScope, tuple[HeadForcePush, ...]] = field(
        default_factory=dict
    )
    edited: set[int] = field(default_factory=set)
    main: CommitSha = MAIN
    upstream_main: CommitSha = MAIN
    ancestors: set[tuple[CommitSha, CommitSha]] = field(
        default_factory=lambda: {(MERGE, MAIN)}
    )
    created: list[QueuedPromotion] = field(default_factory=list)
    dispatched: list[PromotionApprovalClaim] = field(default_factory=list)

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
        raise AssertionError("Unexpected comment")

    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]:
        return self.histories.get(scope, ())

    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        if ref != "main":
            raise AssertionError("Unexpected ref")
        if repository == UV_DEV_REPOSITORY:
            return self.main
        if repository == UV_REPOSITORY:
            return self.upstream_main
        raise AssertionError("Unexpected repository")

    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        if base == head:
            return CommitComparison(
                repository, base, head, ComparisonStatus.IDENTICAL, base
            )
        if (base, head) in self.ancestors:
            return CommitComparison(
                repository, base, head, ComparisonStatus.AHEAD, base
            )
        return CommitComparison(
            repository, base, head, ComparisonStatus.DIVERGED, CommitSha("0" * 40)
        )

    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]:
        return tuple(
            value
            for value in self.pull_requests.values()
            if (
                value.scope.repository == repository
                and (
                    state == PullRequestSelection.ALL
                    or value.is_open == (state == PullRequestSelection.OPEN)
                )
                and (head is None or value.details.head.ref == head)
                and (base is None or value.details.base.ref == base)
            )
        )

    def create_queue_comment(self, queued: QueuedPromotion) -> None:
        self.created.append(queued)
        existing = self.comments.get(queued.source, ())
        self.comments[queued.source] = (
            *existing,
            comment(queued.source, queued.comment(), 3000 + len(self.created)),
        )

    def dispatch_promotion(self, approval: PromotionApprovalClaim) -> WorkflowDispatch:
        self.dispatched.append(approval)
        return WorkflowDispatch(UV_DEV_REPOSITORY, 4000 + len(self.dispatched))

    def add_queue(self, queued: QueuedPromotion, identifier: int = 2000) -> None:
        self.comments[queued.source] = (
            *self.comments.get(queued.source, ()),
            comment(queued.source, queued.comment(), identifier),
        )


MAIN_REVISION = BranchRevision(UV_DEV_REPOSITORY, "main", MAIN)


class QueueSerializationTests(unittest.TestCase):
    def test_both_waiting_states_round_trip(self) -> None:
        for queued in (source_queue(), sync_queue()):
            with self.subTest(parent=queued.parent):
                self.assertEqual(QueuedPromotion.from_json(queued.to_json()), queued)
                self.assertEqual(
                    queued_promotion(comment(CHILD_SCOPE, queued.comment())), queued
                )
                self.assertIn('"event_id":1000', queued.comment())

    def test_only_the_complete_automation_record_is_accepted(self) -> None:
        queued = source_queue()
        original = comment(CHILD_SCOPE, queued.comment())
        invalid = (
            replace(original, author=HUMAN),
            replace(original, app=GitHubAppIdentity(1, AUTOMATIONS_APP.slug)),
            replace(original, body="Quoted:\n\n" + original.body),
            replace(original, body=original.body + "\n\n<!-- another-workflow -->"),
            replace(
                original, body=original.body.replace('"event_id":1000', '"event_id":0')
            ),
            replace(original, scope=PARENT_SCOPE),
        )
        for value in invalid:
            with self.subTest(value=value):
                self.assertIsNone(queued_promotion(value))

    def test_queue_decoder_rejects_extra_or_ambiguous_fields(self) -> None:
        value = source_queue().to_json()
        with self.assertRaises(ValueError):
            QueuedPromotion.from_json(value | {"version": 2})
        encoded = json.dumps(value).replace(
            '"event_id": 1000', '"event_id": 1000, "event_id": 1001'
        )
        body = (
            source_queue().comment().split(QUEUE_MARKER)[0]
            + QUEUE_MARKER
            + encoded
            + " -->"
        )
        self.assertIsNone(queued_promotion(comment(CHILD_SCOPE, body)))

    def test_private_approval_is_not_public_replay_authority(self) -> None:
        queued = source_queue()
        private = PromotionScope(UV_SECURITY_REPOSITORY, CHILD_SCOPE.number)
        with self.assertRaises(ValueError):
            replace(queued, approval=replace(queued.approval, source=private))


class PromotionQueueTests(unittest.TestCase):
    def test_recording_is_idempotent(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        self.assertEqual(
            record_queue(github, github, queued), QueueRecordOutcome.RECORDED
        )
        self.assertEqual(
            record_queue(github, github, queued), QueueRecordOutcome.UNCHANGED
        )
        self.assertEqual(github.created, [queued])

    def test_recording_rechecks_the_exact_approval(self) -> None:
        for changed_head, changed_event in ((True, False), (False, True)):
            with self.subTest(changed_head=changed_head):
                github = FakeGitHub()
                if changed_head:
                    original = github.pull_requests[CHILD_SCOPE]
                    github.pull_requests[CHILD_SCOPE] = replace(
                        original,
                        details=replace(
                            original.details,
                            head=replace(original.details.head, sha=OTHER),
                        ),
                    )
                if changed_event:
                    github.events[CHILD_SCOPE] = (
                        READY,
                        replace(READY, identifier=1002),
                    )
                self.assertEqual(
                    record_queue(github, github, source_queue()),
                    QueueRecordOutcome.STALE,
                )
                self.assertFalse(github.created)

    def test_one_approval_cannot_be_rebound_to_a_changed_base(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        github.add_queue(queued)
        source = github.pull_requests[CHILD_SCOPE]
        github.pull_requests[CHILD_SCOPE] = replace(
            source,
            details=replace(
                source.details, base=replace(source.details.base, sha=OTHER)
            ),
        )
        rebound = replace(queued, base=replace(queued.base, sha=OTHER))
        self.assertEqual(
            record_queue(github, github, rebound), QueueRecordOutcome.STALE
        )
        self.assertFalse(github.created)

    def test_edited_queue_record_cannot_authorize_replay(self) -> None:
        github = FakeGitHub()
        github.add_queue(source_queue())
        github.edited.add(2000)
        self.assertEqual(
            current_queued_promotion(github, child()),
            SkippedReplay(ReplaySkipReason.NO_RECORD),
        )

    def test_conflicting_bindings_for_one_event_fail_closed(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        github.add_queue(queued)
        github.add_queue(
            replace(
                queued,
                approval=replace(queued.approval, head=OTHER),
                head=replace(queued.head, sha=OTHER),
            ),
            2001,
        )
        self.assertEqual(
            current_queued_promotion(github, child()),
            SkippedReplay(ReplaySkipReason.AMBIGUOUS_RECORD),
        )

    def test_withdrawn_approval_is_not_revived_by_a_bot(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        github.add_queue(queued)
        github.events[CHILD_SCOPE] = (
            READY,
            ConvertedToDraftEvent(1001, HUMAN, TIME),
            ReadyForReviewEvent(1002, BOT, TIME),
        )
        self.assertEqual(record_queue(github, github, queued), QueueRecordOutcome.STALE)
        self.assertEqual(
            current_queued_promotion(github, child()),
            SkippedReplay(ReplaySkipReason.STALE_APPROVAL),
        )
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.STALE_APPROVAL),
        )
        self.assertFalse(github.created)
        self.assertFalse(github.dispatched)

    def test_new_human_approval_supersedes_an_old_conflict(self) -> None:
        github = FakeGitHub()
        old = source_queue()
        github.add_queue(old)
        github.add_queue(
            replace(
                old,
                approval=replace(old.approval, head=OTHER),
                head=replace(old.head, sha=OTHER),
            ),
            2001,
        )
        event = ReadyForReviewEvent(1003, HUMAN, TIME)
        github.events[CHILD_SCOPE] = (
            READY,
            ConvertedToDraftEvent(1001, HUMAN, TIME),
            ReadyForReviewEvent(1002, BOT, TIME),
            event,
        )
        approval = PromotionApproval(CHILD_SCOPE, HEAD, event, event.identifier)
        queued = replace(old, approval=approval.claim)
        self.assertEqual(
            record_queue(github, github, queued), QueueRecordOutcome.RECORDED
        )
        self.assertEqual(current_queued_promotion(github, child()), queued)
        self.assertIsInstance(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            DispatchedReplay,
        )
        self.assertEqual(github.dispatched, [approval.claim])


class PromotionReplayTests(unittest.TestCase):
    def test_wait_parent_merge_sync_then_dispatch_exact_original(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        github.add_queue(queued)
        github.pull_requests[PARENT_SCOPE] = source_parent(open=True)
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.PARENT_CHANGED),
        )
        github.pull_requests[PARENT_SCOPE] = source_parent()
        github.ancestors.clear()
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.PARENT_NOT_SYNCED),
        )
        github.ancestors.add((MERGE, MAIN))
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            DispatchedReplay(queued, WorkflowDispatch(UV_DEV_REPOSITORY, 4001)),
        )
        self.assertEqual(github.dispatched, [APPROVAL.claim])

    def test_record_after_sync_still_gets_its_own_replay(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        self.assertEqual(
            record_queue(github, github, queued), QueueRecordOutcome.RECORDED
        )
        outcome = replay_one(
            github, github, github, CHILD_SCOPE, MAIN_REVISION, expected=queued
        )
        self.assertIsInstance(outcome, DispatchedReplay)
        self.assertEqual(github.dispatched, [APPROVAL.claim])

    def test_immediate_retry_accepts_a_stronger_existing_sync_record(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        current = sync_queue()
        github.add_queue(current)
        self.assertEqual(
            record_queue(github, github, queued), QueueRecordOutcome.UNCHANGED
        )
        self.assertEqual(
            replay_one(
                github, github, github, CHILD_SCOPE, MAIN_REVISION, expected=queued
            ),
            DispatchedReplay(current, WorkflowDispatch(UV_DEV_REPOSITORY, 4001)),
        )
        changed = replace(queued, base=replace(queued.base, sha=OTHER))
        self.assertEqual(
            replay_one(
                github, github, github, CHILD_SCOPE, MAIN_REVISION, expected=changed
            ),
            SkippedReplay(ReplaySkipReason.STALE_APPROVAL),
        )

    def test_parent_receipt_finishing_after_sync_wakes_its_children(self) -> None:
        github = FakeGitHub()
        github.add_queue(source_queue())
        github.pull_requests[PARENT_SCOPE] = source_parent(open=True)
        github.comments[PARENT_SCOPE] = ()
        self.assertEqual(
            replay_queued_promotions(github, github, github, MAIN_REVISION),
            (SkippedReplay(ReplaySkipReason.PARENT_CHANGED),),
        )
        github.pull_requests[PARENT_SCOPE] = source_parent()
        github.comments[PARENT_SCOPE] = (parent_record(),)
        outcomes = replay_queued_promotions(
            github, github, github, MAIN_REVISION, parent=PARENT_SCOPE
        )
        self.assertEqual(len(outcomes), 1)
        self.assertIsInstance(outcomes[0], DispatchedReplay)
        self.assertEqual(github.dispatched, [APPROVAL.claim])

    def test_parent_wakeup_does_not_replay_a_reused_branch_identity(self) -> None:
        github = FakeGitHub()
        github.add_queue(source_queue())
        other_parent = PromotionScope(UV_DEV_REPOSITORY, 11)
        github.pull_requests[other_parent] = pull_request(
            other_parent, state=PullRequestState.CLOSED
        )
        self.assertEqual(
            replay_queued_promotions(
                github, github, github, MAIN_REVISION, parent=other_parent
            ),
            (SkippedReplay(ReplaySkipReason.PARENT_CHANGED),),
        )
        self.assertFalse(github.dispatched)

    def test_rebased_public_parent_needs_verified_history(self) -> None:
        github = FakeGitHub()
        queued = source_queue()
        github.add_queue(queued)
        github.pull_requests[UPSTREAM_SCOPE] = upstream_parent(head=OTHER)
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.PARENT_NOT_PROMOTED),
        )
        github.histories[UPSTREAM_SCOPE] = (HeadForcePush("event", BASE, OTHER, TIME),)
        self.assertIsInstance(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            DispatchedReplay,
        )

    def test_wait_for_sync_keeps_exact_upstream_merge(self) -> None:
        github = FakeGitHub()
        queued = sync_queue()
        self.assertIsInstance(queued.parent, PendingParentSync)
        github.add_queue(queued)
        github.pull_requests[UPSTREAM_SCOPE] = replace(
            upstream_parent(), merge=PullRequestMerge(OTHER, TIME)
        )
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.PARENT_CHANGED),
        )
        self.assertFalse(github.dispatched)

    def test_changed_approval_or_source_is_not_replayed(self) -> None:
        for change in ("head", "base", "draft", "closed", "approval"):
            with self.subTest(change=change):
                github = FakeGitHub()
                github.add_queue(source_queue())
                original = github.pull_requests[CHILD_SCOPE]
                if change == "head":
                    original = replace(
                        original,
                        details=replace(
                            original.details,
                            head=replace(original.details.head, sha=OTHER),
                        ),
                    )
                elif change == "base":
                    original = replace(
                        original,
                        details=replace(
                            original.details,
                            base=replace(original.details.base, ref="main", sha=MAIN),
                        ),
                    )
                elif change == "draft":
                    original = replace(original, draft=True)
                elif change == "closed":
                    original = replace(
                        original,
                        details=replace(
                            original.details, state=PullRequestState.CLOSED
                        ),
                    )
                elif change == "approval":
                    github.events[CHILD_SCOPE] = (
                        READY,
                        replace(READY, identifier=1002),
                    )
                github.pull_requests[CHILD_SCOPE] = original
                self.assertIsInstance(
                    replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
                    SkippedReplay,
                )
                self.assertFalse(github.dispatched)

    def test_unrecorded_or_non_bot_parent_is_not_replay_authority(self) -> None:
        for change in ("record", "author", "merged"):
            with self.subTest(change=change):
                github = FakeGitHub()
                github.add_queue(source_queue())
                if change == "record":
                    github.comments[PARENT_SCOPE] = ()
                elif change == "author":
                    github.pull_requests[UPSTREAM_SCOPE] = replace(
                        upstream_parent(), author=HUMAN
                    )
                else:
                    github.pull_requests[UPSTREAM_SCOPE] = upstream_parent(merged=False)
                self.assertIsInstance(
                    replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
                    SkippedReplay,
                )
                self.assertFalse(github.dispatched)

    def test_pinned_sync_can_advance_but_not_roll_back(self) -> None:
        github = FakeGitHub(main=OTHER)
        github.add_queue(source_queue())
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.MAIN_CHANGED),
        )
        github.ancestors.add((MAIN, OTHER))
        github.upstream_main = OTHER
        self.assertIsInstance(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            DispatchedReplay,
        )

    def test_source_only_main_is_not_a_public_replay_target(self) -> None:
        github = FakeGitHub(main=OTHER)
        github.add_queue(source_queue())
        github.ancestors.add((MAIN, OTHER))
        self.assertEqual(
            replay_one(github, github, github, CHILD_SCOPE, MAIN_REVISION),
            SkippedReplay(ReplaySkipReason.MAIN_NOT_PUBLIC),
        )
        self.assertFalse(github.dispatched)

    def test_repository_scan_has_no_blanket_ready_pr_dispatch(self) -> None:
        github = FakeGitHub()
        self.assertEqual(
            replay_queued_promotions(github, github, github, MAIN_REVISION),
            (SkippedReplay(ReplaySkipReason.NO_RECORD),),
        )
        self.assertFalse(github.dispatched)
        github.add_queue(source_queue())
        outcomes = replay_queued_promotions(github, github, github, MAIN_REVISION)
        self.assertEqual(len(outcomes), 1)
        self.assertIsInstance(outcomes[0], DispatchedReplay)

    def test_replay_requires_the_source_main_identity(self) -> None:
        github = FakeGitHub()
        for main in (
            BranchRevision(UV_REPOSITORY, "main", MAIN),
            BranchRevision(UV_DEV_REPOSITORY, "branch", MAIN),
        ):
            with self.subTest(main=main), self.assertRaises(ValueError):
                plan_replay(github, github, source_queue(), main)


if __name__ == "__main__":
    unittest.main()
