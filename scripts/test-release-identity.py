"""Exercise release identity checks with offline GitHub responses."""

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "release_identity", Path(__file__).with_name("check-release-identity.py")
)
assert SPEC is not None and SPEC.loader is not None
IDENTITY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IDENTITY)
COMMIT = "a" * 40
REPOSITORY = "astral-sh/uv"
TAG = "0.12.23"


def run(**changes):
    return {
        "path": ".github/workflows/release.yml",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": COMMIT,
        "status": "completed",
        "repository": {"full_name": REPOSITORY},
    } | changes


def reference(kind="Commit", oid=COMMIT):
    return {
        "data": {"repository": {"ref": {"target": {"__typename": kind, "oid": oid}}}}
    }


class ReleaseIdentity(unittest.TestCase):
    def setUp(self):
        self.github_patch = patch.object(IDENTITY, "github")
        self.github = self.github_patch.start()
        self.addCleanup(self.github_patch.stop)

    def test_run_supplies_the_source_commit(self):
        self.github.return_value = run()
        self.assertEqual(IDENTITY.check_identity(REPOSITORY, None, 123, None), COMMIT)
        self.github.assert_called_once_with(f"repos/{REPOSITORY}/actions/runs/123")

    def test_matching_explicit_commit_is_accepted(self):
        self.github.return_value = run()
        self.assertEqual(
            IDENTITY.check_identity(REPOSITORY, COMMIT.upper(), 123, None), COMMIT
        )

    def test_mismatched_run_commit_is_rejected_before_tag_lookup(self):
        self.github.return_value = run()
        with self.assertRaisesRegex(ValueError, "differs from release run"):
            IDENTITY.check_identity(REPOSITORY, "b" * 40, 123, TAG)
        self.assertEqual(self.github.call_count, 1)

    def test_recovery_requires_the_expected_completed_workflow(self):
        for changes in (
            {"path": ".github/workflows/ci.yml"},
            {"event": "pull_request"},
            {"head_branch": "other"},
            {"status": "in_progress"},
            {"repository": {"full_name": "other/repo"}},
        ):
            with self.subTest(changes=changes):
                self.github.return_value = run(**changes)
                with self.assertRaisesRegex(ValueError, "completed release dispatch"):
                    IDENTITY.check_identity(REPOSITORY, None, 123, None)

    def test_absent_and_matching_lightweight_tags_are_accepted(self):
        for data in ({"data": {"repository": {"ref": None}}}, reference()):
            with self.subTest(data=data):
                self.github.return_value = data
                self.assertEqual(
                    IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG), COMMIT
                )

    def test_mismatched_existing_tag_is_rejected(self):
        self.github.return_value = reference(oid="b" * 40)
        with self.assertRaisesRegex(ValueError, "points to .*expected"):
            IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG)

    def test_nested_annotated_tags_are_peeled(self):
        self.github.side_effect = [
            reference("Tag", "b" * 40),
            {"object": {"type": "tag", "sha": "c" * 40}},
            {"object": {"type": "commit", "sha": COMMIT}},
        ]
        self.assertEqual(IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG), COMMIT)
        self.assertEqual(
            self.github.call_args.args, (f"repos/{REPOSITORY}/git/tags/{'c' * 40}",)
        )

    def test_noncommit_tag_is_rejected(self):
        self.github.return_value = reference("Blob")
        with self.assertRaisesRegex(ValueError, "does not point to a commit"):
            IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG)

    def test_tag_peeling_has_a_request_bound(self):
        self.github.side_effect = [
            reference("Tag"),
            *[{"object": {"type": "tag", "sha": COMMIT}} for _ in range(16)],
        ]
        with self.assertRaisesRegex(ValueError, "too many nested"):
            IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG)
        self.assertEqual(self.github.call_count, 17)

    def test_unreadable_repository_is_not_an_absent_tag(self):
        for data in (
            {"data": {"repository": None}},
            {"errors": [{"message": "fixture"}]},
        ):
            with self.subTest(data=data):
                self.github.return_value = data
                with self.assertRaisesRegex(ValueError, "Could not read"):
                    IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG)

    def test_api_failures_propagate(self):
        self.github.side_effect = subprocess.CalledProcessError(1, "gh api")
        with self.assertRaises(subprocess.CalledProcessError):
            IDENTITY.check_identity(REPOSITORY, COMMIT, None, TAG)

    def test_invalid_inputs_are_rejected_before_requests(self):
        for commit, run_id in (("main", None), (None, 0), (None, None)):
            with (
                self.subTest(commit=commit, run_id=run_id),
                self.assertRaises(ValueError),
            ):
                IDENTITY.check_identity(REPOSITORY, commit, run_id, TAG)
        self.github.assert_not_called()


if __name__ == "__main__":
    unittest.main()
