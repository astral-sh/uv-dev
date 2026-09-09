import unittest
from dataclasses import dataclass, field, replace

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
    GitHubAppIdentity,
    HeadForcePush,
    PromotionActor,
    PromotionComment,
    PromotionPullRequest,
    PromotionScope,
    PullRequestMerge,
    PullRequestSelection,
)
from uv_automations.workflows.promotion_retarget import (
    RetargetBatch,
    Retargeted,
    RetargetSkipReason,
    SkippedRetarget,
    StaleRetargetSync,
    apply_retargets,
    plan_retargets,
    retarget,
)

HEAD = CommitSha("a" * 40)
PARENT_HEAD = CommitSha("b" * 40)
MERGE = CommitSha("c" * 40)
MAIN = CommitSha("d" * 40)
PUBLIC_MAIN = CommitSha("e" * 40)
REBASED_PARENT = CommitSha("f" * 40)
OTHER = CommitSha("1" * 40)
NOW = Timestamp.parse("2026-09-09T12:00:00Z")
BOT = PromotionActor("astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT)
HUMAN = PromotionActor("maintainer", 1234, ActorKind.USER)


def pull_request(
    repository: RepositoryIdentity,
    number: int,
    *,
    state: PullRequestState = PullRequestState.OPEN,
    draft: bool = True,
    base_ref: str = "parent",
    base_sha: CommitSha = PARENT_HEAD,
    head_ref: str = "child",
    head_sha: CommitSha = HEAD,
    head_repository: RepositoryIdentity | None = None,
    merge: PullRequestMerge | None = None,
    author: PromotionActor | None = BOT,
) -> PromotionPullRequest:
    scope = PromotionScope(repository, number)
    return PromotionPullRequest(
        scope=scope,
        details=PullRequestDetails(
            reference=PullRequestRef(repository.name, number),
            state=state,
            url=f"https://github.com/{repository.name}/pull/{number}",
            base=PullRequestRevision(repository, base_ref, base_sha),
            head=PullRequestRevision(head_repository or repository, head_ref, head_sha),
            labels=(),
        ),
        draft=draft,
        author=author,
        title="Sensitive source title",
        body="Sensitive source body",
        merge=merge,
    )


def comparison(
    base: CommitSha, head: CommitSha, *, contains: bool = True
) -> CommitComparison:
    if base == head:
        return CommitComparison(
            UV_REPOSITORY, base, head, ComparisonStatus.IDENTICAL, base
        )
    return CommitComparison(
        UV_REPOSITORY,
        base,
        head,
        ComparisonStatus.AHEAD if contains else ComparisonStatus.DIVERGED,
        base if contains else OTHER,
    )


@dataclass
class FakeGitHub:
    pull_requests: dict[PromotionScope, PromotionPullRequest] = field(
        default_factory=dict
    )
    comments: dict[PromotionScope, tuple[PromotionComment, ...]] = field(
        default_factory=dict
    )
    histories: dict[PromotionScope, tuple[HeadForcePush, ...]] = field(
        default_factory=dict
    )
    read_responses: dict[PromotionScope, list[PromotionPullRequest]] = field(
        default_factory=dict
    )
    refs: dict[tuple[RepositoryIdentity, str], CommitSha | None] = field(
        default_factory=dict
    )
    comparisons: dict[
        tuple[RepositoryIdentity, CommitSha, CommitSha], CommitComparison
    ] = field(default_factory=dict)
    ref_responses: dict[tuple[RepositoryIdentity, str], list[CommitSha | None]] = field(
        default_factory=dict
    )
    list_calls: list[
        tuple[RepositoryIdentity, PullRequestSelection, str | None, str | None]
    ] = field(default_factory=list)
    ref_calls: list[tuple[RepositoryIdentity, str]] = field(default_factory=list)
    comparison_calls: list[tuple[RepositoryIdentity, CommitSha, CommitSha]] = field(
        default_factory=list
    )

    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        responses = self.read_responses.get(scope)
        if responses:
            return responses.pop(0)
        return self.pull_requests[scope]

    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]:
        self.list_calls.append((repository, state, head, base))
        result = []
        for item in self.pull_requests.values():
            if item.scope.repository != repository:
                continue
            if state != PullRequestSelection.ALL and item.details.state.value != state:
                continue
            if head is not None and item.details.head.ref != head:
                continue
            if base is not None and item.details.base.ref != base:
                continue
            result.append(item)
        return tuple(result)

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]:
        return self.comments.get(scope, ())

    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]:
        return self.histories.get(scope, ())

    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        key = repository, ref
        self.ref_calls.append(key)
        responses = self.ref_responses.get(key)
        if responses:
            return responses.pop(0)
        return self.refs[key]

    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        key = repository, base, head
        self.comparison_calls.append(key)
        return self.comparisons[key]


@dataclass
class FakeWriter:
    reader: FakeGitHub
    writes: list[PromotionScope] = field(default_factory=list)
    response: PromotionPullRequest | None = None

    def retarget_to_main(self, source: PromotionScope) -> PromotionPullRequest:
        self.writes.append(source)
        current = self.reader.pull_requests[source]
        updated = self.response or replace(
            current,
            details=replace(
                current.details,
                base=PullRequestRevision(source.repository, "main", MAIN),
            ),
        )
        self.reader.pull_requests[source] = updated
        return updated


@dataclass
class RetargetFixture:
    repository: RepositoryIdentity = UV_DEV_REPOSITORY
    source: FakeGitHub = field(default_factory=FakeGitHub)
    upstream: FakeGitHub = field(default_factory=FakeGitHub)

    def __post_init__(self) -> None:
        child = pull_request(self.repository, 124)
        parent = pull_request(
            self.repository,
            123,
            state=PullRequestState.CLOSED,
            draft=False,
            base_ref="main",
            base_sha=MAIN,
            head_ref="parent",
            head_sha=PARENT_HEAD,
        )
        upstream = pull_request(
            UV_REPOSITORY,
            21500,
            state=PullRequestState.CLOSED,
            draft=False,
            base_ref="main",
            base_sha=PUBLIC_MAIN,
            head_ref="parent",
            head_sha=PARENT_HEAD,
            merge=PullRequestMerge(MERGE, NOW),
        )
        self.source.pull_requests = {child.scope: child, parent.scope: parent}
        self.source.comments[parent.scope] = (
            PromotionComment(
                parent.scope,
                91,
                BOT,
                AUTOMATIONS_APP,
                "Promoted to [#21500](https://github.com/astral-sh/uv/pull/21500).",
                NOW,
                NOW,
            ),
        )
        self.source.refs[self.repository, "main"] = MAIN
        self.upstream.pull_requests[upstream.scope] = upstream
        self.upstream.refs[UV_REPOSITORY, "main"] = PUBLIC_MAIN
        self.upstream.comparisons[UV_REPOSITORY, MAIN, PUBLIC_MAIN] = comparison(
            MAIN, PUBLIC_MAIN
        )
        self.upstream.comparisons[UV_REPOSITORY, MERGE, MAIN] = comparison(MERGE, MAIN)

    @property
    def child(self) -> PromotionPullRequest:
        return self.source.pull_requests[PromotionScope(self.repository, 124)]

    @property
    def parent(self) -> PromotionPullRequest:
        return self.source.pull_requests[PromotionScope(self.repository, 123)]

    @property
    def upstream_parent(self) -> PromotionPullRequest:
        return self.upstream.pull_requests[PromotionScope(UV_REPOSITORY, 21500)]

    def plan(self) -> RetargetBatch:
        result = plan_retargets(self.source, self.upstream, self.repository, MAIN)
        assert isinstance(result, RetargetBatch), "Expected a current retarget batch"
        return result


class PromotionRetargetTests(unittest.TestCase):
    def test_plans_only_same_repository_draft_children(self) -> None:
        fixture = RetargetFixture()
        for child in (
            pull_request(UV_DEV_REPOSITORY, 125, head_ref="second-child"),
            pull_request(UV_DEV_REPOSITORY, 126, draft=False),
            pull_request(UV_DEV_REPOSITORY, 127, base_ref="main", base_sha=MAIN),
            pull_request(UV_DEV_REPOSITORY, 128, head_repository=UV_REPOSITORY),
            pull_request(UV_DEV_REPOSITORY, 129, head_ref="main", head_sha=MAIN),
        ):
            fixture.source.pull_requests[child.scope] = child
        batch = fixture.plan()
        self.assertEqual([plan.source.number for plan in batch.plans], [124, 125])
        self.assertEqual((batch.ready_children, batch.unverified_children), (1, 0))
        self.assertEqual(batch.main, BranchRevision(UV_DEV_REPOSITORY, "main", MAIN))
        self.assertEqual(batch.plans[0].base.sha, PARENT_HEAD)
        self.assertEqual(batch.plans[0].head.sha, HEAD)
        self.assertEqual(batch.plans[0].parent.record.comment_id, 91)
        self.assertEqual(
            fixture.source.list_calls,
            [
                (UV_DEV_REPOSITORY, PullRequestSelection.OPEN, None, None),
                (UV_DEV_REPOSITORY, PullRequestSelection.CLOSED, "parent", None),
            ],
        )

    def test_source_main_must_match_the_sync_result(self) -> None:
        fixture = RetargetFixture()
        fixture.source.refs[UV_DEV_REPOSITORY, "main"] = OTHER
        self.assertEqual(
            plan_retargets(fixture.source, fixture.upstream, UV_DEV_REPOSITORY, MAIN),
            StaleRetargetSync(UV_DEV_REPOSITORY),
        )
        self.assertEqual(fixture.source.list_calls, [])
        self.assertEqual(fixture.upstream.ref_calls, [])

    def test_sync_revision_and_parent_merge_must_be_on_public_main(self) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        fixture.upstream.comparisons[UV_REPOSITORY, MAIN, PUBLIC_MAIN] = comparison(
            MAIN, PUBLIC_MAIN, contains=False
        )
        with self.assertRaisesRegex(ValueError, "not on public main"):
            fixture.plan()
        self.assertEqual(fixture.source.list_calls, [])

        fixture = RetargetFixture()
        fixture.upstream.comparisons[UV_REPOSITORY, MERGE, MAIN] = comparison(
            MERGE, MAIN, contains=False
        )
        batch = fixture.plan()
        self.assertEqual(batch.plans, ())
        self.assertEqual(batch.unverified_children, 1)

    def test_comparison_identity_is_checked(self) -> None:
        fixture = RetargetFixture()
        fixture.upstream.comparisons[UV_REPOSITORY, MAIN, PUBLIC_MAIN] = comparison(
            MERGE, MAIN
        )
        with self.assertRaisesRegex(ValueError, "different public revisions"):
            fixture.plan()

    def test_parent_must_be_unique_closed_and_at_the_original_base(self) -> None:
        for change in ("missing", "duplicate", "open", "different-head", "foreign"):
            with self.subTest(change=change):
                fixture = RetargetFixture()
                parent = fixture.parent
                match change:
                    case "missing":
                        del fixture.source.pull_requests[parent.scope]
                    case "duplicate":
                        duplicate = replace(
                            parent,
                            scope=PromotionScope(UV_DEV_REPOSITORY, 122),
                            details=replace(
                                parent.details,
                                reference=PullRequestRef(UV_DEV_REPOSITORY.name, 122),
                            ),
                        )
                        fixture.source.pull_requests[duplicate.scope] = duplicate
                    case "open":
                        fixture.source.pull_requests[parent.scope] = replace(
                            parent,
                            details=replace(
                                parent.details, state=PullRequestState.OPEN
                            ),
                        )
                    case "different-head":
                        fixture.source.pull_requests[parent.scope] = replace(
                            parent,
                            details=replace(
                                parent.details,
                                head=replace(parent.details.head, sha=OTHER),
                            ),
                        )
                    case "foreign":
                        fixture.source.pull_requests[parent.scope] = replace(
                            parent,
                            details=replace(
                                parent.details,
                                head=replace(
                                    parent.details.head, repository=UV_REPOSITORY
                                ),
                            ),
                        )
                self.assertEqual(fixture.plan().plans, ())

    def test_forged_or_ambiguous_promotion_records_are_not_authority(self) -> None:
        fixture = RetargetFixture()
        original = fixture.source.comments[fixture.parent.scope][0]
        invalid = (
            (),
            (replace(original, author=HUMAN),),
            (replace(original, app=GitHubAppIdentity(1, AUTOMATIONS_APP.slug)),),
            (
                replace(
                    original,
                    body="Promoted to [#21500](https://github.com/astral-sh/uv/pull/9).",
                ),
            ),
            (
                original,
                replace(
                    original,
                    identifier=92,
                    body="Promoted to [#21501](https://github.com/astral-sh/uv/pull/21501).",
                ),
            ),
        )
        for comments in invalid:
            with self.subTest(comments=comments):
                fixture.source.comments[fixture.parent.scope] = comments
                self.assertEqual(fixture.plan().plans, ())

    def test_parent_is_reread_after_unique_discovery(self) -> None:
        fixture = RetargetFixture()
        parent = fixture.parent
        fixture.source.read_responses[parent.scope] = [
            replace(
                parent,
                details=replace(
                    parent.details, head=replace(parent.details.head, sha=OTHER)
                ),
            )
        ]
        self.assertEqual(fixture.plan().plans, ())

    def test_upstream_parent_requires_bot_identity_and_a_merge(self) -> None:
        fixture = RetargetFixture()
        parent = fixture.upstream_parent
        invalid = (
            replace(parent, author=HUMAN),
            replace(parent, merge=None),
            replace(
                parent,
                merge=None,
                details=replace(parent.details, state=PullRequestState.OPEN),
            ),
            replace(
                parent,
                details=replace(
                    parent.details,
                    head=replace(parent.details.head, repository=UV_DEV_REPOSITORY),
                ),
            ),
            replace(
                parent,
                details=replace(
                    parent.details, head=replace(parent.details.head, ref="other")
                ),
            ),
        )
        for value in invalid:
            with self.subTest(parent=value):
                fixture.upstream.pull_requests[parent.scope] = value
                self.assertEqual(fixture.plan().plans, ())

    def test_rebased_parent_requires_the_original_head_in_verified_history(
        self,
    ) -> None:
        fixture = RetargetFixture()
        parent = fixture.upstream_parent
        fixture.upstream.pull_requests[parent.scope] = replace(
            parent,
            details=replace(
                parent.details, head=replace(parent.details.head, sha=REBASED_PARENT)
            ),
        )
        self.assertEqual(fixture.plan().plans, ())
        fixture.upstream.histories[parent.scope] = (
            HeadForcePush("force-push", PARENT_HEAD, REBASED_PARENT, NOW),
        )
        self.assertEqual(len(fixture.plan().plans), 1)

    def test_apply_only_retargets_the_base_and_is_idempotent(self) -> None:
        fixture = RetargetFixture()
        batch = fixture.plan()
        writer = FakeWriter(fixture.source)
        self.assertEqual(
            apply_retargets(fixture.source, fixture.upstream, writer, batch),
            (Retargeted(fixture.child.scope),),
        )
        self.assertEqual(fixture.child.details.head.sha, HEAD)
        self.assertEqual(fixture.child.details.base.ref, "main")
        self.assertEqual(writer.writes, [fixture.child.scope])
        self.assertEqual(
            retarget(fixture.source, fixture.upstream, writer, batch.plans[0]),
            SkippedRetarget(fixture.child.scope, RetargetSkipReason.ALREADY_TARGETED),
        )
        self.assertEqual(writer.writes, [fixture.child.scope])
        self.assertEqual(fixture.plan().plans, ())

    def test_apply_rechecks_source_head_base_state_and_readiness(self) -> None:
        fixture = RetargetFixture()
        plan = fixture.plan().plans[0]
        original = fixture.child
        changed = (
            replace(
                original,
                details=replace(original.details, state=PullRequestState.CLOSED),
            ),
            replace(
                original,
                details=replace(
                    original.details, head=replace(original.details.head, sha=OTHER)
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details, head=replace(original.details.head, ref="new")
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details, base=replace(original.details.base, sha=OTHER)
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details, base=replace(original.details.base, ref="new")
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details,
                    head=replace(original.details.head, repository=UV_REPOSITORY),
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details,
                    head=replace(original.details.head, repository=None),
                ),
            ),
        )
        writer = FakeWriter(fixture.source)
        for current in changed:
            with self.subTest(current=current):
                fixture.source.pull_requests[original.scope] = current
                self.assertEqual(
                    retarget(fixture.source, fixture.upstream, writer, plan),
                    SkippedRetarget(original.scope, RetargetSkipReason.SOURCE_CHANGED),
                )
        fixture.source.pull_requests[original.scope] = replace(original, draft=False)
        self.assertEqual(
            retarget(fixture.source, fixture.upstream, writer, plan),
            SkippedRetarget(original.scope, RetargetSkipReason.READY_FOR_PROMOTION),
        )
        self.assertEqual(writer.writes, [])

    def test_apply_rechecks_source_main_and_parent_record(self) -> None:
        for change in (
            "source-main",
            "public-main",
            "during-parent",
            "record",
            "parent-head",
            "merge",
        ):
            with self.subTest(change=change):
                fixture = RetargetFixture()
                plan = fixture.plan().plans[0]
                writer = FakeWriter(fixture.source)
                reason = RetargetSkipReason.MAIN_CHANGED
                match change:
                    case "source-main":
                        fixture.source.refs[UV_DEV_REPOSITORY, "main"] = OTHER
                    case "public-main":
                        fixture.upstream.comparisons[
                            UV_REPOSITORY, MAIN, PUBLIC_MAIN
                        ] = comparison(MAIN, PUBLIC_MAIN, contains=False)
                    case "during-parent":
                        fixture.source.ref_responses[UV_DEV_REPOSITORY, "main"] = [
                            MAIN,
                            OTHER,
                        ]
                    case "record":
                        original = fixture.source.comments[fixture.parent.scope][0]
                        fixture.source.comments[fixture.parent.scope] = (
                            replace(original, identifier=92),
                        )
                        reason = RetargetSkipReason.PARENT_CHANGED
                    case "parent-head":
                        parent = fixture.parent
                        fixture.source.pull_requests[parent.scope] = replace(
                            parent,
                            details=replace(
                                parent.details,
                                head=replace(parent.details.head, sha=OTHER),
                            ),
                        )
                        reason = RetargetSkipReason.PARENT_CHANGED
                    case "merge":
                        parent = fixture.upstream_parent
                        fixture.upstream.pull_requests[parent.scope] = replace(
                            parent, merge=PullRequestMerge(OTHER, NOW)
                        )
                        reason = RetargetSkipReason.PARENT_CHANGED
                self.assertEqual(
                    retarget(fixture.source, fixture.upstream, writer, plan),
                    SkippedRetarget(fixture.child.scope, reason),
                )
                self.assertEqual(writer.writes, [])

    def test_response_checks_detect_changes_during_the_base_update(self) -> None:
        for change in ("head", "draft", "main", "closed"):
            with self.subTest(change=change):
                fixture = RetargetFixture()
                plan = fixture.plan().plans[0]
                updated = replace(
                    fixture.child,
                    details=replace(
                        fixture.child.details,
                        base=PullRequestRevision(UV_DEV_REPOSITORY, "main", MAIN),
                    ),
                )
                match change:
                    case "head":
                        updated = replace(
                            updated,
                            details=replace(
                                updated.details,
                                head=replace(updated.details.head, sha=OTHER),
                            ),
                        )
                    case "draft":
                        updated = replace(updated, draft=False)
                    case "main":
                        updated = replace(
                            updated,
                            details=replace(
                                updated.details,
                                base=replace(updated.details.base, sha=OTHER),
                            ),
                        )
                    case "closed":
                        updated = replace(
                            updated,
                            details=replace(
                                updated.details, state=PullRequestState.CLOSED
                            ),
                        )
                writer = FakeWriter(fixture.source, response=updated)
                with self.assertRaisesRegex(ValueError, "changed while retargeting"):
                    retarget(fixture.source, fixture.upstream, writer, plan)
                self.assertEqual(writer.writes, [plan.source])

    def test_plan_identity_is_not_an_unchecked_write_capability(self) -> None:
        fixture = RetargetFixture()
        plan = fixture.plan().plans[0]
        invalid = (
            {"source": PromotionScope(UV_REPOSITORY, 124)},
            {"base": BranchRevision(UV_REPOSITORY, "parent", PARENT_HEAD)},
            {"head": BranchRevision(UV_REPOSITORY, "child", HEAD)},
            {"base": BranchRevision(UV_DEV_REPOSITORY, "parent", OTHER)},
            {"main": BranchRevision(UV_DEV_REPOSITORY, "other", MAIN)},
            {"source": fixture.parent.scope},
        )
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                replace(plan, **changes)
        with self.assertRaises(ValueError):
            plan_retargets(
                fixture.source,
                fixture.upstream,
                RepositoryIdentity(UV_DEV_REPOSITORY.name, 1),
                MAIN,
            )

    def test_uv_security_uses_the_same_exact_identity_checks(self) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        batch = fixture.plan()
        self.assertEqual(
            batch.plans[0].source, PromotionScope(UV_SECURITY_REPOSITORY, 124)
        )
        writer = FakeWriter(fixture.source)
        self.assertEqual(
            apply_retargets(fixture.source, fixture.upstream, writer, batch),
            (Retargeted(fixture.child.scope),),
        )
