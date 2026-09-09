import json
import subprocess
import unittest
from dataclasses import dataclass, field, replace
from unittest.mock import patch

from uv_automations.github_promotion import (
    PromotionGitHub,
    PromotionReadError,
    decode_promotion_comment,
    decode_promotion_pull_request,
    parent_sync,
    verified_promoted_parent,
)
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
    MAX_PROMOTION_PAGE_SIZE,
    MAX_PROMOTION_PAGES,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    AmbiguousPromotionRecord,
    BranchRevision,
    ClosedPromotedParent,
    CommitComparison,
    ComparisonStatus,
    ConvertedToDraftEvent,
    GitHubAppIdentity,
    HeadForcePush,
    LabelAddedEvent,
    LabelRemovedEvent,
    MergedPromotedParent,
    OpenPromotedParent,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalClaim,
    PromotionApprovalKind,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    PullRequestMerge,
    PullRequestSelection,
    ReadyForReviewEvent,
    RepositoryPermission,
    SynchronizedParent,
    UneditedPromotionComment,
    UnrecordedMergedParent,
    UnsynchronizedParent,
    current_ready_approval,
    current_ready_event,
    latest_label_event,
    latest_ready_event,
    promotion_record,
    ready_approval,
    unique_promotion_record,
)
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

BASE = CommitSha("a" * 40)
HEAD = CommitSha("b" * 40)
MERGE = CommitSha("c" * 40)
MAIN = CommitSha("d" * 40)
UPDATED = CommitSha("e" * 40)
NOW = Timestamp.parse("2026-09-09T12:00:00Z")
LATER = Timestamp.parse("2026-09-09T12:01:00Z")
HUMAN = PromotionActor("maintainer", 1234, ActorKind.USER)
BOT = PromotionActor("astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT)
SOURCE = PromotionScope(UV_DEV_REPOSITORY, 101)
SOURCE_PARENT = PromotionScope(UV_DEV_REPOSITORY, 100)
UPSTREAM = PromotionScope(UV_REPOSITORY, 201)
UPSTREAM_PARENT = PromotionScope(UV_REPOSITORY, 200)
READY = ReadyForReviewEvent(30_000_000_001, HUMAN, NOW)
MERGED = PullRequestMerge(MERGE, NOW)


def pull_request(
    scope: PromotionScope = SOURCE,
    *,
    state: PullRequestState = PullRequestState.OPEN,
    draft: bool = False,
    base_ref: str = "main",
    base_sha: CommitSha = BASE,
    head_ref: str = "feature",
    head_sha: CommitSha = HEAD,
    head_repository: RepositoryIdentity | None = None,
    author: PromotionActor | None = HUMAN,
    merge: PullRequestMerge | None = None,
    labels: tuple[str, ...] = (),
) -> PromotionPullRequest:
    return PromotionPullRequest(
        scope,
        PullRequestDetails(
            scope.reference,
            state,
            f"https://github.com/{scope.repository.name}/pull/{scope.number}",
            PullRequestRevision(scope.repository, base_ref, base_sha),
            PullRequestRevision(
                head_repository or scope.repository, head_ref, head_sha
            ),
            labels,
        ),
        draft,
        author,
        "Title",
        "Body",
        merge,
    )


def repository_payload(repository: RepositoryIdentity | None) -> object:
    if repository is None:
        return None
    return {"full_name": str(repository.name), "id": repository.database_id}


def graphql_repository(repository: RepositoryIdentity) -> dict[str, object]:
    return {
        "nameWithOwner": str(repository.name),
        "databaseId": repository.database_id,
    }


def actor_payload(actor: PromotionActor | None) -> object:
    if actor is None:
        return None
    return {"login": actor.login, "id": actor.database_id, "type": actor.kind.value}


def pull_request_payload(pull: PromotionPullRequest | None = None) -> dict[str, object]:
    pull = pull or pull_request()
    return {
        "number": pull.scope.number,
        "state": pull.details.state.value,
        "html_url": pull.details.url,
        "base": {
            "repo": repository_payload(pull.details.base.repository),
            "ref": pull.details.base.ref,
            "sha": str(pull.details.base.sha),
        },
        "head": {
            "repo": repository_payload(pull.details.head.repository),
            "ref": pull.details.head.ref,
            "sha": str(pull.details.head.sha),
        },
        "labels": [{"name": name} for name in pull.details.labels],
        "draft": pull.draft,
        "user": actor_payload(pull.author),
        "title": pull.title,
        "body": pull.body,
        "merged_at": str(pull.merge.merged_at) if pull.merge is not None else None,
        "merge_commit_sha": str(pull.merge.sha) if pull.merge is not None else None,
    }


def comment(
    scope: PromotionScope = SOURCE_PARENT,
    *,
    identifier: int = 5_000_000_001,
    upstream: PromotionScope = UPSTREAM_PARENT,
    author: PromotionActor | None = BOT,
    app: GitHubAppIdentity | None = AUTOMATIONS_APP,
    body: str | None = None,
) -> PromotionComment:
    if body is None:
        body = (
            f"Promoted to [#{upstream.number}]"
            f"(https://github.com/{upstream.repository.name}/pull/{upstream.number})."
        )
    return PromotionComment(scope, identifier, author, app, body, NOW, NOW)


def comment_payload(value: PromotionComment) -> dict[str, object]:
    return {
        "id": value.identifier,
        "node_id": "IC_example",
        "issue_url": (
            f"https://api.github.com/repos/{value.scope.repository.name}"
            f"/issues/{value.scope.number}"
        ),
        "user": actor_payload(value.author),
        "performed_via_github_app": (
            {"id": value.app.database_id, "slug": value.app.slug}
            if value.app is not None
            else None
        ),
        "body": value.body,
        "created_at": str(value.created_at),
        "updated_at": str(value.updated_at),
    }


def event_payload(
    identifier: int,
    kind: str,
    *,
    actor: PromotionActor | None = HUMAN,
    label: str = "bot:promote",
) -> dict[str, object]:
    return {
        "id": identifier,
        "event": kind,
        "actor": actor_payload(actor),
        "created_at": str(NOW),
        "label": {"name": label},
    }


def graphql_pull_request(scope: PromotionScope) -> dict[str, object]:
    return {
        "number": scope.number,
        "repository": graphql_repository(scope.repository),
    }


def force_push_payload(
    identifier: str,
    *,
    scope: PromotionScope = UPSTREAM_PARENT,
    before: CommitSha | None = BASE,
    after: CommitSha | None = UPDATED,
) -> dict[str, object]:
    return {
        "__typename": "HeadRefForcePushedEvent",
        "id": identifier,
        "createdAt": str(NOW),
        "beforeCommit": {"oid": str(before)} if before is not None else None,
        "afterCommit": {"oid": str(after)} if after is not None else None,
        "pullRequest": graphql_pull_request(scope),
    }


def history_page(
    events: list[object],
    *,
    scope: PromotionScope = UPSTREAM_PARENT,
    has_next: bool = False,
    cursor: str | None = None,
) -> dict[str, object]:
    return {
        "repository": {
            **graphql_repository(scope.repository),
            "pullRequest": {
                **graphql_pull_request(scope),
                "timelineItems": {
                    "nodes": events,
                    "pageInfo": {"hasNextPage": has_next, "endCursor": cursor},
                },
            },
        }
    }


def unedited_comment_payload(value: PromotionComment) -> dict[str, object]:
    return {
        "__typename": "IssueComment",
        "id": "IC_example",
        "fullDatabaseId": str(value.identifier),
        "body": value.body,
        "lastEditedAt": None,
        "editor": None,
        "repository": graphql_repository(value.scope.repository),
        "pullRequest": graphql_pull_request(value.scope),
    }


@dataclass
class FakePromotionReader:
    pull_requests: dict[PromotionScope, PromotionPullRequest]
    events: dict[PromotionScope, tuple[PromotionEvent, ...]] = field(
        default_factory=dict
    )
    comments: dict[PromotionScope, tuple[PromotionComment, ...]] = field(
        default_factory=dict
    )
    force_pushes: dict[PromotionScope, tuple[HeadForcePush, ...]] = field(
        default_factory=dict
    )
    refs: dict[tuple[RepositoryIdentity, str], CommitSha] = field(default_factory=dict)
    comparisons: dict[
        tuple[RepositoryIdentity, CommitSha, CommitSha], CommitComparison
    ] = field(default_factory=dict)
    permission: RepositoryPermission = RepositoryPermission.WRITE
    calls: list[str] = field(default_factory=list)

    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        self.calls.append("pull-request")
        return self.pull_requests[scope]

    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]:
        self.calls.append("pull-requests")
        return tuple(
            pull
            for pull in self.pull_requests.values()
            if pull.scope.repository == repository
            and (state == PullRequestSelection.ALL or pull.details.state.value == state)
            and (head is None or pull.details.head.ref == head)
            and (base is None or pull.details.base.ref == base)
        )

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]:
        self.calls.append("events")
        return self.events.get(scope, ())

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]:
        self.calls.append("comments")
        return self.comments.get(scope, ())

    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]:
        self.calls.append("force-pushes")
        return self.force_pushes.get(scope, ())

    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        self.calls.append("ref")
        return self.refs.get((repository, ref))

    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        self.calls.append("compare")
        return self.comparisons[(repository, base, head)]

    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission:
        self.calls.append("permission")
        return self.permission


def planner_reader(source: PromotionPullRequest | None = None) -> FakePromotionReader:
    source = source or pull_request()
    return FakePromotionReader(
        {source.scope: source},
        events={source.scope: (READY,)},
        refs={(UV_REPOSITORY, "main"): BASE},
    )


def recorded_parent_reader(
    *,
    upstream_state: PullRequestState = PullRequestState.CLOSED,
    upstream_head: CommitSha = UPDATED,
    merge: PullRequestMerge | None = MERGED,
) -> tuple[FakePromotionReader, PromotionPullRequest]:
    source = pull_request(
        SOURCE_PARENT,
        state=PullRequestState.CLOSED,
        head_ref="parent",
        head_sha=BASE,
    )
    upstream = pull_request(
        UPSTREAM_PARENT,
        state=upstream_state,
        head_ref="parent",
        head_sha=upstream_head,
        author=BOT,
        merge=merge,
    )
    return (
        FakePromotionReader(
            {SOURCE_PARENT: source, UPSTREAM_PARENT: upstream},
            comments={SOURCE_PARENT: (comment(),)},
            force_pushes={
                UPSTREAM_PARENT: (HeadForcePush("event-1", BASE, UPDATED, NOW),)
            },
        ),
        source,
    )


class PromotionModelTests(unittest.TestCase):
    def test_managed_repository_and_full_sha_identity(self) -> None:
        self.assertEqual(PromotionScope.from_json(SOURCE.to_json()), SOURCE)
        revision = BranchRevision(UV_DEV_REPOSITORY, "feature/nested", HEAD)
        self.assertEqual(BranchRevision.from_json(revision.to_json()), revision)
        for value in (
            {**SOURCE.to_json(), "repository_id": 1},
            {**SOURCE.to_json(), "pull_request": True},
            {**SOURCE.to_json(), "unexpected": 1},
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                PromotionScope.from_json(value)
        with self.assertRaises(ValueError):
            BranchRevision.from_json({**revision.to_json(), "sha": "a" * 12})
        with self.assertRaisesRegex(ValueError, "Invalid promotion branch"):
            BranchRevision(UV_DEV_REPOSITORY, "bad\nbranch", HEAD)

    def test_approval_claim_is_strict_and_not_a_reconstructed_event(self) -> None:
        approval = PromotionApproval(SOURCE, HEAD, READY, READY.identifier)
        claim = PromotionApprovalClaim.from_json(approval.to_json())
        self.assertEqual(claim, approval.claim)
        self.assertTrue(claim.matches(approval))
        self.assertFalse(replace(claim, actor_id=99).matches(approval))
        for value in (
            {**claim.to_json(), "event_id": True},
            {**claim.to_json(), "ready_event_id": READY.identifier - 1},
            {**claim.to_json(), "kind": "opened"},
            {**claim.to_json(), "extra": 1},
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                PromotionApprovalClaim.from_json(value)

    def test_approval_selectors_keep_label_removal_and_all_readiness_events(
        self,
    ) -> None:
        bot_ready = ReadyForReviewEvent(READY.identifier + 1, BOT, LATER)
        added = LabelAddedEvent(READY.identifier + 2, HUMAN, LATER, "bot:promote")
        removed = LabelRemovedEvent(READY.identifier + 3, HUMAN, LATER, "bot:promote")
        events = (removed, added, bot_ready, READY)
        self.assertEqual(latest_ready_event(events), bot_ready)
        self.assertEqual(latest_ready_event(events, human_only=True), READY)
        self.assertEqual(latest_label_event(events, "bot:promote"), removed)
        approval = ready_approval(SOURCE, HEAD, events)
        if approval is None:
            self.fail("Expected a human readiness approval")
        self.assertEqual(approval.event_id, READY.identifier)
        self.assertIsNone(ready_approval(SOURCE, HEAD, (bot_ready,)))
        with self.assertRaisesRegex(ValueError, "human actor"):
            PromotionApproval(SOURCE, HEAD, bot_ready, bot_ready.identifier)
        with self.assertRaisesRegex(ValueError, "follow"):
            PromotionApproval(SOURCE, HEAD, added, added.identifier)

    def test_current_readiness_requires_the_latest_transition(self) -> None:
        draft = ConvertedToDraftEvent(READY.identifier + 1, HUMAN, LATER)
        events = (draft, READY)
        self.assertIsNone(current_ready_event(events))
        self.assertIsNone(current_ready_approval(SOURCE, HEAD, events))
        self.assertEqual(
            ready_approval(SOURCE, HEAD, events),
            PromotionApproval(SOURCE, HEAD, READY, READY.identifier),
        )

        for actor in (BOT, None):
            with self.subTest(actor=actor):
                ready = ReadyForReviewEvent(READY.identifier + 2, actor, LATER)
                history = (ready, READY, draft)
                self.assertEqual(current_ready_event(history), ready)
                self.assertIsNone(current_ready_event(history, human_only=True))
                self.assertIsNone(current_ready_approval(SOURCE, HEAD, history))
                self.assertEqual(
                    ready_approval(SOURCE, HEAD, history),
                    PromotionApproval(SOURCE, HEAD, READY, READY.identifier),
                )

        ready = ReadyForReviewEvent(READY.identifier + 3, HUMAN, LATER)
        history = (ready, draft, READY)
        self.assertEqual(current_ready_event(history, human_only=True), ready)
        self.assertEqual(
            current_ready_approval(SOURCE, HEAD, history),
            PromotionApproval(SOURCE, HEAD, ready, ready.identifier),
        )

    def test_legacy_record_requires_exact_bot_app_and_body(self) -> None:
        expected = promotion_record(comment())
        self.assertIsNotNone(expected)
        for candidate in (
            comment(author=replace(BOT, database_id=1)),
            comment(author=replace(BOT, kind=ActorKind.USER)),
            comment(app=replace(AUTOMATIONS_APP, database_id=1)),
            comment(app=replace(AUTOMATIONS_APP, slug="other-app")),
            comment(app=None),
            comment(
                body="Promoted to [#200](https://github.com/astral-sh/uv/pull/201)."
            ),
            comment(
                body="Promoted to [#0200](https://github.com/astral-sh/uv/pull/0200)."
            ),
            comment(body=comment().body + "\n"),
        ):
            with self.subTest(candidate=candidate):
                self.assertIsNone(promotion_record(candidate))
        duplicate = comment(identifier=5_000_000_002)
        self.assertEqual(
            unique_promotion_record(SOURCE_PARENT, (duplicate, comment())), expected
        )
        with self.assertRaises(AmbiguousPromotionRecord):
            unique_promotion_record(
                SOURCE_PARENT, (comment(), comment(identifier=6, upstream=UPSTREAM))
            )
        with self.assertRaisesRegex(ValueError, "inconsistent identities"):
            unique_promotion_record(SOURCE_PARENT, (comment(), comment()))

    def test_comparison_states_are_exhaustive_and_consistent(self) -> None:
        comparisons = (
            CommitComparison(UV_REPOSITORY, BASE, HEAD, ComparisonStatus.AHEAD, BASE),
            CommitComparison(
                UV_REPOSITORY, BASE, BASE, ComparisonStatus.IDENTICAL, BASE
            ),
            CommitComparison(UV_REPOSITORY, HEAD, BASE, ComparisonStatus.BEHIND, BASE),
            CommitComparison(
                UV_REPOSITORY, HEAD, UPDATED, ComparisonStatus.DIVERGED, BASE
            ),
        )
        self.assertEqual(
            tuple(item.is_ancestor for item in comparisons), (True, True, False, False)
        )
        with self.assertRaisesRegex(ValueError, "Inconsistent"):
            replace(comparisons[0], merge_base=HEAD)
        with self.assertRaisesRegex(ValueError, "Inconsistent"):
            replace(comparisons[1], head=HEAD)


class PromotionGitHubTests(unittest.TestCase):
    def test_pull_request_decode_checks_both_repository_name_and_id(self) -> None:
        self.assertEqual(
            decode_promotion_pull_request(pull_request_payload(), SOURCE),
            pull_request(),
        )
        foreign = replace(UV_DEV_REPOSITORY, database_id=1)
        for payload in (
            {**pull_request_payload(), "number": 999},
            {
                **pull_request_payload(),
                "base": {
                    "repo": repository_payload(foreign),
                    "ref": "main",
                    "sha": str(BASE),
                },
            },
            {
                **pull_request_payload(),
                "merged_at": str(NOW),
                "merge_commit_sha": str(MERGE),
            },
        ):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                decode_promotion_pull_request(payload, SOURCE)

    def test_transport_captures_and_redacts_errors(self) -> None:
        private = "private-branch-body-and-sha"
        with patch("uv_automations.github_promotion.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 1, private, private)
            with self.assertRaises(PromotionReadError) as caught:
                PromotionGitHub().get_promotion_pull_request(SOURCE)
        self.assertEqual(str(caught.exception), "GitHub promotion request failed")
        self.assertTrue(run.call_args.kwargs["capture_output"])
        self.assertFalse(run.call_args.kwargs["check"])
        with patch("uv_automations.github_promotion.subprocess.run") as run:
            run.side_effect = subprocess.TimeoutExpired(
                ["gh", private], 60, output=private
            )
            with self.assertRaises(PromotionReadError) as caught:
                PromotionGitHub().get_promotion_pull_request(SOURCE)
        self.assertNotIn(private, str(caught.exception))
        with (
            patch.object(PromotionGitHub, "_api", return_value={"number": private}),
            self.assertRaisesRegex(
                PromotionReadError, "Invalid GitHub promotion response"
            ),
        ):
            PromotionGitHub().get_promotion_pull_request(SOURCE)

    def test_list_query_is_bounded_and_rechecks_empty_repository(self) -> None:
        with patch.object(
            PromotionGitHub,
            "_api",
            side_effect=[repository_payload(UV_DEV_REPOSITORY), []],
        ) as api:
            result = PromotionGitHub().list_pull_requests(
                UV_DEV_REPOSITORY,
                state=PullRequestSelection.CLOSED,
                head="feature/name",
            )
        self.assertEqual(result, ())
        self.assertEqual(api.call_args_list[0].args, ("GET", "repos/astral-sh/uv-dev"))
        self.assertIn("head=astral-sh%3Afeature%2Fname", api.call_args_list[1].args[1])
        self.assertIn("per_page=100&page=1", api.call_args_list[1].args[1])
        with (
            patch.object(
                PromotionGitHub, "_api", return_value=repository_payload(UV_REPOSITORY)
            ),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().list_pull_requests(UV_DEV_REPOSITORY)

    def test_issue_history_decodes_64_bit_ids_and_rejects_duplicates(self) -> None:
        batch = [
            event_payload(READY.identifier + 3, "unlabeled"),
            event_payload(READY.identifier + 2, "labeled"),
            event_payload(READY.identifier + 1, "convert_to_draft", actor=None),
            event_payload(READY.identifier, "ready_for_review"),
            event_payload(1, "referenced"),
        ]
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(PromotionGitHub, "_pages", return_value=tuple(batch)),
        ):
            events = PromotionGitHub().list_promotion_events(SOURCE)
        self.assertEqual(events[0], READY)
        self.assertIsInstance(events[1], ConvertedToDraftEvent)
        self.assertIsInstance(events[2], LabelAddedEvent)
        self.assertIsInstance(events[3], LabelRemovedEvent)
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(PromotionGitHub, "_pages", return_value=(batch[0], batch[0])),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().list_promotion_events(SOURCE)

    def test_history_budget_and_oversized_pages_fail_closed(self) -> None:
        for size, calls in (
            (MAX_PROMOTION_PAGE_SIZE, MAX_PROMOTION_PAGES),
            (MAX_PROMOTION_PAGE_SIZE + 1, 1),
        ):
            with (
                self.subTest(size=size),
                patch.object(
                    PromotionGitHub,
                    "get_promotion_pull_request",
                    return_value=pull_request(),
                ),
                patch.object(PromotionGitHub, "_api", return_value=[{}] * size) as api,
                self.assertRaises(PromotionReadError),
            ):
                PromotionGitHub().list_promotion_events(SOURCE)
            self.assertEqual(api.call_count, calls)

    def test_exact_comment_read_checks_issue_and_returned_id(self) -> None:
        expected = comment(scope=SOURCE)
        self.assertEqual(
            decode_promotion_comment(comment_payload(expected), SOURCE), expected
        )
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(
                PromotionGitHub, "_api", return_value=comment_payload(expected)
            ) as api,
        ):
            result = PromotionGitHub().get_promotion_comment(
                SOURCE, expected.identifier
            )
        self.assertEqual(result, expected)
        self.assertEqual(
            api.call_args.args[1],
            f"repos/astral-sh/uv-dev/issues/comments/{expected.identifier}",
        )
        for payload in (
            {**comment_payload(expected), "id": expected.identifier + 1},
            {
                **comment_payload(expected),
                "issue_url": "https://api.github.com/repos/astral-sh/uv-dev/issues/999",
            },
        ):
            with (
                self.subTest(payload=payload),
                patch.object(
                    PromotionGitHub,
                    "get_promotion_pull_request",
                    return_value=pull_request(),
                ),
                patch.object(PromotionGitHub, "_api", return_value=payload),
                self.assertRaises(PromotionReadError),
            ):
                PromotionGitHub().get_promotion_comment(SOURCE, expected.identifier)

    def test_unedited_comment_proof_binds_rest_graphql_and_bigint_identity(
        self,
    ) -> None:
        expected = comment(scope=SOURCE, body="immutable receipt")
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(
                PromotionGitHub, "_api", return_value=comment_payload(expected)
            ),
            patch.object(
                PromotionGitHub, "list_promotion_comments", return_value=(expected,)
            ),
            patch.object(
                PromotionGitHub,
                "_graphql",
                return_value={"node": unedited_comment_payload(expected)},
            ) as graphql,
        ):
            result = PromotionGitHub().get_unedited_promotion_comment(
                SOURCE, expected.identifier
            )
        self.assertEqual(result, UneditedPromotionComment(expected))
        self.assertEqual(graphql.call_args.kwargs, {"id": "IC_example"})
        self.assertIn("fullDatabaseId", graphql.call_args.args[0])
        self.assertIn("lastEditedAt", graphql.call_args.args[0])

    def test_exact_comment_app_omission_uses_the_bound_list_record(self) -> None:
        expected = comment(scope=SOURCE, body="immutable receipt")
        direct = {**comment_payload(expected), "performed_via_github_app": None}
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(PromotionGitHub, "_api", return_value=direct),
            patch.object(
                PromotionGitHub, "list_promotion_comments", return_value=(expected,)
            ),
            patch.object(
                PromotionGitHub,
                "_graphql",
                return_value={"node": unedited_comment_payload(expected)},
            ),
        ):
            result = PromotionGitHub().get_unedited_promotion_comment(
                SOURCE, expected.identifier
            )
        self.assertEqual(result, UneditedPromotionComment(expected))

    def test_exact_comment_must_agree_with_the_app_owned_list_record(self) -> None:
        expected = comment(scope=SOURCE, body="immutable receipt")
        for direct in (
            {**comment_payload(expected), "body": "different body"},
            {**comment_payload(expected), "user": actor_payload(HUMAN)},
            {**comment_payload(expected), "updated_at": str(LATER)},
            {
                **comment_payload(expected),
                "performed_via_github_app": {"id": 1, "slug": AUTOMATIONS_APP.slug},
            },
        ):
            with (
                self.subTest(direct=direct),
                patch.object(
                    PromotionGitHub,
                    "get_promotion_pull_request",
                    return_value=pull_request(),
                ),
                patch.object(PromotionGitHub, "_api", return_value=direct),
                patch.object(
                    PromotionGitHub,
                    "list_promotion_comments",
                    return_value=(expected,),
                ),
                patch.object(PromotionGitHub, "_graphql") as graphql,
            ):
                self.assertIsNone(
                    PromotionGitHub().get_unedited_promotion_comment(
                        SOURCE, expected.identifier
                    )
                )
            graphql.assert_not_called()

    def test_same_second_edit_and_body_races_are_not_authoritative(self) -> None:
        expected = comment(scope=SOURCE, body="immutable receipt")
        original = unedited_comment_payload(expected)
        for node in (
            {**original, "lastEditedAt": str(NOW)},
            {**original, "editor": {"__typename": "User"}},
            {**original, "body": "changed receipt"},
            None,
        ):
            with (
                self.subTest(node=node),
                patch.object(
                    PromotionGitHub,
                    "get_promotion_pull_request",
                    return_value=pull_request(),
                ),
                patch.object(
                    PromotionGitHub, "_api", return_value=comment_payload(expected)
                ),
                patch.object(
                    PromotionGitHub,
                    "list_promotion_comments",
                    return_value=(expected,),
                ),
                patch.object(PromotionGitHub, "_graphql", return_value={"node": node}),
            ):
                self.assertIsNone(
                    PromotionGitHub().get_unedited_promotion_comment(
                        SOURCE, expected.identifier
                    )
                )

    def test_unedited_comment_rejects_wrong_owner_or_database_identity(self) -> None:
        expected = comment(scope=SOURCE, body="immutable receipt")
        original = unedited_comment_payload(expected)
        for node in (
            {**original, "__typename": "PullRequestReviewComment"},
            {**original, "fullDatabaseId": str(expected.identifier + 1)},
            {**original, "fullDatabaseId": expected.identifier},
            {**original, "repository": graphql_repository(UV_REPOSITORY)},
            {**original, "pullRequest": graphql_pull_request(SOURCE_PARENT)},
        ):
            with (
                self.subTest(node=node),
                patch.object(
                    PromotionGitHub,
                    "get_promotion_pull_request",
                    return_value=pull_request(),
                ),
                patch.object(
                    PromotionGitHub, "_api", return_value=comment_payload(expected)
                ),
                patch.object(
                    PromotionGitHub,
                    "list_promotion_comments",
                    return_value=(expected,),
                ),
                patch.object(PromotionGitHub, "_graphql", return_value={"node": node}),
                self.assertRaises(PromotionReadError),
            ):
                PromotionGitHub().get_unedited_promotion_comment(
                    SOURCE, expected.identifier
                )

    def test_unedited_comment_does_not_accept_an_unrelated_app(self) -> None:
        counterfeit = comment(scope=SOURCE, app=replace(AUTOMATIONS_APP, database_id=1))
        with (
            patch.object(
                PromotionGitHub,
                "get_promotion_pull_request",
                return_value=pull_request(),
            ),
            patch.object(
                PromotionGitHub, "_api", return_value=comment_payload(counterfeit)
            ),
            patch.object(
                PromotionGitHub,
                "list_promotion_comments",
                return_value=(counterfeit,),
            ),
            patch.object(PromotionGitHub, "_graphql") as graphql,
        ):
            self.assertIsNone(
                PromotionGitHub().get_unedited_promotion_comment(
                    SOURCE, counterfeit.identifier
                )
            )
        graphql.assert_not_called()

    def test_ref_read_uses_exact_qualified_name_and_repository_id(self) -> None:
        response = {
            "repository": {
                **graphql_repository(UV_DEV_REPOSITORY),
                "ref": {
                    "name": "feature/name",
                    "prefix": "refs/heads/",
                    "target": {"__typename": "Commit", "oid": str(HEAD)},
                },
            }
        }
        with patch.object(
            PromotionGitHub, "_graphql", return_value=response
        ) as graphql:
            self.assertEqual(
                PromotionGitHub().get_ref(UV_DEV_REPOSITORY, "feature/name"), HEAD
            )
        self.assertEqual(graphql.call_args.kwargs["ref"], "refs/heads/feature/name")
        with patch.object(
            PromotionGitHub,
            "_graphql",
            return_value={
                "repository": {**graphql_repository(UV_DEV_REPOSITORY), "ref": None}
            },
        ):
            self.assertIsNone(PromotionGitHub().get_ref(UV_DEV_REPOSITORY, "missing"))
        with (
            patch.object(
                PromotionGitHub,
                "_graphql",
                return_value={
                    "repository": {**graphql_repository(UV_REPOSITORY), "ref": None}
                },
            ),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().get_ref(UV_DEV_REPOSITORY, "missing")

    def test_comparison_uses_full_shas_and_checks_reported_merge_base(self) -> None:
        response = {
            "base_commit": {"sha": str(BASE)},
            "merge_base_commit": {"sha": str(BASE)},
            "status": "ahead",
        }
        with patch.object(
            PromotionGitHub,
            "_api",
            side_effect=[repository_payload(UV_REPOSITORY), response],
        ) as api:
            result = PromotionGitHub().compare_commits(UV_REPOSITORY, BASE, HEAD)
        self.assertTrue(result.is_ancestor)
        self.assertEqual(
            api.call_args.args[1],
            f"repos/astral-sh/uv/compare/{BASE}...{HEAD}?per_page=1&page=1",
        )
        with (
            patch.object(
                PromotionGitHub,
                "_api",
                side_effect=[
                    repository_payload(UV_REPOSITORY),
                    {**response, "merge_base_commit": {"sha": str(HEAD)}},
                ],
            ),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().compare_commits(UV_REPOSITORY, BASE, HEAD)

    def test_permission_read_rechecks_actor_database_id(self) -> None:
        permission = {"permission": "write", "user": actor_payload(HUMAN)}
        with patch.object(
            PromotionGitHub,
            "_api",
            side_effect=[repository_payload(UV_SECURITY_REPOSITORY), permission],
        ):
            self.assertTrue(
                PromotionGitHub()
                .get_repository_permission(UV_SECURITY_REPOSITORY, HUMAN)
                .can_write
            )
        with (
            patch.object(
                PromotionGitHub,
                "_api",
                side_effect=[
                    repository_payload(UV_SECURITY_REPOSITORY),
                    {
                        **permission,
                        "user": actor_payload(replace(HUMAN, database_id=2)),
                    },
                ],
            ),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().get_repository_permission(UV_SECURITY_REPOSITORY, HUMAN)

    def test_force_push_history_is_complete_and_owns_each_event(self) -> None:
        pages = [
            history_page([force_push_payload("event-1")], has_next=True, cursor="next"),
            history_page([force_push_payload("event-2", before=None, after=HEAD)]),
        ]
        with patch.object(PromotionGitHub, "_graphql", side_effect=pages) as graphql:
            result = PromotionGitHub().list_head_force_pushes(UPSTREAM_PARENT)
        self.assertEqual(
            tuple(event.identifier for event in result), ("event-1", "event-2")
        )
        self.assertTrue(result[0].contains(BASE))
        self.assertIsNone(result[1].before)
        self.assertEqual(graphql.call_args_list[1].kwargs["cursor"], "next")
        wrong = history_page([force_push_payload("event-1", scope=UPSTREAM)])
        with (
            patch.object(PromotionGitHub, "_graphql", return_value=wrong),
            self.assertRaises(PromotionReadError),
        ):
            PromotionGitHub().list_head_force_pushes(UPSTREAM_PARENT)

    def test_force_push_history_rejects_cursor_cycles_duplicates_and_truncation(
        self,
    ) -> None:
        sequences = (
            [
                history_page(
                    [force_push_payload("event-1")], has_next=True, cursor="same"
                ),
                history_page(
                    [force_push_payload("event-2")], has_next=True, cursor="same"
                ),
            ],
            [
                history_page(
                    [force_push_payload("event-1")], has_next=True, cursor="next"
                ),
                history_page([force_push_payload("event-1")]),
            ],
            [
                history_page([], has_next=True, cursor=f"cursor-{index}")
                for index in range(MAX_PROMOTION_PAGES)
            ],
            [
                history_page(
                    [force_push_payload("event-1")] * (MAX_PROMOTION_PAGE_SIZE + 1)
                )
            ],
        )
        for pages in sequences:
            with (
                self.subTest(pages=len(pages)),
                patch.object(PromotionGitHub, "_graphql", side_effect=pages),
                self.assertRaises(PromotionReadError),
            ):
                PromotionGitHub().list_head_force_pushes(UPSTREAM_PARENT)


class PromotedParentTests(unittest.TestCase):
    def test_force_push_provenance_retains_exact_original_head(self) -> None:
        reader, source = recorded_parent_reader()
        result = verified_promoted_parent(reader, source)
        if not isinstance(result, MergedPromotedParent):
            self.fail("Expected a verified merged parent")
        self.assertEqual(result.original_head, BASE)
        self.assertEqual(result.merge, MERGED)
        self.assertEqual(result.record.source, SOURCE_PARENT)
        reader.force_pushes[UPSTREAM_PARENT] = (
            HeadForcePush("unrelated", HEAD, UPDATED, NOW),
        )
        self.assertIsNone(verified_promoted_parent(reader, source))

    def test_equal_head_open_closed_and_merged_states_are_explicit(self) -> None:
        for state, merge, expected in (
            (PullRequestState.OPEN, None, OpenPromotedParent),
            (PullRequestState.CLOSED, None, ClosedPromotedParent),
            (PullRequestState.CLOSED, MERGED, MergedPromotedParent),
        ):
            with self.subTest(state=state, merge=merge):
                reader, source = recorded_parent_reader(
                    upstream_state=state, upstream_head=BASE, merge=merge
                )
                result = verified_promoted_parent(reader, source)
                self.assertIsInstance(result, expected)
                self.assertNotIn("force-pushes", reader.calls)

    def test_counterfeit_or_ambiguous_parent_evidence_is_not_a_parent(self) -> None:
        reader, source = recorded_parent_reader()
        original = reader.pull_requests[UPSTREAM_PARENT]
        for upstream in (
            replace(original, author=HUMAN),
            replace(
                original,
                details=replace(
                    original.details, head=replace(original.details.head, ref="other")
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details,
                    head=replace(original.details.head, repository=UV_DEV_REPOSITORY),
                ),
            ),
        ):
            with self.subTest(upstream=upstream):
                reader.pull_requests[UPSTREAM_PARENT] = upstream
                self.assertIsNone(verified_promoted_parent(reader, source))
        reader.pull_requests[UPSTREAM_PARENT] = original
        reader.comments[SOURCE_PARENT] = (
            comment(),
            comment(identifier=2, upstream=UPSTREAM),
        )
        self.assertIsNone(verified_promoted_parent(reader, source))
        reader.comments[SOURCE_PARENT] = (comment(app=None),)
        self.assertIsNone(verified_promoted_parent(reader, source))

    def test_separate_upstream_reader_receives_only_upstream_proof_reads(self) -> None:
        upstream, source = recorded_parent_reader()
        source_reader = FakePromotionReader(
            {SOURCE_PARENT: source}, comments=upstream.comments
        )
        result = verified_promoted_parent(
            source_reader, source, upstream_reader=upstream
        )
        self.assertIsInstance(result, MergedPromotedParent)
        self.assertEqual(source_reader.calls, ["comments"])
        self.assertEqual(upstream.calls, ["pull-request", "force-pushes"])

    def test_synchronized_main_is_checked_at_its_exact_sha(self) -> None:
        reader, source = recorded_parent_reader()
        parent = verified_promoted_parent(reader, source)
        if not isinstance(parent, MergedPromotedParent):
            self.fail("Expected a verified merged parent")
        main = BranchRevision(UV_DEV_REPOSITORY, "main", MAIN)
        reader.comparisons[(UV_DEV_REPOSITORY, MERGE, MAIN)] = CommitComparison(
            UV_DEV_REPOSITORY, MERGE, MAIN, ComparisonStatus.AHEAD, MERGE
        )
        self.assertIsInstance(parent_sync(reader, parent, main), SynchronizedParent)
        reader.comparisons[(UV_DEV_REPOSITORY, MERGE, MAIN)] = CommitComparison(
            UV_DEV_REPOSITORY, MERGE, MAIN, ComparisonStatus.BEHIND, MAIN
        )
        self.assertIsInstance(parent_sync(reader, parent, main), UnsynchronizedParent)
        with self.assertRaisesRegex(ValueError, "source main"):
            parent_sync(reader, parent, replace(main, ref="other"))


class PromotionPlanTests(unittest.TestCase):
    def test_existing_upstream_base_produces_read_only_publish_plan(self) -> None:
        reader = planner_reader()
        result = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
        if not isinstance(result, Publish):
            self.fail("Expected a publication plan")
        self.assertEqual(result.base, BranchRevision(UV_REPOSITORY, "main", BASE))
        self.assertIsNone(result.copy_base)

    def test_existing_matching_upstream_is_explicit_but_mismatches_reject(self) -> None:
        reader = planner_reader()
        reader.pull_requests[UPSTREAM] = pull_request(UPSTREAM, author=BOT)
        result = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
        if not isinstance(result, AlreadyPublished):
            self.fail("Expected an existing publication")
        self.assertEqual(result.upstream.scope, UPSTREAM)
        reader.pull_requests[UPSTREAM] = pull_request(
            UPSTREAM, head_sha=UPDATED, author=BOT
        )
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(SOURCE, HEAD)), Rejected
        )

    def test_wait_for_open_source_parent_keeps_its_exact_identity(self) -> None:
        source = pull_request(base_ref="parent")
        reader = planner_reader(source)
        parent = pull_request(SOURCE_PARENT, head_ref="parent", head_sha=BASE)
        reader.pull_requests[SOURCE_PARENT] = parent
        result = plan_promotion(
            reader, PromotionRequest(SOURCE, HEAD, READY.identifier)
        )
        if not isinstance(result, WaitForParent):
            self.fail("Expected a wait for the open source parent")
        self.assertEqual(result.parent, parent)
        self.assertEqual(result.approval.event_id, READY.identifier)
        reader.pull_requests[PromotionScope(UV_DEV_REPOSITORY, 99)] = replace(
            parent,
            scope=PromotionScope(UV_DEV_REPOSITORY, 99),
            details=replace(
                parent.details,
                reference=PromotionScope(UV_DEV_REPOSITORY, 99).reference,
            ),
        )
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(SOURCE, HEAD)), Rejected
        )

    def test_open_cross_repository_parent_produces_exact_copy_claim(self) -> None:
        source = pull_request(base_ref="parent")
        reader = planner_reader(source)
        parent = pull_request(
            UPSTREAM_PARENT,
            head_ref="parent",
            head_sha=BASE,
            head_repository=UV_DEV_REPOSITORY,
        )
        reader.pull_requests[UPSTREAM_PARENT] = parent
        result = plan_promotion(
            reader, PromotionRequest(SOURCE, HEAD, READY.identifier)
        )
        if not isinstance(result, Publish):
            self.fail("Expected a publication plan")
        copy_base = result.copy_base
        if copy_base is None:
            self.fail("Expected an upstream-base copy precondition")
        claim = CopyUpstreamBaseClaim.from_json(copy_base.to_json())
        self.assertTrue(claim.matches(copy_base))
        self.assertEqual(claim.parent, UPSTREAM_PARENT)
        self.assertEqual(
            claim.destination, BranchRevision(UV_REPOSITORY, "parent", BASE)
        )
        self.assertNotIn("Title", json.dumps(claim.to_json()))
        with self.assertRaises(ValueError):
            CopyUpstreamBaseClaim.from_json({**claim.to_json(), "version": True})
        with self.assertRaises(ValueError):
            CopyUpstreamBaseClaim.from_json({**claim.to_json(), "extra": 1})

    def _merged_reader(self, *, recorded: bool, synced: bool) -> FakePromotionReader:
        source = pull_request(base_ref="parent")
        reader = planner_reader(source)
        if recorded:
            evidence, _ = recorded_parent_reader()
            reader.pull_requests.update(evidence.pull_requests)
            reader.comments.update(evidence.comments)
            reader.force_pushes.update(evidence.force_pushes)
        else:
            reader.pull_requests[UPSTREAM_PARENT] = pull_request(
                UPSTREAM_PARENT,
                state=PullRequestState.CLOSED,
                head_ref="parent",
                head_sha=BASE,
                author=HUMAN,
                merge=MERGED,
            )
        reader.refs[(UV_DEV_REPOSITORY, "main")] = MAIN
        reader.comparisons[(UV_DEV_REPOSITORY, MERGE, MAIN)] = CommitComparison(
            UV_DEV_REPOSITORY,
            MERGE,
            MAIN,
            ComparisonStatus.AHEAD if synced else ComparisonStatus.BEHIND,
            MERGE if synced else MAIN,
        )
        return reader

    def test_recorded_merged_parent_rebases_only_after_sync(self) -> None:
        reader = self._merged_reader(recorded=True, synced=False)
        waiting = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
        if not isinstance(waiting, WaitForSync):
            self.fail("Expected a wait for the parent merge to synchronize")
        parent = waiting.parent
        if not isinstance(parent, MergedPromotedParent):
            self.fail("Expected a recorded merged parent")
        self.assertEqual(parent.source.scope, SOURCE_PARENT)
        self.assertEqual(parent.upstream.scope, UPSTREAM_PARENT)
        reader.comparisons[(UV_DEV_REPOSITORY, MERGE, MAIN)] = CommitComparison(
            UV_DEV_REPOSITORY, MERGE, MAIN, ComparisonStatus.AHEAD, MERGE
        )
        result = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
        if not isinstance(result, Rebase):
            self.fail("Expected a parent-update rebase plan")
        self.assertEqual(result.base, BranchRevision(UV_DEV_REPOSITORY, "main", MAIN))
        self.assertEqual(result.previous_base, BASE)

    def test_manual_exact_head_parent_remains_distinct_from_replay_authority(
        self,
    ) -> None:
        waiting = plan_promotion(
            self._merged_reader(recorded=False, synced=False),
            PromotionRequest(SOURCE, HEAD),
        )
        if not isinstance(waiting, WaitForSync):
            self.fail("Expected a wait for the parent merge to synchronize")
        self.assertIsInstance(waiting.parent, UnrecordedMergedParent)
        result = plan_promotion(
            self._merged_reader(recorded=False, synced=True),
            PromotionRequest(SOURCE, HEAD),
        )
        if not isinstance(result, Rebase):
            self.fail("Expected a parent-update rebase plan")
        self.assertIsInstance(result.parent, UnrecordedMergedParent)

    def test_rewritten_parent_without_complete_history_rejects(self) -> None:
        reader = self._merged_reader(recorded=True, synced=True)
        reader.force_pushes.clear()
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(SOURCE, HEAD)), Rejected
        )

    def test_stale_dispatch_does_not_adopt_newer_recovery_authority(self) -> None:
        reader = planner_reader(pull_request(head_sha=UPDATED))
        stale = plan_promotion(
            reader, PromotionRequest(SOURCE, HEAD, READY.identifier - 1)
        )
        self.assertIsInstance(stale, Stale)
        self.assertIsNone(stale.approval)
        legacy = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
        self.assertIsInstance(legacy, Stale)
        self.assertIsNone(legacy.approval)
        current = plan_promotion(
            reader, PromotionRequest(SOURCE, HEAD, READY.identifier)
        )
        if not isinstance(current, Stale):
            self.fail("Expected a stale approved head")
        approval = current.approval
        if approval is None:
            self.fail("The exact current approval should be retained")
        self.assertEqual(approval.event_id, READY.identifier)

    def test_closed_or_draft_source_is_noop_without_reading_approval(self) -> None:
        for source in (
            pull_request(state=PullRequestState.CLOSED),
            pull_request(draft=True),
        ):
            with self.subTest(source=source):
                reader = planner_reader(source)
                reader.events.clear()
                result = plan_promotion(reader, PromotionRequest(SOURCE, HEAD))
                self.assertIsInstance(result, Stale)
                self.assertIsNone(result.approval)
                self.assertNotIn("events", reader.calls)

    def test_private_legacy_approval_requires_current_label_and_writer(self) -> None:
        scope = PromotionScope(UV_SECURITY_REPOSITORY, 101)
        source = pull_request(scope, labels=("bot:promote",))
        reader = planner_reader(source)
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(scope, HEAD)), Publish
        )
        reader.permission = RepositoryPermission.READ
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(scope, HEAD)), Rejected
        )
        reader.permission = RepositoryPermission.WRITE
        reader.pull_requests[scope] = pull_request(scope)
        self.assertIsInstance(
            plan_promotion(reader, PromotionRequest(scope, HEAD)), Rejected
        )

    def test_private_label_approval_checks_latest_transition_and_readiness(
        self,
    ) -> None:
        scope = PromotionScope(UV_SECURITY_REPOSITORY, 101)
        source = pull_request(scope, labels=("bot:promote",))
        reader = planner_reader(source)
        label = LabelAddedEvent(READY.identifier + 2, HUMAN, LATER, "bot:promote")
        reader.events[scope] = (READY, label)
        request = PromotionRequest(
            scope,
            HEAD,
            label.identifier,
            PromotionApprovalKind.LABELED,
            READY.identifier,
        )
        result = plan_promotion(reader, request)
        if not isinstance(result, Publish):
            self.fail("Expected a private publication plan")
        self.assertEqual(result.approval.ready_event_id, READY.identifier)
        reader.events[scope] = (
            READY,
            label,
            LabelRemovedEvent(label.identifier + 1, HUMAN, LATER, "bot:promote"),
        )
        self.assertIsInstance(plan_promotion(reader, request), Stale)
        reader.events[scope] = (
            READY,
            ReadyForReviewEvent(READY.identifier + 1, HUMAN, LATER),
            label,
        )
        stale = plan_promotion(reader, request)
        self.assertIsInstance(stale, Stale)
        self.assertIsNone(stale.approval)


if __name__ == "__main__":
    unittest.main()
