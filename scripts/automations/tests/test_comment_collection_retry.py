import json
import subprocess
import unittest
from dataclasses import dataclass, field, replace
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from test_comments import (
    CURRENT_RUN,
    EARLIER,
    LATER,
    MAX_CONTINUATIONS,
    PRIOR_RUN,
    SCOPE,
    SESSION_ID,
    FakeGitHub,
    LocalRemoteGit,
    artifact,
    collection_checkpoint,
    commit_file,
    conversation,
    create_repository,
    feedback_checkpoint,
    pull_request,
    write_session,
)

from uv_automations.comment_models import (
    CommentScope,
    ConversationComment,
    InlineComment,
)
from uv_automations.github_comments import CommentGitHub
from uv_automations.models import Timestamp
from uv_automations.sessions import snapshot_sessions
from uv_automations.workflows.comments import (
    FeedbackArtifactKind,
    PreparedFeedback,
    RetainedFeedback,
    prepare_feedback,
)


def read_failure() -> subprocess.CalledProcessError:
    return subprocess.CalledProcessError(
        1, ["gh", "api", "--method", "GET", "comments"], stderr="HTTP 503"
    )


@dataclass
class FailingCollectionGitHub(FakeGitHub):
    failures: list[Exception] = field(default_factory=list)
    collection_reads: list[Timestamp | None] = field(default_factory=list)

    @override
    def list_inline_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[InlineComment, ...]:
        self.collection_reads.append(since)
        if self.failures:
            raise self.failures.pop(0)
        return super().list_inline_comments(scope, since)


@dataclass
class RemovedCommentGitHub(FailingCollectionGitHub):
    missing_identifier: int = 3
    response_status: int = 404
    retained_reads: list[int] = field(default_factory=list)

    @override
    def get_conversation_comment(
        self, scope: CommentScope, identifier: int
    ) -> ConversationComment:
        if identifier == self.missing_identifier:
            raise subprocess.CalledProcessError(
                1,
                [
                    "gh",
                    "api",
                    "--method",
                    "GET",
                    f"repos/{scope.repository.name}/issues/comments/{identifier}",
                ],
                stderr=f"HTTP {self.response_status}",
            )
        return super().get_conversation_comment(scope, identifier)

    @override
    def find_retained_conversation_comment(
        self, scope: CommentScope, identifier: int
    ) -> ConversationComment | None:
        self.retained_reads.append(identifier)
        if identifier != self.missing_identifier:
            return super().find_retained_conversation_comment(scope, identifier)
        response = subprocess.CompletedProcess(
            [],
            1,
            f'HTTP/2.0 {self.response_status}\r\nContent-Type: application/json\r\n\r\n{{"message":"Not Found"}}',
            f"HTTP {self.response_status}",
        )
        with (
            patch(
                "uv_automations.github_comments.subprocess.run", return_value=response
            ),
            patch.object(
                CommentGitHub,
                "get_comment_pull_request",
                side_effect=self.get_comment_pull_request,
            ),
        ):
            return CommentGitHub().find_retained_conversation_comment(scope, identifier)


class CommentCollectionRetryTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        directory = TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        source = create_repository(self.root / "source")
        self.base = commit_file(source, "base.txt", "base\n")
        remote = create_repository(self.root / "remote", bare=True)
        source.command(
            ("push", "--quiet", str(remote.path), f"{self.base}:refs/heads/feature")
        )
        current = pull_request(self.base)
        current = replace(
            current,
            details=replace(
                current.details, base=replace(current.details.base, sha=self.base)
            ),
        )
        self.old = conversation(1, updated_at=EARLIER)
        self.new = conversation(2, updated_at=LATER)
        self.github = FailingCollectionGitHub(
            pull_request=current, conversation=(self.old, self.new)
        )
        consumer = create_repository(self.root / "consumer")
        consumer.command(("fetch", "--quiet", str(remote.path), "refs/heads/feature"))
        consumer.command(("checkout", "--quiet", "--detach", str(self.base)))
        self.consumer = LocalRemoteGit(consumer.path, remote.path, self.github)
        session = self.root / "sessions"
        write_session(session, self.consumer.path)
        self.sessions = snapshot_sessions(
            session, self.consumer.path, trusted_root=self.root
        )
        self.previous = RetainedFeedback(
            artifact(FeedbackArtifactKind.STATE, PRIOR_RUN, 300),
            feedback_checkpoint(
                head=self.base,
                collection=replace(
                    collection_checkpoint(), comments=(self.old.version,)
                ),
            ),
        )
        self.destination = self.root / "context"

    def prepare(self, *, previous: RetainedFeedback | None = None) -> PreparedFeedback:
        with patch.object(Timestamp, "now", return_value=LATER):
            return prepare_feedback(
                self.github,
                self.consumer,
                SCOPE,
                CURRENT_RUN,
                self.base,
                self.destination,
                trusted_files=Path(__file__).resolve().parents[3],
                previous=previous,
                sessions=self.sessions if previous is not None else None,
                continuations_remaining=MAX_CONTINUATIONS,
                continuation_key=None,
            )

    def test_transient_collection_failure_retains_checkpoint_and_watermark(
        self,
    ) -> None:
        self.github.failures.append(read_failure())
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare(previous=self.previous)
        self.assertEqual(prepared.after, EARLIER)
        self.assertEqual(prepared.collection.through, LATER)
        self.assertEqual(prepared.targets, (self.new.revision,))
        self.assertEqual(prepared.session_id, SESSION_ID)
        self.assertEqual(self.github.collection_reads, [EARLIER.overlap()] * 2)
        self.assertEqual([call.args for call in clock.sleep.call_args_list], [(5,)])
        self.assertEqual(
            json.loads((self.destination / "prepared.json").read_text())["after"],
            str(EARLIER),
        )

    def test_bootstrap_collection_uses_the_same_bounded_read_retry(self) -> None:
        self.github.failures.extend((read_failure(), read_failure()))
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare()
        self.assertIsNone(prepared.after)
        self.assertEqual(prepared.targets, (self.old.revision, self.new.revision))
        self.assertEqual(self.github.collection_reads, [None] * 3)
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )

    def test_exhausted_checkpoint_reads_keep_the_complete_history_fallback(
        self,
    ) -> None:
        self.github.failures.extend(read_failure() for _ in range(3))
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare(previous=self.previous)
        self.assertIsNone(prepared.after)
        self.assertEqual(prepared.targets, (self.old.revision, self.new.revision))
        self.assertEqual(self.github.collection_reads, [EARLIER.overlap()] * 3 + [None])
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )

    def test_unusable_cursor_falls_back_without_transport_retries(self) -> None:
        self.github.failures.append(ValueError("Expired GraphQL cursor"))
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare(previous=self.previous)
        self.assertIsNone(prepared.after)
        self.assertEqual(self.github.collection_reads, [EARLIER.overlap(), None])
        clock.sleep.assert_not_called()

    def test_failed_bootstrap_does_not_create_context(self) -> None:
        failure = read_failure()
        self.github.failures.extend((failure, failure, failure))
        with (
            patch("uv_automations.workflows.comments.time", create=True) as clock,
            self.assertRaises(subprocess.CalledProcessError) as caught,
        ):
            self.prepare()
        self.assertIs(caught.exception, failure)
        self.assertEqual(self.github.collection_reads, [None] * 3)
        self.assertFalse(self.destination.exists())
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )

    def test_unfinished_or_unstarted_reads_are_not_retried(self) -> None:
        for failure in (
            subprocess.TimeoutExpired(["gh", "api"], 60),
            OSError("Could not start gh"),
        ):
            with self.subTest(failure=type(failure).__name__):
                self.github.failures.append(failure)
                self.github.collection_reads.clear()
                with (
                    patch(
                        "uv_automations.workflows.comments.time", create=True
                    ) as clock,
                    self.assertRaises(type(failure)) as caught,
                ):
                    self.prepare(previous=self.previous)
                self.assertIs(caught.exception, failure)
                self.assertEqual(self.github.collection_reads, [EARLIER.overlap()])
                self.assertFalse(self.destination.exists())
                clock.sleep.assert_not_called()

    def test_context_creation_is_outside_the_read_retry(self) -> None:
        self.destination.mkdir()
        with (
            patch("uv_automations.workflows.comments.time", create=True) as clock,
            self.assertRaises(FileExistsError),
        ):
            self.prepare(previous=self.previous)
        self.assertEqual(self.github.collection_reads, [EARLIER.overlap()])
        self.assertEqual(tuple(self.destination.iterdir()), ())
        clock.sleep.assert_not_called()

    def test_deleted_pending_comment_keeps_checkpoint_and_progress(self) -> None:
        github = RemovedCommentGitHub(
            pull_request=self.github.pull_request,
            conversation=self.github.conversation,
        )
        self.github = github
        self.previous = replace(
            self.previous,
            state=replace(
                self.previous.state,
                collection=replace(
                    self.previous.state.collection,
                    pending=(conversation(3, updated_at=EARLIER).revision,),
                ),
            ),
        )
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare(previous=self.previous)
        self.assertEqual(prepared.after, EARLIER)
        self.assertEqual(prepared.targets, (self.new.revision,))
        self.assertEqual(prepared.collection.pending, ())
        self.assertEqual(prepared.processed_targets, 2)
        self.assertEqual(prepared.session_id, SESSION_ID)
        self.assertEqual(github.collection_reads, [EARLIER.overlap()])
        self.assertEqual(github.retained_reads, [3])
        clock.sleep.assert_not_called()

    def test_other_pending_comment_failures_keep_bounded_retry_and_fallback(
        self,
    ) -> None:
        github = RemovedCommentGitHub(
            pull_request=self.github.pull_request,
            conversation=self.github.conversation,
            response_status=503,
        )
        self.github = github
        self.previous = replace(
            self.previous,
            state=replace(
                self.previous.state,
                collection=replace(
                    self.previous.state.collection,
                    pending=(conversation(3, updated_at=EARLIER).revision,),
                ),
            ),
        )
        with patch("uv_automations.workflows.comments.time", create=True) as clock:
            prepared = self.prepare(previous=self.previous)
        self.assertIsNone(prepared.after)
        self.assertEqual(prepared.targets, (self.old.revision, self.new.revision))
        self.assertEqual(github.retained_reads, [3, 3, 3])
        self.assertEqual(github.collection_reads, [EARLIER.overlap()] * 3 + [None])
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )


if __name__ == "__main__":
    unittest.main()
