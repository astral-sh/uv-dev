import argparse
import io
import json
import subprocess
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations.cli import main as automation_main
from uv_automations.github import ISSUE_FIELDS, GitHub, decode_issue
from uv_automations.issues_cli import PrepareIssue, add_commands, main, parse_command
from uv_automations.models import Issue, IssueAuthor, IssueRef, RepositoryName
from uv_automations.workflows.issues import PreparedIssue, prepare_issue

REPOSITORY = RepositoryName("astral-sh/uv")
REFERENCE = IssueRef(REPOSITORY, 123)
AUTHOR = IssueAuthor("MDQ6VXNlcjE=", False, "contributor", "Contributor")
ISSUE = Issue(REFERENCE, "An issue\ntitle", "The issue body", AUTHOR)


class IssueReader:
    def __init__(self, issue: Issue = ISSUE) -> None:
        self.issue = issue
        self.references: list[IssueRef] = []

    def get_issue(self, reference: IssueRef) -> Issue:
        self.references.append(reference)
        return self.issue


class IssueReferenceTests(unittest.TestCase):
    def test_parse_issue_number_or_url(self) -> None:
        for value in ["123", "https://github.com/astral-sh/uv/issues/123"]:
            with self.subTest(value=value):
                self.assertEqual(IssueRef.from_input(REPOSITORY, value), REFERENCE)

    def test_reject_invalid_issue_references(self) -> None:
        values = [
            "",
            "0",
            "-1",
            "0123",
            "１２３",
            "123\n",
            "https://github.com/astral-sh/uv-dev/issues/123",
            "https://github.com/astral-sh/uv/pull/123",
            "https://github.com/astral-sh/uv/issues/123/",
            "https://github.com/astral-sh/uv/issues/123?x=1",
            "https://github.com/astral-sh/uv/issues/123#comment",
            "https://github.com/astral-sh/uv/issues/../123",
            "https://github.com.evil/astral-sh/uv/issues/123",
        ]
        for value in values:
            with self.subTest(value=value), self.assertRaises(ValueError):
                IssueRef.from_input(REPOSITORY, value)

    def test_reject_invalid_issue_numbers(self) -> None:
        for value in [0, -1, True]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                IssueRef(REPOSITORY, value)


class IssueGitHubTests(unittest.TestCase):
    def test_decode_issue_preserves_prompt_payload(self) -> None:
        self.assertEqual(decode_issue(ISSUE.to_payload(), REFERENCE), ISSUE)
        self.assertEqual(
            ISSUE.to_payload(),
            {
                "number": 123,
                "title": "An issue\ntitle",
                "body": "The issue body",
                "author": {
                    "id": "MDQ6VXNlcjE=",
                    "is_bot": False,
                    "login": "contributor",
                    "name": "Contributor",
                },
                "url": "https://github.com/astral-sh/uv/issues/123",
            },
        )

    def test_nullable_author_fields(self) -> None:
        self.assertEqual(
            decode_issue({**ISSUE.to_payload(), "author": None}, REFERENCE).author,
            None,
        )
        author = {**AUTHOR.to_payload(), "name": None}
        self.assertEqual(
            decode_issue({**ISSUE.to_payload(), "author": author}, REFERENCE).author,
            IssueAuthor("MDQ6VXNlcjE=", False, "contributor", None),
        )

    def test_reject_unexpected_issue_identity(self) -> None:
        for changed in [
            {"number": 456},
            {"number": True},
            {"url": "https://github.com/astral-sh/uv-dev/issues/123"},
            {"url": "https://github.com/astral-sh/uv/issues/456"},
            {"url": "https://github.com/astral-sh/uv/pull/123"},
        ]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                decode_issue({**ISSUE.to_payload(), **changed}, REFERENCE)

    def test_reject_untyped_issue_fields(self) -> None:
        for changed in [
            {"title": None},
            {"body": 123},
            {"author": {**AUTHOR.to_payload(), "is_bot": 1}},
            {"author": {**AUTHOR.to_payload(), "id": 123}},
        ]:
            with self.subTest(changed=changed), self.assertRaises(TypeError):
                decode_issue({**ISSUE.to_payload(), **changed}, REFERENCE)

    def test_get_issue_uses_verified_number_and_repository(self) -> None:
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess(
                [], 0, json.dumps(ISSUE.to_payload())
            )
            self.assertEqual(GitHub().get_issue(REFERENCE), ISSUE)
            run.assert_called_once_with(
                [
                    "gh",
                    "issue",
                    "view",
                    "123",
                    "--repo",
                    "astral-sh/uv",
                    "--json",
                    ISSUE_FIELDS,
                ],
                input=None,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                timeout=60,
            )


class IssuePreparationTests(unittest.TestCase):
    def test_prepare_relative_workspace_destination(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            runner_temp = root / "runner-temp"
            workspace.mkdir()
            runner_temp.mkdir()
            reader = IssueReader()
            prepared = prepare_issue(
                reader,
                REFERENCE,
                Path("issue.json"),
                workspace=workspace,
                runner_temp=runner_temp,
            )
            self.assertEqual(
                prepared,
                PreparedIssue(ISSUE, workspace.resolve() / "issue.json"),
            )
            self.assertEqual(reader.references, [REFERENCE])
            self.assertEqual(json.loads(prepared.path.read_text()), ISSUE.to_payload())

    def test_prepare_absolute_runner_destination(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            runner_temp = root / "runner-temp"
            workspace.mkdir()
            runner_temp.mkdir()
            destination = runner_temp / "issue.json"
            prepared = prepare_issue(
                IssueReader(),
                REFERENCE,
                destination,
                workspace=workspace,
                runner_temp=runner_temp,
            )
            self.assertEqual(prepared.path, destination.resolve())
            self.assertEqual(json.loads(prepared.path.read_text()), ISSUE.to_payload())

    def test_reject_escape_before_github_read(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            runner_temp = root / "runner-temp"
            outside = root / "outside"
            workspace.mkdir()
            runner_temp.mkdir()
            outside.mkdir()
            (workspace / "outside-link").symlink_to(outside, target_is_directory=True)
            destinations = [
                root / "issue.json",
                Path("../issue.json"),
                Path("outside-link/issue.json"),
                Path("issue\nother.json"),
                workspace,
            ]
            reader = IssueReader()
            for destination in destinations:
                with (
                    self.subTest(destination=destination),
                    self.assertRaises(ValueError),
                ):
                    prepare_issue(
                        reader,
                        REFERENCE,
                        destination,
                        workspace=workspace,
                        runner_temp=runner_temp,
                    )
            self.assertEqual(reader.references, [])
            self.assertEqual(list(outside.iterdir()), [])

    def test_existing_destination_is_not_replaced(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "issue.json"
            destination.write_text("existing", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                prepare_issue(
                    IssueReader(),
                    REFERENCE,
                    destination,
                    workspace=root,
                    runner_temp=root,
                )
            self.assertEqual(destination.read_text(), "existing")

    def test_dangling_leaf_symlink_is_not_followed(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "issue.json"
            target = root / "missing.json"
            destination.symlink_to(target)
            with self.assertRaises(FileExistsError):
                prepare_issue(
                    IssueReader(),
                    REFERENCE,
                    destination,
                    workspace=root,
                    runner_temp=root,
                )
            self.assertTrue(destination.is_symlink())
            self.assertFalse(target.exists())

    def test_reject_unexpected_reader_result_before_writing(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "issue.json"
            unexpected = Issue(IssueRef(REPOSITORY, 456), "Title", "Body", None)
            with self.assertRaises(ValueError):
                prepare_issue(
                    IssueReader(unexpected),
                    REFERENCE,
                    destination,
                    workspace=root,
                    runner_temp=root,
                )
            self.assertFalse(destination.exists())


class IssueCliTests(unittest.TestCase):
    def test_main_cli_delegates_issue_command(self) -> None:
        with (
            patch("uv_automations.cli.logging.basicConfig"),
            patch("uv_automations.cli.issues_cli.run") as run,
        ):
            automation_main(
                [
                    "issues",
                    "prepare",
                    "--repo",
                    "astral-sh/uv",
                    "--issue",
                    REFERENCE.url,
                    "--path",
                    "issue.json",
                    "--workspace",
                    "workspace",
                    "--runner-temp",
                    "runner-temp",
                ]
            )
        run.assert_called_once_with(
            PrepareIssue(
                reference=REFERENCE,
                path=Path("issue.json"),
                workspace=Path("workspace"),
                runner_temp=Path("runner-temp"),
                github_output=None,
            )
        )

    def test_parse_prepare_issue(self) -> None:
        parser = argparse.ArgumentParser()
        add_commands(parser)
        self.assertEqual(
            parse_command(
                parser.parse_args(
                    [
                        "prepare",
                        "--repo",
                        "astral-sh/uv",
                        "--issue",
                        REFERENCE.url,
                        "--path",
                        "issue.json",
                        "--workspace",
                        "workspace",
                        "--runner-temp",
                        "runner-temp",
                    ]
                )
            ),
            PrepareIssue(
                reference=REFERENCE,
                path=Path("issue.json"),
                workspace=Path("workspace"),
                runner_temp=Path("runner-temp"),
                github_output=None,
            ),
        )

    def test_prepare_issue_actions_outputs(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "issue.json"
            github_output = root / "github-output"
            with (
                patch("uv_automations.issues_cli.logging.basicConfig"),
                patch("uv_automations.github.GitHub.get_issue", return_value=ISSUE),
                redirect_stdout(io.StringIO()) as output,
            ):
                main(
                    [
                        "prepare",
                        "--repo",
                        "astral-sh/uv",
                        "--issue",
                        "123",
                        "--path",
                        str(destination),
                        "--workspace",
                        str(root),
                        "--runner-temp",
                        str(root),
                        "--github-output",
                        str(github_output),
                    ]
                )
            self.assertEqual(output.getvalue(), "")
            outputs = dict(
                line.split("=", 1) for line in github_output.read_text().splitlines()
            )
            self.assertEqual(outputs["issue-number"], "123")
            self.assertEqual(json.loads(outputs["issue-json"]), ISSUE.to_payload())
            self.assertEqual(outputs["path"], str(destination.resolve()))

    def test_invalid_reference_never_reads_github(self) -> None:
        with (
            patch("uv_automations.issues_cli.logging.basicConfig"),
            patch("uv_automations.github.GitHub.get_issue") as get_issue,
            redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as error,
        ):
            main(
                [
                    "prepare",
                    "--repo",
                    "astral-sh/uv",
                    "--issue",
                    "https://github.com/other/repo/issues/123",
                    "--path",
                    "issue.json",
                    "--workspace",
                    "workspace",
                    "--runner-temp",
                    "runner-temp",
                ]
            )
        self.assertEqual(error.exception.code, 2)
        get_issue.assert_not_called()
