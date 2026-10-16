import importlib.util
import unittest
from pathlib import Path
from unittest.mock import call, patch

spec = importlib.util.spec_from_file_location(
    "pull_request_comment", Path(__file__).parents[1] / "pull-request-comment.py"
)
assert spec is not None and spec.loader is not None
comments = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comments)


class PullRequestCommentTests(unittest.TestCase):
    def setUp(self):
        self.pull = {
            "state": "open",
            "base": {"repo": {"full_name": "astral-sh/uv-dev"}},
            "head": {"repo": {"full_name": "astral-sh/uv-dev"}},
        }
        self.reserved = {
            "id": 123,
            "user": {"id": 305554984, "type": "Bot"},
            "body": "<!-- uv-automation-comment -->\nReserved for automation updates.",
        }
        self.other = {
            "id": 100,
            "user": {"id": 42, "type": "User"},
            "body": "A maintainer's comment.",
        }

    def test_find_reads_only_the_first_comment(self):
        with patch.object(comments, "api", return_value=[self.reserved]) as api:
            self.assertEqual(comments.find_comment(7), self.reserved)
        api.assert_called_once_with(
            "repos/astral-sh/uv-dev/issues/7/comments?per_page=1"
        )

    def test_find_does_not_scan_past_an_unrelated_first_comment(self):
        with patch.object(comments, "api", return_value=[self.other]) as api:
            self.assertIsNone(comments.find_comment(7))
        self.assertEqual(api.call_count, 1)

    def test_find_rejects_a_copied_marker(self):
        self.other["body"] = self.reserved["body"]
        self.other["user"]["type"] = "Bot"
        with patch.object(comments, "api", return_value=[self.other]):
            self.assertIsNone(comments.find_comment(7))

    def test_find_requires_a_bot_author(self):
        self.reserved["user"]["type"] = "User"
        with patch.object(comments, "api", return_value=[self.reserved]):
            self.assertIsNone(comments.find_comment(7))

    def test_find_rejects_a_marker_in_another_comment(self):
        self.reserved["body"] = f"Quoted example:\n{self.reserved['body']}"
        with patch.object(comments, "api", return_value=[self.reserved]):
            self.assertIsNone(comments.find_comment(7))

    def test_reserve_creates_the_first_comment(self):
        with patch.object(
            comments, "api", side_effect=[self.pull, [], self.reserved, [self.reserved]]
        ) as api:
            self.assertEqual(comments.reserve_comment(7), self.reserved)
        self.assertEqual(
            api.call_args_list,
            [
                call("repos/astral-sh/uv-dev/pulls/7"),
                call("repos/astral-sh/uv-dev/issues/7/comments?per_page=1"),
                call(
                    "repos/astral-sh/uv-dev/issues/7/comments",
                    body=self.reserved["body"],
                ),
                call("repos/astral-sh/uv-dev/issues/7/comments?per_page=1"),
            ],
        )

    def test_reserve_keeps_existing_automation_updates(self):
        self.reserved["body"] = "<!-- uv-automation-comment -->\nCI report."
        with patch.object(
            comments, "api", side_effect=[self.pull, [self.reserved]]
        ) as api:
            self.assertEqual(comments.reserve_comment(7), self.reserved)
        self.assertEqual(api.call_count, 2)

    def test_reserve_does_not_write_when_the_first_comment_is_occupied(self):
        with patch.object(
            comments, "api", side_effect=[self.pull, [self.other]]
        ) as api:
            self.assertIsNone(comments.reserve_comment(7))
        self.assertEqual(api.call_count, 2)

    def test_reserve_does_not_return_a_reservation_that_lost_the_race(self):
        with patch.object(
            comments, "api", side_effect=[self.pull, [], self.reserved, [self.other]]
        ) as api:
            self.assertIsNone(comments.reserve_comment(7))
        self.assertEqual(api.call_count, 4)
        self.assertEqual(api.call_args_list[-1].kwargs, {})

    def test_reserve_skips_closed_pull_requests(self):
        self.pull["state"] = "closed"
        with patch.object(comments, "api", return_value=self.pull) as api:
            self.assertIsNone(comments.reserve_comment(7))
        api.assert_called_once_with("repos/astral-sh/uv-dev/pulls/7")

    def test_reserve_skips_forks(self):
        self.pull["head"]["repo"] = {"full_name": "someone/uv-dev"}
        with patch.object(comments, "api", return_value=self.pull) as api:
            self.assertIsNone(comments.reserve_comment(7))
        api.assert_called_once_with("repos/astral-sh/uv-dev/pulls/7")

    def test_reserve_skips_deleted_head_repositories(self):
        self.pull["head"]["repo"] = None
        with patch.object(comments, "api", return_value=self.pull) as api:
            self.assertIsNone(comments.reserve_comment(7))
        api.assert_called_once_with("repos/astral-sh/uv-dev/pulls/7")

    def test_api_serializes_the_placeholder_as_json(self):
        with patch.object(comments.subprocess, "run") as run:
            run.return_value.stdout = "{}"
            comments.api("repos/astral-sh/uv-dev/issues/7/comments", body="A\nB")
        self.assertEqual(run.call_args.kwargs["input"], '{"body": "A\\nB"}')
        self.assertEqual(run.call_args.args[0][-2:], ["--input", "-"])

    def test_api_failures_do_not_create_a_comment(self):
        error = comments.subprocess.CalledProcessError(1, "gh")
        with (
            patch.object(comments, "api", side_effect=[self.pull, error]),
            self.assertRaises(comments.subprocess.CalledProcessError),
        ):
            comments.reserve_comment(7)


if __name__ == "__main__":
    unittest.main()
