"""Offline regression coverage for documentation publication."""

import importlib.util
import io
import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "publish_docs", ROOT / "scripts/publish-docs-pr.py"
)
assert SPEC is not None and SPEC.loader is not None
PUBLISHER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLISHER)
BRANCH = "update-docs-0.12.23-200"
HEAD = "a" * 40
SOURCE = "b" * 40
TITLE = "Update uv documentation for 0.12.23"
CREATED_URL = "https://github.com/astral-sh/docs/pull/91"


def pull(number=11, **changes):
    return {
        "number": number,
        "title": TITLE,
        "body": f"Automated documentation update for 0.12.23\n\n<!-- uv-source: {SOURCE} -->",
        "baseRefName": "main",
        "headRefName": f"update-docs-0.12.23-{number}",
        "headRefOid": HEAD,
        "isCrossRepository": False,
        "state": "OPEN",
        "author": {"login": "app/astral-releases-bot"},
    } | changes


class PublishDocs(unittest.TestCase):
    def setUp(self):
        self.commands = []
        self.predecessors = [pull()]
        self.replacement = pull(91, headRefName=BRANCH)
        self.failure = None
        self.output = io.StringIO()

    def run_command(self, *arguments):
        self.commands.append(arguments)
        if arguments[:2] == self.failure or arguments[1:3] == self.failure:
            raise subprocess.CalledProcessError(1, arguments)
        if arguments[:2] == ("git", "rev-parse"):
            return HEAD
        if arguments[1:3] == ("pr", "create"):
            return CREATED_URL
        if arguments[1:3] in (("pr", "view"), ("pr", "list")):
            fields = arguments[arguments.index("--json") + 1].split(",")
            if arguments[1:3] == ("pr", "view"):
                return json.dumps({key: self.replacement[key] for key in fields})
            return json.dumps(
                [
                    {key: predecessor[key] for key in fields}
                    for predecessor in self.predecessors
                ]
            )
        return ""

    def publish(self):
        with (
            patch.object(PUBLISHER, "run", self.run_command),
            redirect_stdout(self.output),
        ):
            PUBLISHER.publish(BRANCH, "0.12.23", SOURCE)

    def closed(self):
        return [args[3] for args in self.commands if args[1:3] == ("pr", "close")]

    def test_replacement_exists_before_predecessor_is_closed(self):
        self.publish()
        self.assertEqual(self.closed(), ["11"])
        operations = [command[1:3] for command in self.commands]
        self.assertLess(
            operations.index(("pr", "view")), operations.index(("pr", "close"))
        )
        self.assertIn(
            ("git", "push", "origin", f"{HEAD}:refs/heads/{BRANCH}"), self.commands
        )

    def test_created_pull_request_link_is_logged(self):
        self.publish()
        self.assertEqual(self.output.getvalue(), CREATED_URL + "\n")

    def test_old_documentation_source_uses_workflow_tools(self):
        workflow = (ROOT / ".github/workflows/publish-docs.yml").read_text()
        publishing = workflow.split("  mkdocs:\n", 1)[1]
        tools = publishing.split('      - name: "Load publication tools"\n', 1)[
            1
        ].split("      - ", 1)[0]
        self.assertIn("ref: ${{ github.workflow_sha }}", tools)
        self.assertIn("path: .docs-workflow", tools)
        step = publishing.split('      - name: "Create Pull Request"\n', 1)[1].split(
            "      - name:", 1
        )[0]
        command = step.split("        run: ", 1)[1].strip()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "astral-docs").mkdir()
            # Older selected sources have no publisher; only workflow tools do.
            (root / ".docs-workflow/scripts").mkdir(parents=True)
            (root / ".docs-workflow/scripts/publish-docs-pr.py").write_text(
                "import json, sys\nprint(json.dumps(sys.argv[1:]))\n"
            )
            result = subprocess.run(
                ["bash", "-e", "-c", command],
                cwd=root / "astral-docs",
                env={
                    **os.environ,
                    "branch_name": BRANCH,
                    "display_name": "0.12.23",
                    "source_commit": SOURCE,
                },
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertEqual(json.loads(result.stdout), [BRANCH, "0.12.23", SOURCE])

    def test_failed_publication_keeps_predecessors(self):
        for failure in (
            ("git", "push"),
            ("pr", "create"),
            ("pr", "view"),
            ("pr", "list"),
        ):
            with self.subTest(failure=failure):
                self.commands = []
                self.failure = failure
                with self.assertRaises(subprocess.CalledProcessError):
                    self.publish()
                self.assertEqual(self.closed(), [])

    def test_mismatched_replacement_keeps_predecessors(self):
        for changes in (
            {"headRefOid": "b" * 40},
            {"isCrossRepository": True},
            {"state": "CLOSED"},
            {"number": True},
        ):
            with self.subTest(changes=changes):
                self.commands = []
                self.replacement = pull(91, headRefName=BRANCH) | changes
                with self.assertRaises(RuntimeError):
                    self.publish()
                self.assertEqual(self.closed(), [])

    def test_only_older_bot_owned_publications_are_closed(self):
        self.predecessors += [
            pull(92),
            pull(12, author={"login": "maintainer"}),
            pull(13, title="Another publication"),
            pull(14, headRefName="unrelated"),
            pull(15, isCrossRepository=True),
            pull(16, author=None),
            pull(17, headRefName=BRANCH),
        ]
        self.publish()
        self.assertEqual(self.closed(), ["11"])

    def test_out_of_order_source_publications_remain_open(self):
        self.predecessors = [
            pull(11, body=f"<!-- uv-source: {'c' * 40} -->"),
            pull(12),
            pull(13, body="No recorded source"),
            pull(
                14, body=f"<!-- uv-source: {SOURCE} -->\n<!-- uv-source: {SOURCE} -->"
            ),
        ]
        self.publish()
        self.assertEqual(self.closed(), ["12"])

    def test_no_change_commit_is_a_successful_noop(self):
        workflow = (ROOT / ".github/workflows/publish-docs.yml").read_text()
        step = workflow.split('      - name: "Commit docs"\n', 1)[1].split(
            "      - name:", 1
        )[0]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            environment = {
                **os.environ,
                "branch_name": BRANCH,
                "version": "0.12.23",
                "GITHUB_OUTPUT": str(root / "outputs"),
            }
            subprocess.run(["git", "init", "-q", directory], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    directory,
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "Initial",
                ],
                check=True,
            )
            (root / "site/uv").mkdir(parents=True)
            subprocess.run(
                ["bash", "-e", "-c", script],
                cwd=root,
                env=environment,
                check=True,
                capture_output=True,
            )
            self.assertEqual((root / "outputs").read_text(), "changed=false\n")
            self.assertIn("steps.commit.outputs.changed == 'true'", workflow)


if __name__ == "__main__":
    unittest.main()
