#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///

"""Integration tests for malicious-code scan evidence and report validation."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("malicious-code-scan.py")


class MaliciousCodeScanTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.repository = self.root / "repository"
        self.repository.mkdir()
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Scan fixture")
        self.git("config", "user.email", "scan@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.base = self.commit("README.md", "Original content\n")

    def git(self, *arguments: str) -> str:
        return subprocess.run(
            ["git", *arguments],
            cwd=self.repository,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def commit(self, name: str, content: str) -> str:
        (self.repository / name).parent.mkdir(parents=True, exist_ok=True)
        (self.repository / name).write_text(content)
        self.git("add", "--", name)
        self.git("commit", "-m", "Update fixture")
        return self.git("rev-parse", "HEAD")

    def prepare(self, base: str, head: str, kind: str = "PUSH") -> dict:
        source = self.root / "source.git"
        self.git("clone", "--bare", str(self.repository), str(source))
        self.output = self.root / "scan"
        subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "prepare",
                "--git-dir",
                str(source),
                "--output",
                str(self.output),
                "--kind",
                kind,
                "--base",
                base,
                "--head",
                head,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        return json.loads((self.output / "receipt.json").read_text())

    def clean_report(self, receipt: dict) -> dict:
        return {
            "outcome": "CLEAN",
            "summary": "Reviewed the fixture changes.",
            "reviewed_commits": [commit["sha"] for commit in receipt["commits"]],
            "reviewed_net_diff": True,
            "findings": [],
            "coverage_gaps": [],
        }

    def report(self, report: dict) -> subprocess.CompletedProcess[str]:
        report_path = self.root / "result.json"
        report_path.write_text(json.dumps(report))
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "report",
                "--receipt",
                str(self.output / "receipt.json"),
                "--report",
                str(report_path),
                "--summary",
                str(self.root / "summary.md"),
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_push_preserves_changes_removed_by_later_commits(self) -> None:
        added = self.commit("temporary.txt", "Transient fixture content\n")
        self.git("rm", "temporary.txt")
        self.git("commit", "-m", "Remove fixture")
        head = self.git("rev-parse", "HEAD")
        receipt = self.prepare(self.base, head)
        self.assertEqual(
            [commit["sha"] for commit in receipt["commits"]], [added, head]
        )
        self.assertEqual((self.output / "net.diff").read_text(), "")
        self.assertIn(
            "+Transient fixture content",
            (self.output / f"commits/{added}.diff").read_text(),
        )
        self.assertIn(
            "-Transient fixture content",
            (self.output / f"commits/{head}.diff").read_text(),
        )

    def test_pull_request_uses_merge_base_and_does_not_materialize_instructions(
        self,
    ) -> None:
        self.git("switch", "-c", "feature")
        head = self.commit("AGENTS.md", "Target repository fixture text\n")
        self.git("switch", "main")
        base = self.commit("base-only.txt", "Unrelated base update\n")
        receipt = self.prepare(base, head, "PULL_REQUEST")
        self.assertEqual(receipt["comparison_sha"], self.base)
        self.assertEqual([commit["sha"] for commit in receipt["commits"]], [head])
        self.assertNotIn("base-only.txt", (self.output / "net.diff").read_text())
        self.assertFalse((self.output / "AGENTS.md").exists())
        self.assertEqual(receipt["commits"][0]["paths"], ["AGENTS.md"])

    def test_merge_resolution_is_included(self) -> None:
        self.git("switch", "-c", "feature")
        self.commit("README.md", "Feature content\n")
        self.git("switch", "main")
        self.commit("README.md", "Main content\n")
        merged = subprocess.run(
            ["git", "merge", "--no-ff", "feature"],
            cwd=self.repository,
            capture_output=True,
            check=False,
        )
        self.assertEqual(merged.returncode, 1)
        head = self.commit("README.md", "Resolved content\n")
        receipt = self.prepare(self.base, head)
        merge = receipt["commits"][-1]
        self.assertEqual(len(merge["parents"]), 2)
        self.assertIn("+Resolved content", (self.output / merge["diff"]).read_text())

    def test_initial_push_includes_root_commit(self) -> None:
        receipt = self.prepare("0" * 40, self.base)
        self.assertEqual([commit["sha"] for commit in receipt["commits"]], [self.base])
        self.assertIn(
            "+Original content",
            (self.output / receipt["commits"][0]["diff"]).read_text(),
        )

    def test_force_push_can_report_restored_code_without_new_commits(self) -> None:
        before = self.commit("README.md", "Replacement content\n")
        receipt = self.prepare(before, self.base)
        self.assertEqual(receipt["commits"], [])
        report = self.clean_report(receipt)
        report["outcome"] = "SUSPICIOUS"
        report["findings"] = [
            {
                "title": "Review restored fixture",
                "body": "The branch update restored this content.",
                "severity": "MEDIUM",
                "commit_sha": self.base,
                "path": "README.md",
                "line": 1,
            }
        ]
        result = self.report(report)
        self.assertEqual(result.returncode, 1)
        self.assertIn("needs human review", result.stderr)
        self.assertTrue((self.root / "summary.md").exists())

    def test_clean_report_requires_complete_commit_and_range_coverage(self) -> None:
        head = self.commit("new.txt", "Fixture content\n")
        receipt = self.prepare(self.base, head)
        for key, value in (
            ("reviewed_commits", []),
            ("reviewed_net_diff", False),
            ("coverage_gaps", ["Not reviewed"]),
        ):
            with self.subTest(key=key):
                report = self.clean_report(receipt)
                report[key] = value
                self.assertNotEqual(self.report(report).returncode, 0)
        self.assertEqual(self.report(self.clean_report(receipt)).returncode, 0)

    def test_unrelated_findings_are_rejected(self) -> None:
        head = self.commit("new.txt", "Fixture content\n")
        receipt = self.prepare(self.base, head)
        report = self.clean_report(receipt)
        report["outcome"] = "SUSPICIOUS"
        report["findings"] = [
            {
                "title": "Unrelated fixture",
                "body": "Fixture",
                "severity": "LOW",
                "commit_sha": head,
                "path": "README.md",
                "line": 1,
            }
        ]
        result = self.report(report)
        self.assertIn("changed path", result.stderr)
        self.assertFalse((self.root / "summary.md").exists())

    def test_summary_keeps_generated_markdown_inside_json(self) -> None:
        receipt = self.prepare(self.base, self.commit("new.txt", "Fixture content\n"))
        report = self.clean_report(receipt)
        report["summary"] = "Fixture\n```\n# Embedded heading\n```"
        self.assertEqual(self.report(report).returncode, 0)
        summary = (self.root / "summary.md").read_text()
        self.assertEqual(summary.splitlines().count("```"), 1)
        self.assertNotIn("\n# Embedded heading\n", summary)


if __name__ == "__main__":
    unittest.main()
