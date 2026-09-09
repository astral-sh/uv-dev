import argparse
import io
import json
import os
import shlex
import subprocess
import unittest
from collections.abc import Sequence
from contextlib import redirect_stdout
from dataclasses import dataclass, field, replace
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch
from uuid import UUID

from uv_automations.artifacts import CommitRange
from uv_automations.comment_models import (
    MAX_ACTIONS,
    MAX_COLLECTION_PAGES,
    MAX_CONTINUATIONS,
    ActorKind,
    AuthorAssociation,
    CollectionCheckpoint,
    CommentAction,
    CommentAuthor,
    CommentOutcome,
    CommentRecommendation,
    CommentScope,
    CommentTarget,
    CommentTargetKind,
    ConversationComment,
    InlineComment,
    ReviewState,
    ReviewThread,
    SubmittedReview,
    ThreadComment,
    ThreadPage,
    ThreadRoot,
)
from uv_automations.comments_cli import (
    FindSource,
    add_commands,
    main,
    parse_command,
)
from uv_automations.git import Git
from uv_automations.github_actions import ActionsRun, ArtifactIdentity, WorkflowRun
from uv_automations.github_comments import CommentPullRequest
from uv_automations.models import (
    CommitSha,
    PullRequestDetails,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)
from uv_automations.sessions import snapshot_sessions
from uv_automations.workflows.comments import (
    BOT_EMAIL,
    BOT_NAME,
    CHECKPOINT_DISCOVERY_RUNS,
    WORKFLOW,
    CheckpointLocator,
    FeedbackArtifactKind,
    FeedbackCheckpoint,
    FeedbackPublication,
    PreparedFeedback,
    RetainedFeedback,
    apply_publication,
    artifact_name,
    collect_comments,
    find_checkpoint,
    persist_feedback_result,
    prepare_feedback,
    prepare_publication,
    require_eligible,
    validate_checkpoint,
    verify_commits,
    write_json_file,
)

REPOSITORY = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
SCOPE = CommentScope(REPOSITORY, 123)
BASE = CommitSha("a" * 40)
HEAD = CommitSha("b" * 40)
WORKFLOW_SHA = CommitSha("d" * 40)
EARLIER = Timestamp.parse("2026-09-08T12:00:00Z")
LATER = Timestamp.parse("2026-09-08T12:01:00Z")
FUTURE = Timestamp.parse("2026-09-08T12:02:00Z")
PRIOR_RUN = ActionsRun(REPOSITORY, 100, 1, WORKFLOW_SHA)
CURRENT_RUN = ActionsRun(REPOSITORY, 200, 2, WORKFLOW_SHA)
SESSION_ID = UUID("123e4567-e89b-12d3-a456-426614174000")
TRUSTED = CommentAuthor("maintainer", ActorKind.USER, AuthorAssociation.MEMBER)
OUTSIDER = CommentAuthor("contributor", ActorKind.USER, AuthorAssociation.CONTRIBUTOR)
BOT = CommentAuthor(BOT_NAME, ActorKind.BOT, AuthorAssociation.MEMBER)
THREAD_ID = "PRRT_test-thread"


def pull_request(head: CommitSha = HEAD) -> CommentPullRequest:
    return CommentPullRequest(
        PullRequestDetails(
            reference=SCOPE.reference,
            state=PullRequestState.OPEN,
            url=f"https://github.com/{REPOSITORY.name}/pull/{SCOPE.number}",
            base=PullRequestRevision(REPOSITORY, "main", BASE),
            head=PullRequestRevision(REPOSITORY, "feature", head),
            labels=(),
        ),
        draft=False,
        event_json='{"number":123}',
    )


def workflow_run(source: ActionsRun, *, successful: bool = True) -> WorkflowRun:
    return WorkflowRun(
        source,
        REPOSITORY,
        WORKFLOW,
        "workflow_dispatch",
        "main",
        "completed" if successful else "in_progress",
        "success" if successful else None,
        Timestamp.parse("2026-09-08T11:59:00Z"),
    )


def artifact(
    kind: FeedbackArtifactKind, source: ActionsRun, identifier: int
) -> ArtifactIdentity:
    return ArtifactIdentity(
        source, identifier, artifact_name(kind, SCOPE, source), "sha256:" + "e" * 64
    )


def conversation(
    identifier: int = 1,
    *,
    body: str = "Please clarify",
    author: CommentAuthor = TRUSTED,
    updated_at: Timestamp = EARLIER,
) -> ConversationComment:
    return ConversationComment(identifier, author, body, updated_at)


def inline(
    identifier: int = 2,
    *,
    root: int = 2,
    author: CommentAuthor = TRUSTED,
    updated_at: Timestamp = EARLIER,
) -> InlineComment:
    return InlineComment(
        identifier,
        root,
        author,
        "Please fix this",
        updated_at,
        "file.py",
        "@@ -1 +1 @@",
    )


def review(
    identifier: int = 3,
    *,
    body: str = "Please simplify",
    updated_at: Timestamp = EARLIER,
) -> SubmittedReview:
    return SubmittedReview(
        identifier, TRUSTED, body, updated_at, ReviewState.CHANGES_REQUESTED
    )


def thread() -> ReviewThread:
    return ReviewThread(
        THREAD_ID,
        False,
        False,
        "file.py",
        (ThreadComment(2, TRUSTED, "Please fix this", EARLIER),),
    )


def collection_checkpoint() -> CollectionCheckpoint:
    return CollectionCheckpoint(
        EARLIER, "cursor-1", (ThreadRoot(2, THREAD_ID),), (review().revision,)
    )


def feedback_checkpoint(
    *,
    source: ActionsRun = PRIOR_RUN,
    head: CommitSha = HEAD,
    collection: CollectionCheckpoint | None = None,
) -> FeedbackCheckpoint:
    return FeedbackCheckpoint(
        scope=SCOPE,
        source=source,
        head=head,
        collection=collection if collection is not None else collection_checkpoint(),
        preparation=artifact(FeedbackArtifactKind.CONTEXT, source, 302),
        result=artifact(FeedbackArtifactKind.RESULT, source, 303),
        session=artifact(FeedbackArtifactKind.SESSION, source, 301),
        session_id=SESSION_ID,
        processed_targets=1,
        continuations_remaining=MAX_CONTINUATIONS,
        continuation_key=None,
    )


@dataclass
class FakeGitHub:
    pull_request: CommentPullRequest = field(default_factory=pull_request)
    conversation: tuple[ConversationComment, ...] = ()
    inline: tuple[InlineComment, ...] = ()
    reviews: tuple[SubmittedReview, ...] = ()
    pages: dict[str | None, ThreadPage] = field(
        default_factory=lambda: {
            None: ThreadPage((ThreadRoot(2, THREAD_ID),), "cursor-1", False),
            "cursor-1": ThreadPage((), None, False),
        }
    )
    threads: dict[str, ReviewThread] = field(
        default_factory=lambda: {THREAD_ID: thread()}
    )
    runs: tuple[WorkflowRun, ...] = ()
    workflow_runs: dict[ActionsRun, WorkflowRun] = field(default_factory=dict)
    artifacts: dict[int, ArtifactIdentity] = field(default_factory=dict)
    since: list[tuple[str, Timestamp | None]] = field(default_factory=list)
    cursors: list[str | None] = field(default_factory=list)
    thread_queries: list[tuple[str, ...]] = field(default_factory=list)
    run_queries: list[tuple[RepositoryName, str, int]] = field(default_factory=list)
    artifact_queries: list[tuple[ActionsRun, str]] = field(default_factory=list)
    posts: list[str] = field(default_factory=list)
    replies: list[tuple[str, str]] = field(default_factory=list)
    resolutions: list[str] = field(default_factory=list)

    def get_comment_pull_request(self, scope: CommentScope) -> CommentPullRequest:
        if scope != SCOPE:
            raise AssertionError("Unexpected pull request scope")
        return self.pull_request

    def list_conversation_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[ConversationComment, ...]:
        self.get_comment_pull_request(scope)
        self.since.append(("conversation", since))
        return tuple(
            comment
            for comment in self.conversation
            if since is None or comment.updated_at > since
        )

    def list_inline_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[InlineComment, ...]:
        self.get_comment_pull_request(scope)
        self.since.append(("inline", since))
        return tuple(
            comment
            for comment in self.inline
            if since is None or comment.updated_at > since
        )

    def list_reviews(self, scope: CommentScope) -> tuple[SubmittedReview, ...]:
        self.get_comment_pull_request(scope)
        return self.reviews

    def list_thread_roots(self, scope: CommentScope, cursor: str | None) -> ThreadPage:
        self.get_comment_pull_request(scope)
        self.cursors.append(cursor)
        return self.pages[cursor]

    def get_review_threads(
        self, scope: CommentScope, identifiers: tuple[str, ...]
    ) -> tuple[ReviewThread, ...]:
        self.get_comment_pull_request(scope)
        self.thread_queries.append(identifiers)
        return tuple(self.threads[identifier] for identifier in identifiers)

    def get_conversation_comment(
        self, scope: CommentScope, identifier: int
    ) -> ConversationComment:
        self.get_comment_pull_request(scope)
        return next(
            comment for comment in self.conversation if comment.identifier == identifier
        )

    def get_review(self, scope: CommentScope, identifier: int) -> SubmittedReview:
        self.get_comment_pull_request(scope)
        return next(
            review for review in self.reviews if review.identifier == identifier
        )

    def list_successful_workflow_runs(
        self, repository: RepositoryName, workflow: str, *, limit: int
    ) -> tuple[WorkflowRun, ...]:
        self.run_queries.append((repository, workflow, limit))
        return self.runs

    def read_workflow_run(
        self, repository: RepositoryIdentity, identifier: int, attempt: int
    ) -> WorkflowRun:
        return next(
            run
            for run in self.workflow_runs.values()
            if run.source.repository == repository
            and run.source.identifier == identifier
            and run.source.attempt == attempt
        )

    def get_workflow_run(self, source: ActionsRun) -> WorkflowRun:
        return self.workflow_runs[source]

    def find_artifact(self, source: ActionsRun, name: str) -> ArtifactIdentity | None:
        self.artifact_queries.append((source, name))
        return next(
            (
                artifact
                for artifact in self.artifacts.values()
                if artifact.source == source and artifact.name == name
            ),
            None,
        )

    def get_artifact(
        self, source: ActionsRun, identifier: int, name: str
    ) -> ArtifactIdentity:
        value = self.artifacts[identifier]
        if value.source != source or value.name != name:
            raise ValueError("Unexpected artifact provenance")
        return value


@dataclass(frozen=True, slots=True)
class FakeCommentWriter:
    github: FakeGitHub

    def post_conversation_comment(self, scope: CommentScope, body: str) -> None:
        self.github.get_comment_pull_request(scope)
        self.github.posts.append(body)
        self.github.conversation += (
            conversation(
                10_000 + len(self.github.posts), body=body, author=BOT, updated_at=LATER
            ),
        )

    def reply_to_review_thread(self, identifier: str, body: str) -> None:
        self.github.replies.append((identifier, body))
        current = self.github.threads[identifier]
        self.github.threads[identifier] = replace(
            current,
            comments=(
                *current.comments,
                ThreadComment(20_000 + len(self.github.replies), BOT, body, LATER),
            ),
        )

    def resolve_review_thread(self, identifier: str) -> None:
        self.github.resolutions.append(identifier)
        self.github.threads[identifier] = replace(
            self.github.threads[identifier], resolved=True
        )


class CommentCliTests(unittest.TestCase):
    def test_schema_command_uses_the_model_contract(self) -> None:
        output = io.StringIO()
        with (
            redirect_stdout(output),
            patch("uv_automations.comments_cli.logging.basicConfig"),
        ):
            main(["schema"])
        self.assertEqual(json.loads(output.getvalue()), CommentRecommendation.schema())

    def test_cli_reads_actions_context_without_shell_parsing(self) -> None:
        environment = {
            "GITHUB_REPOSITORY": str(REPOSITORY.name),
            "GITHUB_REPOSITORY_ID": str(REPOSITORY.database_id),
            "PULL_REQUEST_NUMBER": str(SCOPE.number),
            "EXPECTED_HEAD_SHA": str(HEAD),
            "GITHUB_RUN_ID": str(CURRENT_RUN.identifier),
            "GITHUB_RUN_ATTEMPT": str(CURRENT_RUN.attempt),
            "GITHUB_WORKFLOW_SHA": str(WORKFLOW_SHA),
        }
        with patch.dict(os.environ, environment):
            parser = argparse.ArgumentParser()
            add_commands(parser)
            parsed = parse_command(
                parser.parse_args(
                    [
                        "source",
                        "--destination",
                        "source.json",
                        "--github-output",
                        "output",
                    ]
                )
            )
        self.assertIsInstance(parsed, FindSource)
        if not isinstance(parsed, FindSource):
            raise TypeError("Expected source command")
        self.assertEqual(parsed.context.scope, SCOPE)
        self.assertEqual(parsed.context.source, CURRENT_RUN)


class CommentCollectionTests(unittest.TestCase):
    def test_pending_edits_newer_than_the_watermark_are_deferred_once(self) -> None:
        original = conversation(99)
        edited = conversation(99, body="Edited after collection", updated_at=FUTURE)
        previous = replace(collection_checkpoint(), pending=(original.revision,))
        github = FakeGitHub(conversation=(edited,))
        first = collect_comments(github, SCOPE, previous=previous, through=LATER)
        self.assertEqual(first.targets, ())
        self.assertEqual(first.processed_targets, 0)
        self.assertEqual(first.conversation, ())
        self.assertEqual(first.checkpoint.pending, (edited.revision,))
        second = collect_comments(
            github, SCOPE, previous=first.checkpoint, through=FUTURE
        )
        self.assertEqual(second.targets, (edited.revision,))
        self.assertEqual(second.processed_targets, 1)
        self.assertEqual(second.checkpoint.pending, ())
        third = collect_comments(
            github,
            SCOPE,
            previous=second.checkpoint,
            through=Timestamp.parse("2026-09-08T12:03:00Z"),
        )
        self.assertEqual(third.targets, ())

    def test_new_threads_with_post_watermark_human_replies_are_deferred(self) -> None:
        newer = replace(
            thread(),
            comments=(
                *thread().comments,
                ThreadComment(9, OUTSIDER, "New information", FUTURE),
            ),
        )
        github = FakeGitHub(
            inline=(inline(), inline(9, root=2, author=OUTSIDER, updated_at=FUTURE)),
            threads={THREAD_ID: newer},
        )
        first = collect_comments(github, SCOPE, previous=None, through=LATER)
        self.assertEqual(first.targets, ())
        self.assertEqual(first.processed_targets, 0)
        self.assertEqual(first.threads, ())
        self.assertEqual(first.checkpoint.pending, (newer.revision,))
        second = collect_comments(
            github, SCOPE, previous=first.checkpoint, through=FUTURE
        )
        self.assertEqual(second.targets, (newer.revision,))
        self.assertEqual(second.processed_targets, 1)
        self.assertEqual(second.checkpoint.pending, ())

    def test_automation_replies_remain_typed_but_are_not_agent_context(self) -> None:
        newer = replace(
            thread(),
            comments=(
                *thread().comments,
                ThreadComment(9, BOT, "Publisher retry marker", FUTURE),
            ),
        )
        github = FakeGitHub(inline=(inline(),), threads={THREAD_ID: newer})
        result = collect_comments(github, SCOPE, previous=None, through=LATER)
        self.assertEqual(result.targets, (thread().revision,))
        self.assertEqual(result.threads[0].comments, newer.comments)
        self.assertEqual(
            newer.to_json()["comments"],
            [comment.to_json() for comment in newer.comments],
        )
        payload = result.to_json()["review_threads"]
        self.assertEqual(payload, [thread().to_agent_json()])

    def test_a_bounded_batch_retains_undispositioned_feedback(self) -> None:
        github = FakeGitHub(
            conversation=tuple(
                conversation(identifier) for identifier in range(1, MAX_ACTIONS + 2)
            )
        )
        first = collect_comments(github, SCOPE, previous=None, through=LATER)
        self.assertEqual(len(first.targets), MAX_ACTIONS)
        self.assertEqual(first.processed_targets, MAX_ACTIONS)
        self.assertEqual(len(first.checkpoint.pending), 1)
        second = collect_comments(
            github, SCOPE, previous=first.checkpoint, through=FUTURE
        )
        self.assertEqual(len(second.targets), 1)
        self.assertEqual(second.processed_targets, 1)
        self.assertEqual(second.checkpoint.pending, ())
        self.assertEqual(
            {revision.target for revision in (*first.targets, *second.targets)},
            {comment.revision.target for comment in github.conversation},
        )

    def test_bootstrap_collects_complete_context_and_trusted_targets(self) -> None:
        github = FakeGitHub(
            conversation=(
                conversation(),
                conversation(4, author=OUTSIDER),
                conversation(5, author=BOT),
            ),
            inline=(inline(),),
            reviews=(review(),),
        )
        result = collect_comments(github, SCOPE, previous=None, through=LATER)
        self.assertEqual(
            [comment.identifier for comment in result.conversation], [1, 4]
        )
        self.assertEqual(github.since, [("conversation", None), ("inline", None)])
        self.assertEqual(github.cursors, [None])
        self.assertEqual(github.thread_queries, [(THREAD_ID,)])
        self.assertEqual(
            result.targets,
            (conversation().revision, review().revision, thread().revision),
        )
        self.assertEqual(
            CollectionCheckpoint.from_json(result.checkpoint.to_json()),
            result.checkpoint,
        )

    def test_incremental_collection_keeps_old_review_edits_and_skips_same_second_replays(
        self,
    ) -> None:
        previous = replace(
            collection_checkpoint(), comments=(conversation().version, inline().version)
        )
        github = FakeGitHub(
            conversation=(
                conversation(),
                conversation(4, updated_at=LATER),
                conversation(5, updated_at=FUTURE),
            ),
            inline=(inline(), inline(6, root=2, updated_at=LATER)),
            reviews=(review(body="Edited older review", updated_at=LATER),),
        )
        result = collect_comments(github, SCOPE, previous=previous, through=LATER)
        self.assertEqual(
            github.since,
            [("conversation", EARLIER.overlap()), ("inline", EARLIER.overlap())],
        )
        self.assertEqual(github.cursors, ["cursor-1"])
        self.assertEqual([comment.identifier for comment in result.conversation], [4])
        self.assertEqual([comment.identifier for comment in result.inline], [6])
        self.assertEqual(
            [item.body for item in result.reviews], ["Edited older review"]
        )
        self.assertEqual(result.checkpoint.through, LATER)
        self.assertEqual(
            result.checkpoint.comments,
            (github.conversation[1].version, github.inline[1].version),
        )

    def test_history_truncation_never_produces_a_complete_checkpoint(self) -> None:
        pages: dict[str | None, ThreadPage] = {}
        cursor: str | None = None
        for index in range(MAX_COLLECTION_PAGES):
            next_cursor = f"cursor-{index}"
            pages[cursor] = ThreadPage((), next_cursor, True)
            cursor = next_cursor
        github = FakeGitHub(pages=pages)
        with self.assertRaisesRegex(ValueError, "bounded collection budget"):
            collect_comments(github, SCOPE, previous=None, through=LATER)
        self.assertEqual(len(github.cursors), MAX_COLLECTION_PAGES)
        incomplete = {**collection_checkpoint().to_json(), "complete": False}
        with self.assertRaisesRegex(ValueError, "truncated collection"):
            CollectionCheckpoint.from_json(incomplete)

    def test_missing_thread_identity_fails_closed(self) -> None:
        github = FakeGitHub(inline=(inline(999, root=999),))
        with self.assertRaisesRegex(ValueError, "no review thread"):
            collect_comments(github, SCOPE, previous=None, through=LATER)

    def test_eligibility_requires_numeric_repository_identity_and_exact_head(
        self,
    ) -> None:
        original = pull_request()
        wrong_id = RepositoryIdentity(REPOSITORY.name, 1)
        for changed in (
            replace(original, draft=True),
            replace(
                original,
                details=replace(original.details, state=PullRequestState.CLOSED),
            ),
            replace(
                original,
                details=replace(
                    original.details, head=replace(original.details.head, sha=BASE)
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details,
                    head=replace(original.details.head, repository=wrong_id),
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details,
                    base=replace(original.details.base, repository=wrong_id),
                ),
            ),
            replace(
                original,
                details=replace(
                    original.details, head=replace(original.details.head, ref="main")
                ),
            ),
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                require_eligible(changed, SCOPE, HEAD)


class CommentCheckpointTests(unittest.TestCase):
    def test_explicit_checkpoint_uses_a_named_locator(self) -> None:
        selected = artifact(FeedbackArtifactKind.STATE, PRIOR_RUN, 300)
        github = FakeGitHub(
            workflow_runs={PRIOR_RUN: workflow_run(PRIOR_RUN)},
            artifacts={selected.identifier: selected},
        )
        self.assertEqual(
            find_checkpoint(
                github,
                SCOPE,
                CURRENT_RUN,
                explicit=CheckpointLocator(
                    PRIOR_RUN.identifier, PRIOR_RUN.attempt, 300
                ),
            ),
            selected,
        )

    def test_discovery_is_bounded_and_uses_exact_attempt_names(self) -> None:
        selected = artifact(FeedbackArtifactKind.STATE, PRIOR_RUN, 300)
        invalid = replace(
            workflow_run(ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA)),
            event="pull_request",
        )
        github = FakeGitHub(
            runs=(invalid, workflow_run(PRIOR_RUN)), artifacts={300: selected}
        )
        self.assertEqual(find_checkpoint(github, SCOPE, CURRENT_RUN), selected)
        self.assertEqual(
            github.run_queries,
            [(REPOSITORY.name, "pull-request-comments.yml", CHECKPOINT_DISCOVERY_RUNS)],
        )
        self.assertEqual(
            github.artifact_queries,
            [(PRIOR_RUN, artifact_name(FeedbackArtifactKind.STATE, SCOPE, PRIOR_RUN))],
        )

    def test_checkpoint_requires_successful_attempt_and_matching_session_identity(
        self,
    ) -> None:
        state_artifact = artifact(FeedbackArtifactKind.STATE, PRIOR_RUN, 300)
        session_artifact = artifact(FeedbackArtifactKind.SESSION, PRIOR_RUN, 301)
        checkpoint = feedback_checkpoint()
        github = FakeGitHub(
            workflow_runs={PRIOR_RUN: workflow_run(PRIOR_RUN)},
            artifacts={300: state_artifact, 301: session_artifact},
        )
        self.assertEqual(
            validate_checkpoint(github, SCOPE, state_artifact, checkpoint.to_json()),
            RetainedFeedback(state_artifact, checkpoint),
        )
        github.workflow_runs[PRIOR_RUN] = replace(
            workflow_run(PRIOR_RUN), conclusion="failure"
        )
        with self.assertRaisesRegex(ValueError, "provenance"):
            validate_checkpoint(github, SCOPE, state_artifact, checkpoint.to_json())
        github.workflow_runs[PRIOR_RUN] = workflow_run(PRIOR_RUN)
        github.artifacts[301] = replace(session_artifact, digest="sha256:" + "f" * 64)
        with self.assertRaisesRegex(ValueError, "provenance"):
            validate_checkpoint(github, SCOPE, state_artifact, checkpoint.to_json())


def create_repository(path: Path, *, bare: bool = False) -> Git:
    path.mkdir()
    repository = Git(path)
    repository.command(
        ("init", "--quiet", "--bare" if bare else "--initial-branch=main")
    )
    if not bare:
        repository.command(("config", "user.name", BOT_NAME))
        repository.command(("config", "user.email", BOT_EMAIL))
    return repository


def commit_file(
    repository: Git, name: str, content: str, *, message: str | None = None
) -> CommitSha:
    (repository.path / name).write_text(content, encoding="utf-8")
    repository.command(("add", "--", name))
    repository.command(("commit", "--quiet", "--message", message or name))
    return repository.resolve_commit("HEAD")


def write_session(path: Path, workspace: Path) -> None:
    path.mkdir()
    payload = {
        "type": "session_meta",
        "payload": {
            "id": str(SESSION_ID),
            "source": "exec",
            "originator": "codex_github_action",
            "cwd": str(workspace),
        },
    }
    (path / f"rollout-{SESSION_ID}.jsonl").write_text(
        json.dumps(payload) + "\n", encoding="utf-8"
    )


@dataclass(frozen=True, slots=True)
class LocalRemoteGit(Git):
    remote: Path
    github: FakeGitHub
    commands: list[tuple[str, ...]] = field(default_factory=list, compare=False)

    @override
    def command(
        self, arguments: Sequence[str], *, check: bool = True, input: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        self.commands.append(tuple(arguments))
        mapped = tuple(
            str(self.remote)
            if argument == f"https://github.com/{REPOSITORY.name}.git"
            else argument
            for argument in arguments
        )
        result = Git.command(self, mapped, check=check, input=input)
        if "push" in arguments and result.returncode == 0:
            head = Git(self.remote).resolve_commit("refs/heads/feature")
            self.github.pull_request = replace(
                self.github.pull_request,
                details=replace(
                    self.github.pull_request.details,
                    head=replace(self.github.pull_request.details.head, sha=head),
                ),
            )
        return result


class CommentPublicationTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        directory = TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.source = create_repository(self.root / "source")
        self.ancestor = commit_file(self.source, "ancestor.txt", "ancestor\n")
        self.base = commit_file(self.source, "base.txt", "base\n")
        self.remote = create_repository(self.root / "remote", bare=True)
        self.source.command(
            (
                "push",
                "--quiet",
                str(self.remote.path),
                f"{self.base}:refs/heads/feature",
            )
        )
        self.github = FakeGitHub(
            pull_request=pull_request(self.base),
            conversation=(conversation(),),
            reviews=(review(),),
        )
        self.writer = FakeCommentWriter(self.github)
        consumer = create_repository(self.root / "consumer")
        consumer.command(
            ("fetch", "--quiet", str(self.remote.path), "refs/heads/feature")
        )
        consumer.command(("checkout", "--quiet", "--detach", str(self.base)))
        self.consumer = LocalRemoteGit(consumer.path, self.remote.path, self.github)
        self.head = commit_file(self.source, "change.txt", "change\n")
        self.context = self.root / "context"
        self.context.mkdir()
        self.prepared = PreparedFeedback(
            scope=SCOPE,
            source=CURRENT_RUN,
            dispatch_head=self.base,
            head=self.base,
            head_ref="feature",
            base=self.base,
            base_ref="main",
            after=None,
            collection=collection_checkpoint(),
            targets=(conversation().revision, thread().revision),
            session_id=None,
            continuation=None,
            processed_targets=2,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        )
        write_json_file(self.context / "prepared.json", self.prepared.to_json())
        self.session = self.root / "sessions"
        write_session(self.session, self.consumer.path)
        self.session_artifact = artifact(FeedbackArtifactKind.SESSION, CURRENT_RUN, 501)
        self.github.artifacts[499] = artifact(
            FeedbackArtifactKind.CONTEXT, CURRENT_RUN, 499
        )
        self.github.artifacts[500] = artifact(
            FeedbackArtifactKind.RESULT, CURRENT_RUN, 500
        )
        self.github.artifacts[501] = self.session_artifact
        self.github.workflow_runs[CURRENT_RUN] = workflow_run(
            CURRENT_RUN, successful=False
        )

    def recommendation(self) -> CommentRecommendation:
        return CommentRecommendation(
            "Handled two comments",
            (
                CommentAction(
                    CommentTarget(CommentTargetKind.REVIEW_THREAD, THREAD_ID),
                    CommentOutcome.COMMIT_AND_RESOLVE,
                    "",
                    self.head,
                ),
                CommentAction(
                    CommentTarget(CommentTargetKind.CONVERSATION_COMMENT, "1"),
                    CommentOutcome.RESPOND,
                    "Thanks @maintainer",
                    None,
                ),
            ),
        )

    def publication(self) -> FeedbackPublication:
        result_directory = self.root / "result"
        persist_feedback_result(
            self.source,
            self.prepared,
            self.recommendation(),
            result_directory,
            source=CURRENT_RUN,
            scratch=self.root,
        )
        return prepare_publication(
            self.github,
            self.consumer,
            SCOPE,
            CURRENT_RUN,
            self.base,
            self.context,
            result_directory,
            self.session,
            trusted_root=self.root,
            preparation_artifact_id=499,
            result_artifact_id=500,
            session_artifact_id=501,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        )

    def test_result_transport_ignores_agent_owned_git_filters(self) -> None:
        marker = self.root / "agent-filter-ran"
        script = self.root / "agent-filter"
        script.write_text(
            f"#!/bin/sh\n: > {shlex.quote(str(marker))}\ncat\n", encoding="utf-8"
        )
        script.chmod(0o755)
        self.source.command(("config", "filter.agent.clean", shlex.quote(str(script))))
        attributes = self.source.path / ".git/info/attributes"
        attributes.parent.mkdir(exist_ok=True)
        attributes.write_text("*.txt filter=agent\n", encoding="utf-8")
        publication = self.publication()
        self.assertEqual(publication.commits, (self.head,))
        self.assertFalse(marker.exists())

    def test_real_bundle_and_local_push_round_trip_is_retryable(self) -> None:
        publication = self.publication()
        self.assertEqual(self.consumer.resolve_commit("HEAD"), self.base)
        self.assertFalse((self.consumer.path / "change.txt").exists())
        self.assertEqual(publication.commits, (self.head,))
        state = apply_publication(self.github, self.writer, self.consumer, publication)
        self.assertEqual(self.remote.resolve_commit("refs/heads/feature"), self.head)
        self.assertEqual(state.head, self.head)
        self.assertEqual(state.session, self.session_artifact)
        self.assertEqual(self.github.resolutions, [THREAD_ID])
        self.assertEqual(len(self.github.posts), 1)
        self.assertIn("@\u200bmaintainer", self.github.posts[0])
        self.assertIn(
            "<!-- uv-automations:pull-request-comments:", self.github.posts[0]
        )
        self.assertEqual(
            apply_publication(self.github, self.writer, self.consumer, publication),
            state,
        )
        self.assertEqual(self.github.resolutions, [THREAD_ID])
        self.assertEqual(len(self.github.posts), 1)
        pushes = [
            arguments for arguments in self.consumer.commands if "push" in arguments
        ]
        self.assertEqual(len(pushes), 1)
        self.assertIn(f"--force-with-lease=refs/heads/feature:{self.base}", pushes[0])

    def test_exact_lease_preserves_a_concurrent_intentional_rewind(self) -> None:
        publication = self.publication()
        self.remote.command(
            ("update-ref", "refs/heads/feature", str(self.ancestor), str(self.base))
        )
        with self.assertRaises(subprocess.CalledProcessError):
            apply_publication(self.github, self.writer, self.consumer, publication)
        self.assertEqual(
            self.remote.resolve_commit("refs/heads/feature"), self.ancestor
        )
        self.assertEqual(self.github.posts, [])
        self.assertEqual(self.github.resolutions, [])

    def test_edited_feedback_is_rejected_before_publication(self) -> None:
        result_directory = self.root / "result"
        persist_feedback_result(
            self.source,
            self.prepared,
            self.recommendation(),
            result_directory,
            source=CURRENT_RUN,
            scratch=self.root,
        )
        self.github.conversation = (conversation(body="Changed the question"),)
        with self.assertRaisesRegex(ValueError, "Feedback changed"):
            prepare_publication(
                self.github,
                self.consumer,
                SCOPE,
                CURRENT_RUN,
                self.base,
                self.context,
                result_directory,
                self.session,
                trusted_root=self.root,
                preparation_artifact_id=499,
                result_artifact_id=500,
                session_artifact_id=501,
                continuations_remaining=MAX_CONTINUATIONS,
                continuation_key=None,
            )
        self.assertEqual(self.remote.resolve_commit("refs/heads/feature"), self.base)

    def test_nonexistent_addressing_commit_is_rejected(self) -> None:
        recommendation = self.recommendation()
        recommendation = replace(
            recommendation,
            actions=(
                replace(recommendation.actions[0], addressing_commit=BASE),
                recommendation.actions[1],
            ),
        )
        with self.assertRaisesRegex(ValueError, "not a new feedback commit"):
            persist_feedback_result(
                self.source,
                self.prepared,
                recommendation,
                self.root / "result",
                source=CURRENT_RUN,
                scratch=self.root,
            )

    def test_unaccounted_commits_and_undispositioned_targets_are_rejected(self) -> None:
        no_actions = tuple(
            CommentAction(revision.target, CommentOutcome.NO_ACTION, "", None)
            for revision in self.prepared.targets
        )
        with self.assertRaisesRegex(ValueError, "Every new commit"):
            persist_feedback_result(
                self.source,
                self.prepared,
                CommentRecommendation("No authorized changes", no_actions),
                self.root / "unaccounted-result",
                source=CURRENT_RUN,
                scratch=self.root,
            )
        with self.assertRaisesRegex(ValueError, "Every prepared feedback target"):
            persist_feedback_result(
                self.source,
                self.prepared,
                CommentRecommendation(
                    "Forgot one target", (self.recommendation().actions[0],)
                ),
                self.root / "incomplete-result",
                source=CURRENT_RUN,
                scratch=self.root,
            )

    def test_failed_publisher_can_retry_mixed_producer_attempts(self) -> None:
        preparation_source = replace(CURRENT_RUN, attempt=1)
        result_source = replace(CURRENT_RUN, attempt=2)
        publisher_source = replace(CURRENT_RUN, attempt=3)
        retry_source = replace(CURRENT_RUN, attempt=4)
        prepared = replace(self.prepared, source=preparation_source)
        (self.context / "prepared.json").write_text(json.dumps(prepared.to_json()))
        self.github.artifacts[499] = artifact(
            FeedbackArtifactKind.CONTEXT, preparation_source, 499
        )
        self.github.artifacts[500] = artifact(
            FeedbackArtifactKind.RESULT, result_source, 500
        )
        self.github.artifacts[501] = artifact(
            FeedbackArtifactKind.SESSION, result_source, 501
        )
        self.github.workflow_runs.update(
            {
                preparation_source: replace(
                    workflow_run(preparation_source), conclusion="failure"
                ),
                result_source: replace(
                    workflow_run(result_source), conclusion="failure", started_at=FUTURE
                ),
                publisher_source: workflow_run(publisher_source, successful=False),
                retry_source: workflow_run(retry_source, successful=False),
            }
        )
        result_directory = self.root / "mixed-result"
        persist_feedback_result(
            self.source,
            prepared,
            self.recommendation(),
            result_directory,
            source=result_source,
            scratch=self.root,
        )

        def prepare(source: ActionsRun) -> FeedbackPublication:
            return prepare_publication(
                self.github,
                self.consumer,
                SCOPE,
                source,
                self.base,
                self.context,
                result_directory,
                self.session,
                trusted_root=self.root,
                preparation_artifact_id=499,
                result_artifact_id=500,
                session_artifact_id=501,
                continuations_remaining=MAX_CONTINUATIONS,
                continuation_key=None,
            )

        publication = prepare(publisher_source)
        with (
            patch.object(
                FakeCommentWriter,
                "post_conversation_comment",
                side_effect=RuntimeError("retry"),
            ),
            self.assertRaisesRegex(RuntimeError, "retry"),
        ):
            apply_publication(self.github, self.writer, self.consumer, publication)
        self.assertEqual(self.remote.resolve_commit("refs/heads/feature"), self.head)
        state = apply_publication(
            self.github, self.writer, self.consumer, prepare(retry_source)
        )
        self.assertEqual(state.source, retry_source)
        self.assertEqual(state.preparation.source, preparation_source)
        self.assertEqual(state.result.source, result_source)
        self.assertEqual(state.session.source, result_source)
        self.assertEqual(len(self.github.posts), 1)
        self.assertEqual(
            sum("push" in arguments for arguments in self.consumer.commands), 1
        )
        state_artifact = artifact(FeedbackArtifactKind.STATE, retry_source, 600)
        self.github.artifacts[600] = state_artifact
        self.github.workflow_runs[retry_source] = replace(
            workflow_run(retry_source), started_at=FUTURE
        )
        self.assertEqual(
            validate_checkpoint(self.github, SCOPE, state_artifact, state.to_json()),
            RetainedFeedback(state_artifact, state),
        )

    def test_coalesced_dispatch_only_follows_the_completed_checkpoint_head(
        self,
    ) -> None:
        self.source.command(
            (
                "push",
                "--quiet",
                str(self.remote.path),
                f"{self.head}:refs/heads/feature",
            )
        )
        self.github.pull_request = replace(
            pull_request(self.head),
            details=replace(
                pull_request(self.head).details,
                base=replace(pull_request(self.head).details.base, sha=self.base),
            ),
        )
        previous = RetainedFeedback(
            artifact(FeedbackArtifactKind.STATE, PRIOR_RUN, 300),
            feedback_checkpoint(head=self.head),
        )
        sessions = snapshot_sessions(
            self.session, self.consumer.path, trusted_root=self.root
        )
        prepared = prepare_feedback(
            self.github,
            self.consumer,
            SCOPE,
            CURRENT_RUN,
            self.base,
            self.root / "coalesced-context",
            trusted_files=Path(__file__).resolve().parents[3],
            previous=previous,
            sessions=sessions,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        )
        self.assertEqual(prepared.dispatch_head, self.base)
        self.assertEqual(prepared.head, self.head)
        self.assertEqual(prepared.continuation, previous.artifact)
        self.assertEqual(prepared.session_id, SESSION_ID)

        self.github.artifacts[300] = previous.artifact
        self.github.artifacts[301] = previous.state.session
        self.github.workflow_runs[PRIOR_RUN] = workflow_run(PRIOR_RUN)
        addressed = commit_file(self.source, "follow-up.txt", "follow-up\n")
        recommendation = CommentRecommendation(
            "Addressed the feedback collected during the previous run",
            tuple(
                CommentAction(
                    revision.target,
                    CommentOutcome.COMMIT_AND_RESOLVE,
                    ""
                    if revision.target.kind == CommentTargetKind.REVIEW_THREAD
                    else "Addressed",
                    addressed,
                )
                for revision in prepared.targets
            ),
        )
        result_directory = self.root / "coalesced-result"
        persist_feedback_result(
            self.source,
            prepared,
            recommendation,
            result_directory,
            source=CURRENT_RUN,
            scratch=self.root,
        )
        publication = prepare_publication(
            self.github,
            self.consumer,
            SCOPE,
            CURRENT_RUN,
            self.base,
            self.root / "coalesced-context",
            result_directory,
            self.session,
            trusted_root=self.root,
            preparation_artifact_id=499,
            result_artifact_id=500,
            session_artifact_id=501,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        )
        state = apply_publication(self.github, self.writer, self.consumer, publication)
        self.assertEqual(state.head, addressed)
        self.assertEqual(self.remote.resolve_commit("refs/heads/feature"), addressed)
        self.github.pull_request = replace(
            self.github.pull_request,
            details=replace(
                self.github.pull_request.details,
                head=replace(self.github.pull_request.details.head, sha=self.head),
            ),
        )
        for retained in (
            None,
            replace(previous, state=feedback_checkpoint(head=self.base)),
        ):
            with (
                self.subTest(retained=retained),
                self.assertRaisesRegex(ValueError, "moved beyond"),
            ):
                prepare_feedback(
                    self.github,
                    self.consumer,
                    SCOPE,
                    CURRENT_RUN,
                    self.base,
                    self.root / "rejected-context",
                    trusted_files=Path(__file__).resolve().parents[3],
                    previous=retained,
                    sessions=sessions,
                    continuations_remaining=MAX_CONTINUATIONS,
                    continuation_key=None,
                )

    def test_no_action_result_advances_checkpoint_without_writes(self) -> None:
        self.source.command(("checkout", "--quiet", "--detach", str(self.base)))
        result_directory = self.root / "result"
        persist_feedback_result(
            self.source,
            self.prepared,
            CommentRecommendation(
                "Nothing new to address",
                tuple(
                    CommentAction(revision.target, CommentOutcome.NO_ACTION, "", None)
                    for revision in self.prepared.targets
                ),
            ),
            result_directory,
            source=CURRENT_RUN,
            scratch=self.root,
        )
        publication = prepare_publication(
            self.github,
            self.consumer,
            SCOPE,
            CURRENT_RUN,
            self.base,
            self.context,
            result_directory,
            self.session,
            trusted_root=self.root,
            preparation_artifact_id=499,
            result_artifact_id=500,
            session_artifact_id=501,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        )
        self.assertFalse(publication.needs_writes)
        state = apply_publication(self.github, self.writer, self.consumer, publication)
        self.assertEqual(state.head, self.base)
        self.assertFalse(
            any("push" in arguments for arguments in self.consumer.commands)
        )
        self.assertEqual(self.github.posts, [])

    def test_commit_trailers_and_rewritten_history_are_rejected(self) -> None:
        bad = commit_file(
            self.source,
            "bad.txt",
            "bad\n",
            message="bad\n\nCo-Authored-By: Someone <person@example.com>",
        )
        with self.assertRaisesRegex(ValueError, "authorship or trailers"):
            verify_commits(self.source, CommitRange(self.base, bad))
        tree = self.source.output("rev-parse", f"{self.base}^{{tree}}")
        orphan = CommitSha(self.source.output("commit-tree", tree, "-m", "orphan"))
        with self.assertRaisesRegex(ValueError, "rewrite existing history"):
            verify_commits(self.source, CommitRange(self.base, orphan))


class CommentWorkflowBoundaryTests(unittest.TestCase):
    def test_workflow_keeps_agent_and_writer_authority_separate(self) -> None:
        root = Path(__file__).resolve().parents[3]
        workflow = (root / WORKFLOW).read_text()
        self.assertNotIn("gh api", workflow)
        self.assertNotIn("jq ", workflow)
        self.assertNotIn("pull_request_target", workflow)
        self.assertLess(workflow.index("concurrency:"), workflow.index("jobs:"))
        handle = workflow.split("  handle:\n", 1)[1].split("  publish:\n", 1)[0]
        self.assertNotIn("id-token: write", handle)
        self.assertNotIn("actions: write", handle)
        self.assertNotIn("contents: write", handle)
        self.assertIn("TMPDIR: ${{ runner.temp }}/comments-agent", handle)
        self.assertIn(
            "CARGO_HOME: ${{ runner.temp }}/comments-agent/cargo-home", handle
        )
        self.assertIn(
            "UV_CACHE_DIR: ${{ runner.temp }}/comments-agent/uv-cache", handle
        )
        self.assertLess(
            handle.index("setup-automations"), handle.index("Install project uv")
        )
        self.assertIn("ref: ${{ needs.prepare.outputs.head-sha }}", handle)
        self.assertIn("-I -m uv_automations comments", handle)
        publish = workflow.split("  publish:\n", 1)[1]
        self.assertIn("id-token: write", publish)
        self.assertIn("GH_READ_TOKEN: ${{ secrets.GITHUB_TOKEN }}", publish)
        self.assertIn('--preparation-artifact "$PREPARATION_ARTIFACT"', publish)
        self.assertIn('--result-artifact "$RESULT_ARTIFACT"', publish)
        self.assertIn("artifact-ids: ${{ needs.prepare.outputs.artifact-id }}", publish)
        self.assertIn(
            "artifact-ids: ${{ needs.handle.outputs.result-artifact-id }}", publish
        )
        self.assertLess(
            publish.index("uv_automations comments validate"),
            publish.index("Get the narrowly scoped uv-dev token"),
        )
        self.assertIn("actions: write", publish)
        self.assertEqual(publish.count("overwrite: true"), 2)
        self.assertEqual(publish.count("archive: true"), 3)
        immutable_index = publish.split(
            'name: "Retain the immutable feedback index"', 1
        )[1].split('name: "Build the per-pull-request discovery aliases"', 1)[0]
        self.assertIn("archive: true", immutable_index)
        self.assertNotIn("overwrite:", immutable_index)
        self.assertIn(
            "INDEX_ARTIFACT: ${{ steps.index-artifact.outputs.artifact-id }}", publish
        )
        self.assertLess(
            publish.index("Retain the completed checkpoint"),
            publish.index("uv_automations comments index"),
        )
        self.assertLess(
            publish.index("uv_automations comments index"),
            publish.index("uv_automations comments index-aliases"),
        )
        self.assertLess(
            publish.index("uv_automations comments index-aliases"),
            publish.index("uv_automations comments continue"),
        )
        self.assertIn("CHECKPOINT_INDEX: ${{ inputs.checkpoint_index }}", workflow)
        self.assertIn("CONTINUATION_KEY: ${{ inputs.continuation_key }}", workflow)

    def test_dispatcher_and_sts_policy_match_the_feature(self) -> None:
        root = Path(__file__).resolve().parents[3]
        dispatch = json.loads((root / ".github/automations-dispatch.json").read_text())
        rules = [
            rule
            for rule in dispatch["rules"]
            if rule["workflow"] == "pull-request-comments.yml"
        ]
        self.assertEqual(
            rules,
            [
                {
                    "repository": "uv-dev",
                    "on": {
                        "issue_comment": ["created", "edited"],
                        "pull_request_review_comment": ["created", "edited"],
                        "pull_request_review": ["submitted", "edited"],
                    },
                    "workflow": "pull-request-comments.yml",
                }
            ],
        )
        policy = json.loads((root / ".github/ost-simple-sts.json").read_text())
        rule = next(
            rule
            for rule in policy["rules"]
            if rule["caller_workflow"] == "pull-request-comments.yml"
        )
        self.assertEqual(rule["caller"], "uv-dev")
        self.assertEqual(rule["target"], "uv-dev")
        self.assertEqual(rule["installation"], "automations")
        self.assertEqual(
            rule["permissions"],
            {
                "actions": "write",
                "contents": "write",
                "workflows": "write",
                "issues": "write",
                "pull_requests": "write",
            },
        )
