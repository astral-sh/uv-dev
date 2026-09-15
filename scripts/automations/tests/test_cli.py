import io
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations.cli import (
    FindConflicts,
    ValidateLabels,
    create_parser,
    main,
    parse_command,
)
from uv_automations.github import GitHub
from uv_automations.models import RepositoryName
from uv_automations.workflows.labels import LabelApplyOutcome


class CliTests(unittest.TestCase):
    def test_parse_validate_labels(self) -> None:
        self.assertEqual(
            parse_command(
                create_parser(),
                ["labels", "validate", "--allowed", "allowed.json"],
            ),
            ValidateLabels(allowed=Path("allowed.json"), github_output=None),
        )

    def test_parse_find_conflicts(self) -> None:
        self.assertEqual(
            parse_command(
                create_parser(),
                [
                    "pull-requests",
                    "conflicts",
                    "--repo",
                    "astral-sh/uv",
                    "--author",
                    "app/astral-automations-bot",
                ],
            ),
            FindConflicts(
                repository=RepositoryName("astral-sh/uv"),
                author="app/astral-automations-bot",
            ),
        )

    def test_find_conflicts(self) -> None:
        output = io.StringIO()
        with (
            patch("uv_automations.cli.logging.basicConfig"),
            patch(
                "uv_automations.cli.find_conflicted_pull_requests", return_value=()
            ) as find,
            redirect_stdout(output),
        ):
            main(["pull-requests", "conflicts", "--repo", "astral-sh/uv"])
        find.assert_called_once_with(
            GitHub(), RepositoryName("astral-sh/uv"), author=None
        )
        self.assertEqual(output.getvalue(), "[]\n")

    def test_apply_revalidates_labels(self) -> None:
        with TemporaryDirectory() as directory:
            allowed = Path(directory) / "allowed.json"
            allowed.write_text('["bug"]', encoding="utf-8")
            arguments = [
                "labels",
                "apply",
                "--repo",
                "astral-sh/uv-dev",
                "--pull-request",
                "123",
                "--allowed",
                str(allowed),
                "--expected-head",
                "a" * 40,
            ]
            with (
                patch("uv_automations.cli.logging.basicConfig"),
                patch("uv_automations.cli.sys.stdin", io.StringIO('["bot:rebase"]')),
                patch("uv_automations.cli.apply_labels") as apply,
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit) as error,
            ):
                main(arguments)
            self.assertEqual(error.exception.code, 2)
            apply.assert_not_called()

            output = io.StringIO()
            with (
                patch("uv_automations.cli.logging.basicConfig"),
                patch("uv_automations.cli.sys.stdin", io.StringIO('["bug"]')),
                patch(
                    "uv_automations.cli.apply_labels",
                    return_value=LabelApplyOutcome.STALE,
                ) as apply,
                redirect_stdout(output),
            ):
                main(arguments)
            apply.assert_called_once()
            self.assertEqual(
                output.getvalue(),
                "Skipped labels because the pull request is closed or its head changed.\n",
            )
