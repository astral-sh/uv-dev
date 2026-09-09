import json
import os
import subprocess
import unittest
from unittest.mock import call, patch

from uv_automations.github import GitHub, decode_pull_request_details
from uv_automations.models import (
    CommitSha,
    PullRequestRef,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
)

REPOSITORY = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
REFERENCE = PullRequestRef(REPOSITORY.name, 123)
HEAD = CommitSha("a" * 40)


def pull_request_payload() -> dict[str, object]:
    revision = {
        "repo": {"full_name": str(REPOSITORY.name), "id": REPOSITORY.database_id},
        "ref": "feature",
        "sha": str(HEAD),
    }
    return {
        "number": 123,
        "state": "open",
        "html_url": "https://github.com/astral-sh/uv-dev/pull/123",
        "base": revision,
        "head": revision,
        "labels": [{"name": "bot:rebase"}],
    }


class GitHubTests(unittest.TestCase):
    def test_rest_pull_request_identity(self) -> None:
        pull_request = decode_pull_request_details(pull_request_payload(), REFERENCE)
        self.assertEqual(pull_request.state, PullRequestState.OPEN)
        self.assertEqual(pull_request.head.repository, REPOSITORY)
        self.assertEqual(pull_request.head.sha, HEAD)
        self.assertEqual(pull_request.labels, ("bot:rebase",))

    def test_reject_unexpected_pull_request(self) -> None:
        with self.assertRaises(ValueError):
            decode_pull_request_details(
                {**pull_request_payload(), "number": 456}, REFERENCE
            )

    def test_add_labels_uses_json_stdin(self) -> None:
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, "[]")
            GitHub().add_labels(REFERENCE, ("bug", "testing"))
            run.assert_called_once_with(
                [
                    "gh",
                    "api",
                    "--method",
                    "POST",
                    "repos/astral-sh/uv-dev/issues/123/labels",
                    "--input",
                    "-",
                ],
                input=json.dumps({"labels": ("bug", "testing")}, allow_nan=False),
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                env=None,
                timeout=60,
            )

    def test_remove_label_encodes_path(self) -> None:
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, "")
            GitHub().remove_label(REFERENCE, "bot:rebase")
            run.assert_called_once_with(
                [
                    "gh",
                    "api",
                    "--method",
                    "DELETE",
                    "repos/astral-sh/uv-dev/issues/123/labels/bot%3Arebase",
                ],
                input=None,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                env=None,
                timeout=60,
            )

    def test_read_token_is_scoped_to_the_github_subprocess(self) -> None:
        with (
            patch.dict(os.environ, {"GH_TOKEN": "writer", "GH_READ_TOKEN": "reader"}),
            patch("uv_automations.github.subprocess.run") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                [], 0, json.dumps(pull_request_payload())
            )
            GitHub(token_variable="GH_READ_TOKEN").get_pull_request(REFERENCE)
            self.assertEqual(run.call_args.kwargs["env"]["GH_TOKEN"], "reader")
            self.assertEqual(os.environ["GH_TOKEN"], "writer")
            self.assertNotIn("reader", run.call_args.args[0])
            self.assertNotIn("writer", run.call_args.args[0])

    def test_missing_read_token_fails_closed(self) -> None:
        with (
            patch.dict(os.environ, {}, clear=True),
            patch("uv_automations.github.subprocess.run") as run,
            self.assertRaisesRegex(ValueError, "GH_READ_TOKEN"),
        ):
            GitHub(token_variable="GH_READ_TOKEN").get_pull_request(REFERENCE)
        run.assert_not_called()

    def test_close_pull_request_uses_fixed_json_endpoints(self) -> None:
        with patch("uv_automations.github.GitHub._api") as api:
            GitHub().close_pull_request(REFERENCE, comment="Already merged.")
        self.assertEqual(
            api.call_args_list,
            [
                call(
                    "POST",
                    "repos/astral-sh/uv-dev/issues/123/comments",
                    payload={"body": "Already merged."},
                ),
                call(
                    "PATCH",
                    "repos/astral-sh/uv-dev/pulls/123",
                    payload={"state": "closed"},
                ),
            ],
        )
