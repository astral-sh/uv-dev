"""Exercise issue-context publication without writing to GitHub."""

import argparse
import copy
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

import issue_context

REPOSITORY = "astral-sh/uv"
ISSUE_NUMBER = 123
ENDPOINT = f"repos/{REPOSITORY}/issues/{ISSUE_NUMBER}/artifacts"
ISSUE = {
    "number": ISSUE_NUMBER,
    "title": "A reported issue",
    "url": f"https://github.com/{REPOSITORY}/issues/{ISSUE_NUMBER}",
}


class IssueContextTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.outputs = self.directory / "outputs"
        self.enterContext(
            patch.dict(
                os.environ,
                {
                    "RUNNER_TEMP": str(self.directory),
                    "GITHUB_WORKSPACE": str(self.directory),
                    "GITHUB_OUTPUT": str(self.outputs),
                    "GITHUB_STEP_SUMMARY": str(self.directory / "summary"),
                    "GIT_CONFIG_GLOBAL": os.devnull,
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )
        )
        self.artifacts = []
        self.versions = {}
        self.legacy_repository = self.directory / "legacy"
        issue_context.git(
            "init", "--quiet", "--initial-branch=issue/123", str(self.legacy_repository)
        )
        self.enterContext(
            patch.object(
                issue_context, "LEGACY_REPOSITORY", str(self.legacy_repository)
            )
        )
        self.writes = []
        self.enterContext(patch.object(issue_context, "api", side_effect=self.api))
        self.readme = self.directory / "README.md"
        self.readme.write_text("# Findings\n\nThe issue is reproducible.\n")
        self.issue_file = self.directory / "issue.json"
        self.issue_file.write_text(json.dumps(ISSUE))

    def api(self, endpoint, *, method="GET", payload=None):
        if method == "POST":
            self.writes.append((method, endpoint, payload))
            return self.add_artifact(payload["body"])
        if method == "PATCH":
            self.writes.append((method, endpoint, payload))
            artifact = next(
                item
                for item in self.artifacts
                if endpoint.endswith(f"/{item['number']}")
            )
            artifact.update(payload)
            self.versions[artifact["number"]] += 1
            return copy.deepcopy(artifact)
        if endpoint.startswith(f"{ENDPOINT}?"):
            query = parse_qs(urlsplit(endpoint).query)
            page = int(query["page"][0])
            return copy.deepcopy(self.artifacts[(page - 1) * 100 : page * 100])
        if endpoint.startswith(f"{ENDPOINT}/"):
            number = int(endpoint.rsplit("/", 1)[1])
            artifact = next(item for item in self.artifacts if item["number"] == number)
            return {
                "artifact": copy.deepcopy(artifact),
                "versions": [{"version": self.versions[number]}],
            }
        self.fail(f"Unexpected API request: {method} {endpoint}")

    def add_artifact(self, body, *, name=issue_context.ARTIFACT_NAME, creator=None):
        number = len(self.artifacts) + 1
        artifact = {
            "id": number + 1000,
            "number": number,
            "name": name,
            "type": "generic",
            "creator": {"login": creator or issue_context.ARTIFACT_CREATOR},
            "body": body,
        }
        self.artifacts.append(artifact)
        self.versions[number] = 1
        return copy.deepcopy(artifact)

    def save_args(self, revision):
        return argparse.Namespace(
            repository=REPOSITORY,
            issue_number=ISSUE_NUMBER,
            expected_revision=revision,
            issue_file=str(self.issue_file),
            result_file=None,
            result_name=None,
            readme=str(self.readme),
        )

    def load_args(self):
        return argparse.Namespace(
            repository=REPOSITORY,
            issue_number=ISSUE_NUMBER,
            path=str(self.directory / "context"),
            template=None,
            issue_file=None,
            allow_missing=False,
        )

    def test_new_report_is_created_and_loaded_as_local_context(self):
        missing = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        issue_context.save(self.save_args(missing.revision))
        issue_context.load(self.load_args())
        context = self.directory / "context"
        self.assertEqual((context / "README.md").read_text(), self.readme.read_text())
        self.assertEqual(json.loads((context / "issue.json").read_text()), ISSUE)
        status = subprocess.check_output(
            ["git", "-C", str(context), "status", "--porcelain"]
        )
        self.assertEqual(status, b"")
        self.assertEqual([method for method, _, _ in self.writes], ["POST"])

    def test_legacy_report_migrates_with_all_structured_results(self):
        legacy_files = {
            "README.md": "# Earlier investigation\n",
            "issue.json": json.dumps(ISSUE),
            "triage.json": '{"type": "bug"}',
            "reproduction.json": '{"reproduction": "reproducible"}',
            "fix.json": '{"outcome": "not_fixed"}',
        }
        for name, content in legacy_files.items():
            (self.legacy_repository / name).write_text(content)
        issue_context.git("add", ".", directory=self.legacy_repository)
        issue_context.git(
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--quiet",
            "--message",
            "Existing issue report",
            directory=self.legacy_repository,
        )
        legacy = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        issue_context.save(self.save_args(legacy.revision))
        migrated = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.assertEqual(migrated.data, legacy.data)
        self.assertEqual(migrated.readme, self.readme.read_text())
        self.assertEqual([method for method, _, _ in self.writes], ["POST"])

    def test_reproduction_update_preserves_triage_and_fix(self):
        data = {
            "repository": REPOSITORY,
            "issue": ISSUE,
            "triage": {"type": "bug"},
            "fix": {"outcome": "not_fixed"},
        }
        self.add_artifact(issue_context.encode("# Earlier investigation\n", data))
        context = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        result = self.directory / "reproduction.json"
        result.write_text('{"reproduction": "reproducible"}')
        args = self.save_args(context.revision)
        args.result_name = "reproduction"
        args.result_file = str(result)
        issue_context.save(args)
        updated = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.assertEqual(updated.data["triage"], data["triage"])
        self.assertEqual(updated.data["fix"], data["fix"])
        self.assertEqual(updated.data["reproduction"], {"reproduction": "reproducible"})
        self.assertEqual([method for method, _, _ in self.writes], ["PATCH"])

    def test_legacy_report_without_readme_can_be_initialized_and_migrated(self):
        """The first context writer stored JSON results without a Markdown report."""
        (self.legacy_repository / "issue.json").write_text(json.dumps(ISSUE))
        (self.legacy_repository / "triage.json").write_text('{"type": "question"}')
        issue_context.git("add", ".", directory=self.legacy_repository)
        issue_context.git(
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--quiet",
            "--message",
            "Existing JSON context",
            directory=self.legacy_repository,
        )
        legacy = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        updated_issue = {**ISSUE, "title": "Updated issue title"}
        self.issue_file.write_text(json.dumps(updated_issue))
        args = self.load_args()
        args.template = str(self.readme)
        args.issue_file = str(self.issue_file)
        issue_context.load(args)
        context = Path(args.path)
        self.assertEqual((context / "README.md").read_text(), self.readme.read_text())
        self.assertEqual(
            json.loads((context / "issue.json").read_text()), updated_issue
        )
        self.assertEqual(
            json.loads((context / "triage.json").read_text()), {"type": "question"}
        )
        issue_context.save(self.save_args(legacy.revision))
        migrated = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.assertEqual(migrated.readme, self.readme.read_text())
        self.assertEqual(migrated.data["triage"], {"type": "question"})
        self.assertEqual(migrated.data["issue"], updated_issue)

    def test_comment_updates_skip_legacy_reports_without_readme(self):
        (self.legacy_repository / "issue.json").write_text(json.dumps(ISSUE))
        issue_context.git("add", ".", directory=self.legacy_repository)
        issue_context.git(
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--quiet",
            "--message",
            "Existing JSON context",
            directory=self.legacy_repository,
        )
        args = self.load_args()
        args.allow_missing = True
        issue_context.load(args)
        self.assertFalse(Path(args.path).exists())
        self.assertEqual(self.outputs.read_text(), "exists=false\n")
        self.assertEqual(self.writes, [])

    def test_stale_analysis_cannot_overwrite_a_newer_version(self):
        data = {"repository": REPOSITORY, "issue": ISSUE}
        self.add_artifact(issue_context.encode("# Original\n", data))
        original = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.artifacts[0]["body"] = issue_context.encode("# Newer findings\n", data)
        self.versions[1] = 2
        issue_context.save(self.save_args(original.revision))
        self.assertEqual(self.writes, [])
        self.assertEqual(self.outputs.read_text(), "current=false\nchanged=false\n")

    def test_restoring_identical_content_still_invalidates_a_stale_analysis(self):
        data = {"repository": REPOSITORY, "issue": ISSUE}
        self.add_artifact(issue_context.encode("# Original\n", data))
        original = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.versions[1] = 3
        issue_context.save(self.save_args(original.revision))
        self.assertEqual(self.writes, [])

    def test_concurrent_initialization_does_not_create_a_second_artifact(self):
        missing = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.add_artifact(
            issue_context.encode(
                "# Another run\n", {"repository": REPOSITORY, "issue": ISSUE}
            )
        )
        issue_context.save(self.save_args(missing.revision))
        self.assertEqual(len(self.artifacts), 1)
        self.assertEqual(self.writes, [])

    def test_unchanged_report_does_not_create_another_version(self):
        data = {"repository": REPOSITORY, "issue": ISSUE}
        self.add_artifact(issue_context.encode(self.readme.read_text(), data))
        context = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        issue_context.save(self.save_args(context.revision))
        self.assertEqual(self.writes, [])
        self.assertIn("current=true\nchanged=false\n", self.outputs.read_text())

    def test_artifact_is_found_after_a_full_page(self):
        for number in range(100):
            self.add_artifact("Unrelated document", name=f"Reference {number}")
        data = {"repository": REPOSITORY, "issue": ISSUE}
        self.add_artifact(issue_context.encode("# Findings\n", data))
        context = issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.assertEqual(context.artifact_number, 101)

    def test_report_for_another_issue_is_rejected(self):
        data = {"repository": REPOSITORY, "issue": {**ISSUE, "number": 456}}
        self.add_artifact(issue_context.encode("# Wrong issue\n", data))
        with self.assertRaisesRegex(ValueError, "does not match"):
            issue_context.read_context(REPOSITORY, ISSUE_NUMBER)

    def test_human_artifact_with_same_name_is_not_overwritten(self):
        self.add_artifact("A maintainer's report", creator="maintainer")
        with self.assertRaisesRegex(ValueError, "not owned"):
            issue_context.read_context(REPOSITORY, ISSUE_NUMBER)
        self.assertEqual(self.writes, [])

    def test_duplicate_reports_are_rejected(self):
        self.add_artifact("First report")
        self.add_artifact("Second report")
        with self.assertRaisesRegex(ValueError, "more than one"):
            issue_context.read_context(REPOSITORY, ISSUE_NUMBER)

    def test_missing_report_can_be_skipped_by_comment_updates(self):
        args = self.load_args()
        args.allow_missing = True
        issue_context.load(args)
        self.assertFalse(Path(args.path).exists())
        self.assertEqual(self.outputs.read_text(), "exists=false\n")

    def test_new_report_can_start_from_the_template_without_publishing(self):
        args = self.load_args()
        args.template = str(self.readme)
        args.issue_file = str(self.issue_file)
        issue_context.load(args)
        self.assertEqual(
            (Path(args.path) / "README.md").read_text(), self.readme.read_text()
        )
        self.assertEqual(self.writes, [])


if __name__ == "__main__":
    unittest.main()
