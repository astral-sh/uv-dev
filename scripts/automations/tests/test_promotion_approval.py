import unittest
from dataclasses import dataclass, field, replace

from uv_automations.github_actions import WorkflowDispatch
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
    UV_DEV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    ConvertedToDraftEvent,
    GitHubAppIdentity,
    LabelAddedEvent,
    LabelRemovedEvent,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalKind,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    ReadyForReviewEvent,
    RepositoryPermission,
    UneditedPromotionComment,
)
from uv_automations.workflows.promotion_approval import (
    PRIVATE_PROMOTION_LABEL,
    TRUSTED_DISPATCHER,
    ApprovalUnavailableReason,
    InspectedPrivatePromotion,
    PrivateApprovalReference,
    PrivateApprovalUnavailable,
    PrivatePromotionRequest,
    RecordedPrivateApproval,
    RecordedPromotionReminder,
    SkippedPrivatePromotionRequest,
    dispatch_private_promotion,
    inspect_private_promotion,
    private_approval_receipt,
    private_approval_receipt_body,
    promotion_reminder_body,
    record_private_request,
    select_private_approval,
    verify_private_approval,
)

SOURCE = PromotionScope(UV_SECURITY_REPOSITORY, 17)
HEAD = CommitSha("a" * 40)
OTHER_HEAD = CommitSha("b" * 40)
BASE = CommitSha("c" * 40)
HUMAN = PromotionActor("reviewer", 123, ActorKind.USER)
OTHER_HUMAN = PromotionActor("other-reviewer", 456, ActorKind.USER)
READY_TIME = Timestamp.parse("2026-09-09T12:00:00Z")
LABEL_TIME = Timestamp.parse("2026-09-09T12:01:00Z")
LATER_TIME = Timestamp.parse("2026-09-09T12:02:00Z")
READY = ReadyForReviewEvent(10, HUMAN, READY_TIME)
LABEL = LabelAddedEvent(20, HUMAN, LABEL_TIME, PRIVATE_PROMOTION_LABEL)
DISPATCH = WorkflowDispatch(UV_SECURITY_REPOSITORY, 123456)


def pull_request() -> PromotionPullRequest:
    return PromotionPullRequest(
        SOURCE,
        PullRequestDetails(
            SOURCE.reference,
            PullRequestState.OPEN,
            f"https://github.com/{SOURCE.repository.name}/pull/{SOURCE.number}",
            PullRequestRevision(SOURCE.repository, "main", BASE),
            PullRequestRevision(SOURCE.repository, "private-fix", HEAD),
            (PRIVATE_PROMOTION_LABEL,),
        ),
        False,
        HUMAN,
        "A private fix",
        "Private body",
        None,
    )


def request(
    kind: PromotionApprovalKind = PromotionApprovalKind.LABELED,
) -> PrivatePromotionRequest:
    return PrivatePromotionRequest(
        SOURCE,
        HEAD,
        kind,
        TRUSTED_DISPATCHER,
        HUMAN,
        LABEL_TIME if kind == PromotionApprovalKind.LABELED else READY_TIME,
    )


def approval() -> PromotionApproval:
    return PromotionApproval(SOURCE, HEAD, LABEL, READY.identifier)


def comment(body: str, identifier: int = 1000) -> PromotionComment:
    return PromotionComment(
        SOURCE,
        identifier,
        TRUSTED_DISPATCHER,
        AUTOMATIONS_APP,
        body,
        LATER_TIME,
        LATER_TIME,
    )


@dataclass
class FakeGitHub:
    pull_request: PromotionPullRequest = field(default_factory=pull_request)
    events: tuple[PromotionEvent, ...] = (READY, LABEL)
    permission: RepositoryPermission = RepositoryPermission.WRITE
    comments: dict[int, PromotionComment] = field(default_factory=dict)
    edited: set[int] = field(default_factory=set)
    omit_app_from_create_response: bool = False
    permission_reads: list[tuple[RepositoryIdentity, PromotionActor]] = field(
        default_factory=list
    )
    receipt_writes: list[PromotionApproval] = field(default_factory=list)
    reminder_writes: list[int] = field(default_factory=list)
    dispatches: list[PrivateApprovalReference] = field(default_factory=list)
    dispatch_result: WorkflowDispatch = DISPATCH
    unedited_reads: list[int] = field(default_factory=list)

    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        if scope != self.pull_request.scope:
            raise AssertionError("Unexpected private pull request")
        return self.pull_request

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]:
        self.get_promotion_pull_request(scope)
        return self.events

    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission:
        self.permission_reads.append((repository, actor))
        return self.permission

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]:
        self.get_promotion_pull_request(scope)
        return tuple(self.comments.values())

    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None:
        self.get_promotion_pull_request(scope)
        self.unedited_reads.append(identifier)
        found = self.comments.get(identifier)
        if found is None or identifier in self.edited or not found.is_automation:
            return None
        return UneditedPromotionComment(found)

    def _add_comment(self, body: str) -> PromotionComment:
        identifier = max(self.comments, default=999) + 1
        result = comment(body, identifier)
        self.comments[identifier] = result
        return (
            replace(result, app=None) if self.omit_app_from_create_response else result
        )

    def create_approval_receipt(self, approval: PromotionApproval) -> PromotionComment:
        self.receipt_writes.append(approval)
        return self._add_comment(private_approval_receipt_body(approval))

    def create_promotion_reminder(
        self, scope: PromotionScope, ready_event_id: int
    ) -> PromotionComment:
        self.get_promotion_pull_request(scope)
        self.reminder_writes.append(ready_event_id)
        return self._add_comment(promotion_reminder_body(ready_event_id))

    def dispatch_private_promotion(
        self, reference: PrivateApprovalReference
    ) -> WorkflowDispatch:
        self.dispatches.append(reference)
        return self.dispatch_result


class PrivateApprovalTests(unittest.TestCase):
    def assert_unavailable(
        self, github: FakeGitHub, reason: ApprovalUnavailableReason
    ) -> None:
        with self.assertRaises(PrivateApprovalUnavailable) as raised:
            select_private_approval(github, SOURCE, HEAD)
        self.assertEqual(raised.exception.reason, reason)
        self.assertEqual(github.receipt_writes, [])
        self.assertEqual(github.dispatches, [])

    def test_selects_exact_current_writer_label(self) -> None:
        github = FakeGitHub(events=(LABEL, READY))
        self.assertEqual(select_private_approval(github, SOURCE, HEAD), approval())
        self.assertEqual(github.permission_reads, [(UV_SECURITY_REPOSITORY, HUMAN)])

    def test_inspection_reports_current_events_without_capturing_approval(self) -> None:
        github = FakeGitHub()
        self.assertEqual(
            inspect_private_promotion(
                github, SOURCE, HEAD, PromotionApprovalKind.LABELED
            ),
            InspectedPrivatePromotion(PromotionApprovalKind.LABELED, 20, 10),
        )
        self.assertEqual(
            inspect_private_promotion(
                github, SOURCE, HEAD, PromotionApprovalKind.READY_FOR_REVIEW
            ),
            InspectedPrivatePromotion(PromotionApprovalKind.READY_FOR_REVIEW, 10, 10),
        )
        github.permission = RepositoryPermission.READ
        self.assertEqual(
            inspect_private_promotion(
                github, SOURCE, HEAD, PromotionApprovalKind.LABELED
            ),
            SkippedPrivatePromotionRequest(
                ApprovalUnavailableReason.APPROVER_CANNOT_WRITE
            ),
        )
        self.assertEqual(github.comments, {})
        self.assertEqual(github.receipt_writes, [])
        self.assertEqual(github.reminder_writes, [])
        self.assertEqual(github.dispatches, [])

    def test_requires_exact_private_source_and_ready_head(self) -> None:
        original = pull_request()
        for changed, reason in [
            (replace(original, draft=True), ApprovalUnavailableReason.NOT_READY),
            (
                replace(
                    original,
                    details=replace(original.details, state=PullRequestState.CLOSED),
                ),
                ApprovalUnavailableReason.NOT_READY,
            ),
            (
                replace(
                    original,
                    details=replace(
                        original.details,
                        head=replace(
                            original.details.head, repository=UV_DEV_REPOSITORY
                        ),
                    ),
                ),
                ApprovalUnavailableReason.NOT_READY,
            ),
            (
                replace(
                    original,
                    details=replace(
                        original.details,
                        head=replace(original.details.head, sha=OTHER_HEAD),
                    ),
                ),
                ApprovalUnavailableReason.HEAD_CHANGED,
            ),
            (
                replace(original, details=replace(original.details, labels=())),
                ApprovalUnavailableReason.LABEL_MISSING,
            ),
        ]:
            with self.subTest(reason=reason, changed=changed):
                self.assert_unavailable(FakeGitHub(changed), reason)
        with self.assertRaises(ValueError):
            select_private_approval(
                FakeGitHub(), PromotionScope(UV_DEV_REPOSITORY, SOURCE.number), HEAD
            )

    def test_latest_readiness_and_current_label_transition_are_required(self) -> None:
        later_ready = ReadyForReviewEvent(30, TRUSTED_DISPATCHER, LATER_TIME)
        for events, reason in [
            ((), ApprovalUnavailableReason.NO_READINESS),
            ((READY,), ApprovalUnavailableReason.LABEL_NOT_CURRENT),
            ((LABEL, later_ready), ApprovalUnavailableReason.LABEL_PRECEDES_READINESS),
            (
                (
                    READY,
                    LABEL,
                    LabelRemovedEvent(21, HUMAN, LATER_TIME, PRIVATE_PROMOTION_LABEL),
                ),
                ApprovalUnavailableReason.LABEL_NOT_CURRENT,
            ),
            (
                (READY, LABEL, replace(LABEL, identifier=21, actor=TRUSTED_DISPATCHER)),
                ApprovalUnavailableReason.APPROVER_NOT_HUMAN,
            ),
            (
                (READY, replace(LABEL, actor=None)),
                ApprovalUnavailableReason.APPROVER_NOT_HUMAN,
            ),
        ]:
            with self.subTest(events=events):
                self.assert_unavailable(FakeGitHub(events=events), reason)

    def test_draft_event_after_the_pr_read_revokes_private_approval(self) -> None:
        github = FakeGitHub(
            events=(READY, LABEL, ConvertedToDraftEvent(21, HUMAN, LATER_TIME)),
            comments={1000: comment(private_approval_receipt_body(approval()))},
        )
        reference = PrivateApprovalReference(SOURCE, HEAD, 20, 10, 1000)
        # The first PR response is intentionally stale: the event history is
        # the later read that observes conversion back to draft.
        self.assertFalse(github.pull_request.draft)
        self.assert_unavailable(github, ApprovalUnavailableReason.NO_READINESS)
        for kind in PromotionApprovalKind:
            with self.subTest(kind=kind):
                skipped = SkippedPrivatePromotionRequest(
                    ApprovalUnavailableReason.NO_READINESS
                )
                self.assertEqual(
                    inspect_private_promotion(github, SOURCE, HEAD, kind), skipped
                )
                self.assertEqual(
                    record_private_request(github, github, request(kind)), skipped
                )
        with self.assertRaises(PrivateApprovalUnavailable) as raised:
            verify_private_approval(github, reference)
        self.assertEqual(
            raised.exception.reason, ApprovalUnavailableReason.NO_READINESS
        )
        with self.assertRaises(PrivateApprovalUnavailable):
            dispatch_private_promotion(github, github, reference)
        self.assertEqual(github.permission_reads, [])
        self.assertEqual(github.receipt_writes, [])
        self.assertEqual(github.reminder_writes, [])
        self.assertEqual(github.dispatches, [])

    def test_private_label_follows_current_readiness_from_any_actor(self) -> None:
        bot_ready = ReadyForReviewEvent(30, TRUSTED_DISPATCHER, LATER_TIME)
        fresh_label = replace(LABEL, identifier=40, created_at=LATER_TIME)
        github = FakeGitHub(
            events=(
                fresh_label,
                READY,
                ConvertedToDraftEvent(21, HUMAN, LATER_TIME),
                LabelRemovedEvent(22, HUMAN, LATER_TIME, PRIVATE_PROMOTION_LABEL),
                LABEL,
                bot_ready,
            )
        )
        expected = PromotionApproval(SOURCE, HEAD, fresh_label, bot_ready.identifier)
        self.assertEqual(select_private_approval(github, SOURCE, HEAD), expected)
        self.assertEqual(
            inspect_private_promotion(
                github, SOURCE, HEAD, PromotionApprovalKind.READY_FOR_REVIEW
            ),
            InspectedPrivatePromotion(PromotionApprovalKind.READY_FOR_REVIEW, 30, 30),
        )
        recorded = record_private_request(
            github, github, replace(request(), original_updated_at=LATER_TIME)
        )
        if not isinstance(recorded, RecordedPrivateApproval):
            self.fail("Expected the current label approval")
        self.assertTrue(recorded.receipt.claim.matches(expected))
        self.assertEqual(github.receipt_writes, [expected])
        self.assertEqual(github.dispatches, [])

    def test_current_permission_and_expected_event_ids(self) -> None:
        for permission in (RepositoryPermission.READ, RepositoryPermission.NONE):
            with self.subTest(permission=permission):
                self.assert_unavailable(
                    FakeGitHub(permission=permission),
                    ApprovalUnavailableReason.APPROVER_CANNOT_WRITE,
                )
        for approval_id, ready_id in [(21, 10), (20, 9)]:
            with self.subTest(approval_id=approval_id, ready_id=ready_id):
                with self.assertRaises(PrivateApprovalUnavailable) as raised:
                    select_private_approval(
                        FakeGitHub(),
                        SOURCE,
                        HEAD,
                        approval_id=approval_id,
                        ready_event_id=ready_id,
                    )
                self.assertEqual(
                    raised.exception.reason, ApprovalUnavailableReason.APPROVAL_CHANGED
                )

    def test_request_requires_the_exact_dispatcher(self) -> None:
        original = request()
        for changed in [
            {"dispatcher": replace(TRUSTED_DISPATCHER, database_id=1)},
            {"dispatcher": replace(TRUSTED_DISPATCHER, login="other-bot[bot]")},
            {"dispatcher": replace(TRUSTED_DISPATCHER, kind=ActorKind.USER)},
            {"sender": TRUSTED_DISPATCHER},
            {"source": PromotionScope(UV_DEV_REPOSITORY, SOURCE.number)},
        ]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                replace(original, **changed)

    def test_request_correlation_rejects_ambiguity_and_newer_applications(self) -> None:
        for changed_request, events in [
            (replace(request(), sender=OTHER_HUMAN), (READY, LABEL)),
            (replace(request(), original_updated_at=LATER_TIME), (READY, LABEL)),
            (request(), (READY, LABEL, replace(LABEL, identifier=21))),
            (
                request(),
                (READY, LABEL, replace(LABEL, identifier=22, created_at=LATER_TIME)),
            ),
        ]:
            with self.subTest(request=changed_request, events=events):
                github = FakeGitHub(events=events)
                self.assertEqual(
                    record_private_request(github, github, changed_request),
                    SkippedPrivatePromotionRequest(
                        ApprovalUnavailableReason.EVENT_NOT_CORRELATED
                    ),
                )
                self.assertEqual(github.receipt_writes, [])

    def test_request_records_and_reuses_one_receipt_without_dispatching(self) -> None:
        github = FakeGitHub()
        first = record_private_request(github, github, request())
        second = record_private_request(github, github, request())
        self.assertIsInstance(first, RecordedPrivateApproval)
        self.assertEqual(first, second)
        self.assertEqual(github.receipt_writes, [approval()])
        self.assertEqual(github.dispatches, [])

    def test_creation_uses_independent_issuer_proof(self) -> None:
        for kind in PromotionApprovalKind:
            with self.subTest(kind=kind):
                github = FakeGitHub(omit_app_from_create_response=True)
                result = record_private_request(github, github, request(kind))
                match kind:
                    case PromotionApprovalKind.READY_FOR_REVIEW:
                        self.assertEqual(result, RecordedPromotionReminder(10, 1000))
                    case PromotionApprovalKind.LABELED:
                        self.assertIsInstance(result, RecordedPrivateApproval)
                self.assertEqual(github.unedited_reads, [1000])
                self.assertEqual(github.dispatches, [])

        github = FakeGitHub(edited={1000})
        with self.assertRaisesRegex(ValueError, "unexpected private approval receipt"):
            record_private_request(github, github, request())
        self.assertEqual(github.dispatches, [])

    def test_old_label_cannot_be_recorded_for_another_head(self) -> None:
        github = FakeGitHub()
        github.create_approval_receipt(approval())
        github.pull_request = replace(
            github.pull_request,
            details=replace(
                github.pull_request.details,
                head=replace(github.pull_request.details.head, sha=OTHER_HEAD),
            ),
        )
        self.assertEqual(
            record_private_request(github, github, replace(request(), head=OTHER_HEAD)),
            SkippedPrivatePromotionRequest(ApprovalUnavailableReason.RECEIPT_CONFLICT),
        )
        self.assertEqual(github.receipt_writes, [approval()])

    def test_fresh_label_recovers_from_untrusted_historical_receipts(self) -> None:
        fresh_label = replace(LABEL, identifier=22, created_at=LATER_TIME)
        fresh_approval = PromotionApproval(
            SOURCE, OTHER_HEAD, fresh_label, READY.identifier
        )
        historical = private_approval_receipt_body(approval())
        current_looking = private_approval_receipt_body(
            PromotionApproval(SOURCE, HEAD, fresh_label, READY.identifier)
        )
        for body, edited in (
            (historical, False),
            (historical.replace('"version":1', '"version":'), False),
            (historical.replace('"version":1', '"version":'), True),
            (current_looking, True),
        ):
            with self.subTest(body=body, edited=edited):
                original = pull_request()
                github = FakeGitHub(
                    pull_request=replace(
                        original,
                        details=replace(
                            original.details,
                            head=replace(original.details.head, sha=OTHER_HEAD),
                        ),
                    ),
                    events=(
                        READY,
                        LABEL,
                        LabelRemovedEvent(
                            21, HUMAN, LATER_TIME, PRIVATE_PROMOTION_LABEL
                        ),
                        fresh_label,
                    ),
                    comments={1000: comment(body)},
                    edited={1000} if edited else set(),
                )
                recorded = record_private_request(
                    github,
                    github,
                    replace(request(), head=OTHER_HEAD, original_updated_at=LATER_TIME),
                )
                if not isinstance(recorded, RecordedPrivateApproval):
                    self.fail("Expected a fresh approval receipt")
                self.assertEqual(
                    recorded.receipt.reference,
                    PrivateApprovalReference(SOURCE, OTHER_HEAD, 22, 10, 1001),
                )
                self.assertEqual(github.receipt_writes, [fresh_approval])
                self.assertEqual(github.dispatches, [])

    def test_unedited_current_event_conflict_still_blocks_recording(self) -> None:
        fresh_label = replace(LABEL, identifier=22, created_at=LATER_TIME)
        conflicting = PromotionApproval(SOURCE, HEAD, fresh_label, READY.identifier)
        original = pull_request()
        github = FakeGitHub(
            pull_request=replace(
                original,
                details=replace(
                    original.details,
                    head=replace(original.details.head, sha=OTHER_HEAD),
                ),
            ),
            events=(READY, LABEL, fresh_label),
            comments={1000: comment(private_approval_receipt_body(conflicting))},
        )
        self.assertEqual(
            record_private_request(
                github,
                github,
                replace(request(), head=OTHER_HEAD, original_updated_at=LATER_TIME),
            ),
            SkippedPrivatePromotionRequest(ApprovalUnavailableReason.RECEIPT_CONFLICT),
        )
        self.assertEqual(github.unedited_reads, [1000])
        self.assertEqual(github.receipt_writes, [])
        self.assertEqual(github.dispatches, [])

    def test_receipt_requires_exact_app_and_canonical_body(self) -> None:
        original = comment(private_approval_receipt_body(approval()))
        receipt = private_approval_receipt(original)
        self.assertIsNotNone(receipt)
        if receipt is None:
            self.fail("Expected a decoded receipt")
        self.assertTrue(receipt.claim.matches(approval()))
        for changed in [
            replace(original, author=HUMAN),
            replace(original, author=replace(TRUSTED_DISPATCHER, database_id=1)),
            replace(original, app=None),
            replace(original, app=GitHubAppIdentity(1, AUTOMATIONS_APP.slug)),
            replace(
                original, app=GitHubAppIdentity(AUTOMATIONS_APP.database_id, "other")
            ),
        ]:
            with self.subTest(changed=changed):
                self.assertIsNone(private_approval_receipt(changed))
        for body in [
            "Quoted receipt:\n" + original.body,
            original.body + "\nextra",
            original.body.replace('"version":1', '"version":2'),
            original.body.replace('"version":1', '"version":1,"extra":true'),
            original.body.replace('"version":1', '"version":1,"version":1'),
        ]:
            with self.subTest(body=body):
                self.assertIsNone(
                    private_approval_receipt(replace(original, body=body))
                )

    def test_dispatch_verifies_the_exact_receipt_and_current_permission(self) -> None:
        github = FakeGitHub()
        recorded = record_private_request(github, github, request())
        if not isinstance(recorded, RecordedPrivateApproval):
            self.fail("Expected an approval receipt")
        reference = recorded.receipt.reference
        self.assertEqual(verify_private_approval(github, reference), approval())
        self.assertEqual(
            dispatch_private_promotion(github, github, reference),
            DISPATCH,
        )
        self.assertEqual(github.dispatches, [reference])
        github.permission = RepositoryPermission.READ
        with self.assertRaises(PrivateApprovalUnavailable):
            dispatch_private_promotion(github, github, reference)
        self.assertEqual(github.dispatches, [reference])

    def test_dispatch_rejects_a_run_from_another_repository(self) -> None:
        github = FakeGitHub(dispatch_result=WorkflowDispatch(UV_DEV_REPOSITORY, 123456))
        recorded = record_private_request(github, github, request())
        if not isinstance(recorded, RecordedPrivateApproval):
            self.fail("Expected an approval receipt")
        reference = recorded.receipt.reference
        with self.assertRaisesRegex(ValueError, "different private promotion dispatch"):
            dispatch_private_promotion(github, github, reference)
        self.assertEqual(github.dispatches, [reference])

    def test_same_second_edit_cannot_rebind_an_approval_receipt(self) -> None:
        github = FakeGitHub()
        recorded = record_private_request(github, github, request())
        if not isinstance(recorded, RecordedPrivateApproval):
            self.fail("Expected an approval receipt")
        reference = recorded.receipt.reference
        forged = replace(approval(), head=OTHER_HEAD)
        github.pull_request = replace(
            github.pull_request,
            details=replace(
                github.pull_request.details,
                head=replace(github.pull_request.details.head, sha=OTHER_HEAD),
            ),
        )
        github.comments[reference.receipt_id] = replace(
            github.comments[reference.receipt_id],
            body=private_approval_receipt_body(forged),
        )
        github.edited.add(reference.receipt_id)
        with self.assertRaises(ValueError):
            dispatch_private_promotion(
                github, github, replace(reference, head=OTHER_HEAD)
            )
        self.assertEqual(github.dispatches, [])

    def test_label_reapplication_invalidates_the_recorded_receipt(self) -> None:
        github = FakeGitHub()
        recorded = record_private_request(github, github, request())
        if not isinstance(recorded, RecordedPrivateApproval):
            self.fail("Expected an approval receipt")
        github.events = (
            READY,
            LABEL,
            replace(LABEL, identifier=22, created_at=LATER_TIME),
        )
        with self.assertRaises(PrivateApprovalUnavailable):
            dispatch_private_promotion(github, github, recorded.receipt.reference)
        self.assertEqual(github.dispatches, [])

    def test_readiness_only_posts_an_idempotent_reminder(self) -> None:
        github = FakeGitHub(events=(READY,))
        spoof = comment(promotion_reminder_body(READY.identifier), 999)
        github.comments[999] = replace(spoof, author=HUMAN, app=None)
        first = record_private_request(
            github, github, request(PromotionApprovalKind.READY_FOR_REVIEW)
        )
        second = record_private_request(
            github, github, request(PromotionApprovalKind.READY_FOR_REVIEW)
        )
        self.assertEqual(first, RecordedPromotionReminder(10, 1000))
        self.assertEqual(second, first)
        self.assertEqual(github.reminder_writes, [10])
        self.assertEqual(github.receipt_writes, [])
        self.assertEqual(github.dispatches, [])
