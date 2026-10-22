"""Tests for the test inventory publisher."""

import copy
import io
import json
import subprocess
import unittest
import zipfile
from unittest.mock import patch

import publish_test_inventory as publisher

REPOSITORY = "astral-sh/uv-dev"
REPOSITORY_ID = publisher.REPOSITORIES[REPOSITORY]
HEAD = "a" * 40
BASE = "b" * 40
MERGE = "c" * 40
RUN = {
    "id": 200,
    "run_attempt": 2,
    "repository": {"id": REPOSITORY_ID},
    "head_repository": {"id": 123, "owner": {"login": "contributor"}},
    "event": "pull_request",
    "path": ".github/workflows/ci.yml",
    "head_sha": HEAD,
    "head_branch": "change",
    "status": "completed",
    "conclusion": "success",
    "pull_requests": [{"number": 42}],
}
PULL = {
    "number": 42,
    "state": "open",
    "base": {"repo": {"id": REPOSITORY_ID}, "ref": "main", "sha": "d" * 40},
    "head": {"repo": {"id": 123}, "ref": "change", "sha": HEAD},
    "merge_commit_sha": MERGE,
}
REPORT = {
    "pull_request": 42,
    "merge_sha": MERGE,
    "changed": True,
    "body": "## uv test inventory changes\n\nAdded tests: **1**",
}
COMMENT = {
    "id": 456,
    "user": {"id": publisher.BOT_ID},
    "issue_url": f"https://api.github.com/repos/{REPOSITORY}/issues/42",
    "body": f"{publisher.MARKER}\n<!-- report-run:199:1 -->\n\nPrevious report",
}


def archive(members):
    contents = io.BytesIO()
    with zipfile.ZipFile(contents, "w") as zipped:
        for name, value in members:
            zipped.writestr(name, value)
    return zipfile.ZipFile(io.BytesIO(contents.getvalue()))


class InventoryTests(unittest.TestCase):
    def test_deduplicates_test_ids_across_suites_and_shards(self):
        xml = b"""<testsuites>
          <testsuite name="suite"><testcase classname="module" name="same" />
            <testcase name="unqualified" /></testsuite>
          <testsuite name="suite"><testcase classname="module" name="same" />
            <testcase classname="other" name="same" /></testsuite>
        </testsuites>"""
        with patch.object(
            publisher,
            "download",
            side_effect=[
                archive([("ci/junit.xml", xml)]),
                archive([("ci-windows/junit.xml", xml)]),
            ],
        ):
            self.assertEqual(
                publisher.read_inventory(REPOSITORY, [{"id": 1}, {"id": 2}]),
                {"suite": {("module", "same"), ("other", "same"), ("", "unqualified")}},
            )

    def test_rejects_entities_and_malformed_xml(self):
        for xml in [
            b"<!DOCTYPE testsuite><testsuite name='a'/>",
            b"<!ENTITY secret SYSTEM 'file:///etc/passwd'>",
            b"<testsuite",
            b"<other/>",
        ]:
            with (
                self.subTest(xml=xml),
                self.assertRaises((ValueError, publisher.ET.ParseError)),
            ):
                publisher.inventory(xml)

    def test_rejects_oversized_members_before_reading(self):
        with archive([("junit.xml", b"12345")]) as zipped, self.assertRaises(ValueError):
            publisher.read_member(zipped, zipped.infolist()[0], 4)

    def test_reads_members_without_extracting_paths(self):
        with patch.object(
            publisher,
            "download",
            return_value=archive(
                [
                    (
                        "../../junit.xml",
                        b'<testsuite name="suite"><testcase name="test"/></testsuite>',
                    )
                ]
            ),
        ):
            self.assertEqual(
                publisher.read_inventory(REPOSITORY, [{}]), {"suite": {("", "test")}}
            )

    def test_total_xml_limit(self):
        with (
            patch.object(publisher, "MAX_TOTAL_XML_BYTES", 5),
            patch.object(
                publisher, "download", return_value=archive([("junit.xml", b"123456")])
            ),
            self.assertRaises(ValueError),
        ):
            publisher.read_inventory(REPOSITORY, [{}])

    def test_counts_changes_and_escapes_artifact_text(self):
        body, changed = publisher.render(
            {"<suite>": {("", "removed"), ("", "same")}},
            {"<suite>": {("", "<script>"), ("", "same")}},
            "stack<base>",
        )
        self.assertTrue(changed)
        self.assertEqual(
            body,
            """## uv test inventory changes

This PR changes the tests compared with the <code>stack&lt;base&gt;</code> base revision.

- Added tests: **1**
- Removed tests: **1**
- Changed suites: **1**

<details>
<summary><code>&lt;suite&gt;</code>: +1 / -1</summary>

**Added:**
- <code>&lt;script&gt;</code>

**Removed:**
- <code>removed</code>

</details>""",
        )

    def test_truncation_keeps_counts_and_closes_details(self):
        with patch.object(publisher, "MAX_DETAILS_CHARACTERS", 1):
            body, changed = publisher.render({}, {"suite": {("", "new")}}, "main")
        self.assertTrue(changed)
        self.assertIn("Added tests: **1**", body)
        self.assertIn("truncated", body)
        self.assertNotIn("<details>", body)


class SourceTests(unittest.TestCase):
    def test_run_requires_current_successful_attempt_and_expected_source(self):
        for changes, raises in [
            ({"run_attempt": 3}, False),
            ({"conclusion": "failure"}, False),
            ({"status": "in_progress"}, False),
            ({"head_sha": BASE}, True),
            ({"event": "workflow_dispatch"}, True),
            ({"path": ".github/workflows/other.yml"}, True),
            ({"repository": {"id": 999}}, True),
        ]:
            with (
                self.subTest(changes=changes),
                patch.object(publisher, "api", return_value=RUN | changes),
            ):
                if raises:
                    with self.assertRaises(ValueError):
                        publisher.current_run(REPOSITORY, 200, 2, HEAD)
                else:
                    self.assertIsNone(publisher.current_run(REPOSITORY, 200, 2, HEAD))

    def test_fork_run_without_pr_metadata_resolves_exact_head(self):
        wrong = copy.deepcopy(PULL)
        wrong["head"]["repo"]["id"] = 999
        with patch.object(publisher, "pages", return_value=[wrong, PULL]) as pages:
            self.assertEqual(
                publisher.matching_pull(REPOSITORY, RUN | {"pull_requests": []}), PULL
            )
        self.assertEqual(
            pages.call_args.args[1], "pulls?state=open&head=contributor%3Achange"
        )

    def test_ambiguous_pull_requests_are_not_selected(self):
        with patch.object(
            publisher, "pages", return_value=[PULL, PULL | {"number": 43}]
        ):
            self.assertIsNone(
                publisher.matching_pull(REPOSITORY, RUN | {"pull_requests": []})
            )

    def test_comparison_uses_tested_base_and_shared_artifact_names(self):
        current = {
            "test-inventory-revision": {"id": 1},
            "junit-results-linux": {"id": 2},
            "junit-results-windows-1": {"id": 3},
        }
        baseline = {"junit-results-linux": {"id": 4}, "junit-results-macos": {"id": 5}}
        with (
            patch.object(publisher, "current_run", return_value=RUN),
            patch.object(publisher, "matching_pull", return_value=PULL),
            patch.object(publisher, "artifacts", side_effect=[current, baseline]),
            patch.object(
                publisher,
                "revision",
                return_value={"merge": MERGE, "base": BASE, "head": HEAD},
            ),
            patch.object(
                publisher,
                "api",
                return_value={"parents": [{"sha": BASE}, {"sha": HEAD}]},
            ),
            patch.object(
                publisher, "baseline_run", return_value={"id": 100}
            ) as baseline_run,
            patch.object(
                publisher, "read_inventory", side_effect=[{}, {"suite": {("", "new")}}]
            ) as read,
        ):
            report = publisher.prepare(REPOSITORY, 200, 2, HEAD)
        baseline_run.assert_called_once_with(REPOSITORY, "main", BASE)
        self.assertEqual(
            [call.args[1] for call in read.call_args_list], [[{"id": 4}], [{"id": 2}]]
        )
        self.assertTrue(report["changed"])
        self.assertEqual(report["merge_sha"], MERGE)

    def test_missing_baseline_updates_only_existing_reports(self):
        with (
            patch.object(publisher, "current_run", return_value=RUN),
            patch.object(publisher, "matching_pull", return_value=PULL),
            patch.object(
                publisher,
                "artifacts",
                return_value={"test-inventory-revision": {"id": 1}},
            ),
            patch.object(
                publisher,
                "revision",
                return_value={"merge": MERGE, "base": BASE, "head": HEAD},
            ),
            patch.object(
                publisher,
                "api",
                return_value={"parents": [{"sha": BASE}, {"sha": HEAD}]},
            ),
            patch.object(publisher, "baseline_run", return_value=None),
        ):
            report = publisher.prepare(REPOSITORY, 200, 2, HEAD)
        self.assertFalse(report["changed"])
        self.assertIn("unavailable", report["body"])

    def test_stacked_base_requires_ci_for_its_exact_merge(self):
        producing = copy.deepcopy(PULL) | {
            "merged_at": "2026-10-01",
            "merge_commit_sha": BASE,
        }
        with (
            patch.object(publisher, "latest_run", side_effect=[None, RUN]),
            patch.object(publisher, "pages", return_value=[producing]),
            patch.object(
                publisher,
                "artifacts",
                return_value={"test-inventory-revision": {"id": 1}},
            ),
            patch.object(publisher, "revision", return_value={"merge": BASE}),
        ):
            self.assertEqual(publisher.baseline_run(REPOSITORY, "main", BASE), RUN)
        with (
            patch.object(publisher, "latest_run", side_effect=[None, RUN]),
            patch.object(publisher, "pages", return_value=[producing]),
            patch.object(
                publisher,
                "artifacts",
                return_value={"test-inventory-revision": {"id": 1}},
            ),
            patch.object(publisher, "revision", return_value={"merge": MERGE}),
        ):
            self.assertIsNone(publisher.baseline_run(REPOSITORY, "main", BASE))


class PublishingTests(unittest.TestCase):
    def setUp(self):
        run = patch.object(publisher, "current_run", return_value=RUN)
        pull = patch.object(publisher, "matching_pull", return_value=PULL)
        self.run = run.start()
        self.pull = pull.start()
        self.addCleanup(run.stop)
        self.addCleanup(pull.stop)

    def test_creates_on_missing_or_deleted_id(self):
        for stored in [None, 456]:
            responses = [None, {"id": 789}] if stored else [{"id": 789}]
            with (
                self.subTest(stored=stored),
                patch.object(publisher, "api", side_effect=responses) as api,
            ):
                self.assertEqual(
                    publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT, stored), 789
                )
                self.assertEqual(api.call_args.args[1], "issues/42/comments")
                self.assertEqual(api.call_args.kwargs["method"], "POST")

    def test_no_change_does_not_create_a_comment(self):
        with patch.object(publisher, "api") as api:
            self.assertIsNone(
                publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT | {"changed": False})
            )
            api.assert_not_called()

    def test_no_change_updates_a_previous_report(self):
        with patch.object(publisher, "api", side_effect=[COMMENT, {}]) as api:
            self.assertEqual(
                publisher.publish(
                    REPOSITORY, 200, 2, HEAD, REPORT | {"changed": False}, 456
                ),
                456,
            )
            self.assertEqual(api.call_args.kwargs["method"], "PATCH")

    def test_validates_comment_author_pr_marker_and_run(self):
        for changes in [
            {"user": {"id": 999}},
            {"issue_url": COMMENT["issue_url"] + "0"},
            {"body": "Other comment"},
            {"body": publisher.MARKER},
        ]:
            with (
                self.subTest(changes=changes),
                patch.object(publisher, "api", return_value=COMMENT | changes) as api,
            ):
                with self.assertRaises(ValueError):
                    publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT, 456)
                self.assertEqual(api.call_count, 1)

    def test_newer_comment_is_not_overwritten(self):
        newer = COMMENT | {"body": f"{publisher.MARKER}\n<!-- report-run:201:1 -->"}
        with patch.object(publisher, "api", return_value=newer) as api:
            self.assertEqual(
                publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT, 456), 456
            )
            self.assertEqual(api.call_count, 1)

    def test_changed_merge_or_pull_does_not_publish(self):
        for changes in [{"merge_commit_sha": BASE}, {"number": 43}]:
            with self.subTest(changes=changes), patch.object(publisher, "api") as api:
                self.pull.return_value = PULL | changes
                self.assertIsNone(
                    publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT, 456)
                )
                api.assert_not_called()

    def test_ambiguous_write_failure_is_not_retried(self):
        with patch.object(publisher, "api", side_effect=RuntimeError("timeout")) as api:
            with self.assertRaises(RuntimeError):
                publisher.publish(REPOSITORY, 200, 2, HEAD, REPORT)
            self.assertEqual(api.call_count, 1)

    def test_invalid_cached_ids(self):
        for value in ["", "0", "-1", "1.0", "1e3", " 1", "1\n", "1/other"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                publisher.comment_id(value)


class ApiTests(unittest.TestCase):
    def test_only_read_404_is_a_missing_result(self):
        for method, status in [("GET", 403), ("GET", 429), ("GET", 500), ("POST", 404)]:
            with (
                self.subTest(method=method, status=status),
                patch.object(
                    publisher.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess(
                        [], 1, "", f"gh: error (HTTP {status})"
                    ),
                ),
                self.assertRaises(RuntimeError),
            ):
                publisher.api(REPOSITORY, "issues/comments/456", method=method)
        with patch.object(
            publisher.subprocess,
            "run",
            return_value=subprocess.CompletedProcess(
                [], 1, "", "gh: Not Found (HTTP 404)"
            ),
        ):
            self.assertIsNone(publisher.api(REPOSITORY, "issues/comments/456"))

    def test_comment_body_is_json_data(self):
        body = "$(touch /should-not-exist)\n${{ secrets.TOKEN }}"
        with patch.object(
            publisher.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 0, '{"id": 1}', ""),
        ) as run:
            publisher.api(REPOSITORY, "issues/42/comments", method="POST", body=body)
        self.assertEqual(json.loads(run.call_args.kwargs["input"]), {"body": body})
        self.assertNotIn(body, run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
