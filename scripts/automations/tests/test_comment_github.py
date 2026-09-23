import json
import subprocess
import unittest
from dataclasses import replace
from unittest.mock import patch

from uv_automations.comment_models import (
    MAX_COLLECTION_PAGES,
    MAX_PAGE_SIZE,
    ActorKind,
    CommentScope,
)
from uv_automations.github_actions import (
    ActionsRun,
    decode_artifact,
    decode_workflow_run,
)
from uv_automations.github_comments import CommentGitHub
from uv_automations.json import as_array, as_object
from uv_automations.models import (
    CommitSha,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

REPOSITORY = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
SCOPE = CommentScope(REPOSITORY, 123)
SHA = CommitSha("a" * 40)
SOURCE = ActionsRun(REPOSITORY, 456, 2, SHA)
THREAD_ID = "PRRT_example"
NOW = "2026-09-08T12:00:00Z"


def repository_payload() -> dict[str, object]:
    return {"nameWithOwner": str(REPOSITORY.name), "databaseId": REPOSITORY.database_id}


def pull_request_payload() -> dict[str, object]:
    return {"number": SCOPE.number, "repository": repository_payload()}


def thread_payload() -> dict[str, object]:
    return {
        "id": THREAD_ID,
        "isResolved": False,
        "isOutdated": False,
        "path": "file.py",
        "repository": repository_payload(),
        "pullRequest": pull_request_payload(),
        "comments": {
            "pageInfo": {"hasNextPage": False},
            "nodes": [
                {
                    "fullDatabaseId": "3604896309",
                    "author": {"login": "maintainer", "__typename": "User"},
                    "authorAssociation": "MEMBER",
                    "body": "Please fix this",
                    "updatedAt": NOW,
                }
            ],
        },
    }


def review_payload() -> dict[str, object]:
    return {
        "fullDatabaseId": "4724664307",
        "author": {"login": "maintainer", "__typename": "User"},
        "authorAssociation": "MEMBER",
        "body": "Please simplify",
        "updatedAt": NOW,
        "state": "CHANGES_REQUESTED",
    }


def conversation_payload() -> dict[str, object]:
    return {
        "id": 1,
        "user": {"login": "maintainer", "type": "User"},
        "author_association": "MEMBER",
        "body": "Question",
        "updated_at": NOW,
    }


class CommentGitHubTests(unittest.TestCase):
    def test_enterprise_user_account_is_preserved_as_non_trigger_context(self) -> None:
        payload = thread_payload()
        comment = as_object(as_array(as_object(payload["comments"])["nodes"])[0])
        comment["author"] = {
            "login": "managed-account",
            "__typename": "EnterpriseUserAccount",
        }
        with patch.object(CommentGitHub, "_graphql", return_value={"nodes": [payload]}):
            result = CommentGitHub().get_review_threads(SCOPE, (THREAD_ID,))
        author = result[0].comments[0].author
        self.assertEqual(author.kind, ActorKind.ENTERPRISE_USER_ACCOUNT)
        self.assertFalse(author.can_trigger)

    def test_selected_threads_are_checked_by_direct_node_identity(self) -> None:
        response = {"data": {"nodes": [thread_payload()]}}
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, json.dumps(response))
            result = CommentGitHub().get_review_threads(SCOPE, (THREAD_ID,))
        self.assertEqual(result[0].identifier, THREAD_ID)
        self.assertEqual(result[0].comments[0].identifier, 3604896309)
        arguments = run.call_args.args[0]
        self.assertEqual(
            arguments, ["gh", "api", "--method", "POST", "graphql", "--input", "-"]
        )
        payload = json.loads(run.call_args.kwargs["input"])
        self.assertEqual(payload["variables"], {"identifiers": [THREAD_ID]})
        self.assertIn("pullRequest", payload["query"])
        self.assertIn("fullDatabaseId", payload["query"])

    def test_wrong_repository_or_pull_request_is_rejected(self) -> None:
        foreign = {"nameWithOwner": str(REPOSITORY.name), "databaseId": 1}
        for payload in (
            {**thread_payload(), "repository": foreign},
            {**thread_payload(), "pullRequest": {"number": 123, "repository": foreign}},
            {
                **thread_payload(),
                "pullRequest": {"number": 999, "repository": repository_payload()},
            },
            {**thread_payload(), "id": "PRRT_other"},
        ):
            with (
                self.subTest(payload=payload),
                patch.object(
                    CommentGitHub, "_graphql", return_value={"nodes": [payload]}
                ),
                self.assertRaises(ValueError),
            ):
                CommentGitHub().get_review_threads(SCOPE, (THREAD_ID,))

    def test_truncated_thread_context_is_not_accepted(self) -> None:
        payload = thread_payload()
        payload["comments"] = {"pageInfo": {"hasNextPage": True}, "nodes": []}
        with (
            patch.object(CommentGitHub, "_graphql", return_value={"nodes": [payload]}),
            self.assertRaisesRegex(ValueError, "complete-context limit"),
        ):
            CommentGitHub().get_review_threads(SCOPE, (THREAD_ID,))

    def test_comment_requests_are_incremental_and_bounded(self) -> None:
        since = Timestamp.parse(NOW)
        with patch.object(CommentGitHub, "_api", return_value=[]) as api:
            self.assertEqual(
                CommentGitHub().list_conversation_comments(SCOPE, since), ()
            )
            self.assertEqual(CommentGitHub().list_inline_comments(SCOPE, since), ())
        paths = [call.args[1] for call in api.call_args_list]
        self.assertIn("since=2026-09-08T12%3A00%3A00Z", paths[0])
        self.assertIn("sort=updated&direction=asc", paths[1])
        with (
            patch.object(
                CommentGitHub,
                "_api",
                return_value=[conversation_payload()] * MAX_PAGE_SIZE,
            ) as api,
            self.assertRaisesRegex(ValueError, "bounded collection budget"),
        ):
            CommentGitHub().list_conversation_comments(SCOPE, None)
        self.assertEqual(api.call_count, MAX_COLLECTION_PAGES)

    def test_review_scan_uses_64_bit_ids_and_checks_cursor_progress(self) -> None:
        complete = {
            "repository": {
                "pullRequest": {
                    **pull_request_payload(),
                    "reviews": {
                        "nodes": [review_payload()],
                        "pageInfo": {"hasNextPage": False, "endCursor": "cursor"},
                    },
                }
            }
        }
        with patch.object(CommentGitHub, "_graphql", return_value=complete):
            self.assertEqual(
                CommentGitHub().list_reviews(SCOPE)[0].identifier, 4724664307
            )
        repeated = {
            "repository": {
                "pullRequest": {
                    **pull_request_payload(),
                    "reviews": {
                        "nodes": [],
                        "pageInfo": {"hasNextPage": True, "endCursor": "cursor"},
                    },
                }
            }
        }
        with (
            patch.object(CommentGitHub, "_graphql", return_value=repeated) as graphql,
            self.assertRaisesRegex(ValueError, "non-advancing"),
        ):
            CommentGitHub().list_reviews(SCOPE)
        self.assertEqual(graphql.call_count, 2)

    def test_review_pages_cannot_exceed_or_duplicate_the_requested_results(
        self,
    ) -> None:
        for values in (
            [review_payload()] * (MAX_PAGE_SIZE + 1),
            [review_payload(), review_payload()],
        ):
            response = {
                "repository": {
                    "pullRequest": {
                        **pull_request_payload(),
                        "reviews": {
                            "nodes": values,
                            "pageInfo": {"hasNextPage": False, "endCursor": "cursor"},
                        },
                    }
                }
            }
            with (
                self.subTest(count=len(values)),
                patch.object(CommentGitHub, "_graphql", return_value=response),
                self.assertRaisesRegex(ValueError, "oversized|duplicate"),
            ):
                CommentGitHub().list_reviews(SCOPE)

    def test_conversation_publication_uses_a_json_body(self) -> None:
        body = 'Quotes: "hello" and shell text $(false)'
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, "{}")
            CommentGitHub().post_conversation_comment(SCOPE, body)
        self.assertEqual(
            run.call_args.args[0],
            [
                "gh",
                "api",
                "--method",
                "POST",
                "repos/astral-sh/uv-dev/issues/123/comments",
                "--input",
                "-",
            ],
        )
        self.assertEqual(json.loads(run.call_args.kwargs["input"]), {"body": body})


def workflow_run_payload() -> dict[str, object]:
    repository = {"full_name": str(REPOSITORY.name), "id": REPOSITORY.database_id}
    return {
        "id": SOURCE.identifier,
        "run_attempt": SOURCE.attempt,
        "head_sha": str(SHA),
        "repository": repository,
        "head_repository": repository,
        "path": ".github/workflows/pull-request-comments.yml",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "status": "completed",
        "conclusion": "success",
        "run_started_at": NOW,
    }


def artifact_payload() -> dict[str, object]:
    return {
        "id": 789,
        "name": "expected",
        "expired": False,
        "size_in_bytes": 1234,
        "digest": "sha256:" + "e" * 64,
        "workflow_run": {
            "id": SOURCE.identifier,
            "repository_id": REPOSITORY.database_id,
            "head_repository_id": REPOSITORY.database_id,
            "head_sha": str(SHA),
            "head_branch": "main",
        },
    }


class ActionsIdentityTests(unittest.TestCase):
    def test_retry_attempts_share_only_the_same_immutable_run(self) -> None:
        self.assertTrue(SOURCE.same_run(replace(SOURCE, attempt=3)))
        for other in (
            replace(SOURCE, identifier=789),
            replace(SOURCE, workflow_sha=CommitSha("b" * 40)),
            replace(SOURCE, repository=replace(REPOSITORY, database_id=1)),
        ):
            with self.subTest(other=other):
                self.assertFalse(SOURCE.same_run(other))

    def test_workflow_attempt_and_artifact_identity(self) -> None:
        run = decode_workflow_run(workflow_run_payload())
        self.assertEqual(run.source, SOURCE)
        self.assertTrue(
            run.is_successful_dispatch(".github/workflows/pull-request-comments.yml")
        )
        self.assertFalse(
            replace(run, event="pull_request").is_successful_dispatch(run.path)
        )
        identity = decode_artifact(artifact_payload(), SOURCE, "expected")
        self.assertEqual(identity.identifier, 789)
        self.assertEqual(identity.source.attempt, 2)

    def test_artifact_metadata_preserves_the_caller_supplied_attempt(self) -> None:
        payload = artifact_payload()
        for attempt in (2, 3):
            with self.subTest(attempt=attempt):
                source = replace(SOURCE, attempt=attempt)
                self.assertEqual(
                    decode_artifact(payload, source, "expected").source, source
                )

    def test_artifact_metadata_rejects_a_mismatched_run_identity(self) -> None:
        base = artifact_payload()
        run = base["workflow_run"]
        if not isinstance(run, dict):
            raise TypeError("Expected run payload")
        for payload in (
            {**base, "expired": True},
            {**base, "name": "different"},
            {**base, "digest": None},
            {**base, "workflow_run": {**run, "id": 1}},
            {**base, "workflow_run": {**run, "repository_id": 1}},
            {**base, "workflow_run": {**run, "head_repository_id": 1}},
            {**base, "workflow_run": {**run, "head_sha": "b" * 40}},
            {**base, "workflow_run": {**run, "head_branch": "feature"}},
        ):
            with (
                self.subTest(payload=payload),
                self.assertRaises((ValueError, TypeError)),
            ):
                decode_artifact(payload, SOURCE, "expected")
