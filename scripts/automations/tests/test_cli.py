import io
import json
import subprocess
import unittest
from contextlib import redirect_stderr, redirect_stdout
from functools import partial
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
from uv_automations.workflows.conflicts import find_conflicted_pull_requests
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

    def test_find_conflicts_retries_failed_github_request(self) -> None:
        output = io.StringIO()
        sleeps: list[float] = []
        with (
            patch("uv_automations.cli.logging.basicConfig"),
            patch(
                "uv_automations.cli.find_conflicted_pull_requests",
                partial(find_conflicted_pull_requests, sleep=sleeps.append),
            ),
            patch("uv_automations.github.subprocess.run") as run,
            redirect_stdout(output),
        ):
            run.side_effect = [
                subprocess.CalledProcessError(1, ["gh", "pr", "list"]),
                subprocess.CompletedProcess([], 0, "[]"),
            ]
            main(["pull-requests", "conflicts", "--repo", "astral-sh/uv"])
        self.assertEqual(run.call_count, 2)
        self.assertEqual(sleeps, [5])
        self.assertEqual(output.getvalue(), "[]\n")

    def test_identify_exhaustion_does_not_publish_stale_results(self) -> None:
        pull_request = {
            "number": 123,
            "author": {"login": "astral-automations-bot"},
            "url": "https://github.com/astral-sh/uv/pull/123",
            "baseRefName": "main",
            "headRefName": "feature",
            "headRefOid": "a" * 40,
            "headRepository": {"nameWithOwner": "astral-sh/uv-dev"},
            "mergeable": "CONFLICTING",
        }
        partial_response = json.dumps(
            [pull_request, {**pull_request, "number": 124, "mergeable": "UNKNOWN"}]
        )
        sleeps: list[float] = []
        output = io.StringIO()
        errors = io.StringIO()
        with TemporaryDirectory() as directory:
            github_output = Path(directory) / "github-output"
            summary = Path(directory) / "summary"
            github_output.write_text("existing-output\n", encoding="utf-8")
            summary.write_text("existing-summary\n", encoding="utf-8")
            with (
                patch("uv_automations.cli.logging.basicConfig"),
                patch(
                    "uv_automations.workflows.conflicts.find_conflicted_pull_requests",
                    partial(find_conflicted_pull_requests, sleep=sleeps.append),
                ),
                patch("uv_automations.github.subprocess.run") as run,
                self.assertLogs(
                    "uv_automations.workflows.conflicts", level="WARNING"
                ) as logs,
                redirect_stdout(output),
                redirect_stderr(errors),
                self.assertRaises(SystemExit) as error,
            ):
                run.side_effect = [
                    subprocess.CompletedProcess([], 0, partial_response),
                    *[subprocess.CalledProcessError(1, ["gh", "pr", "list"])] * 4,
                ]
                main(
                    [
                        "pull-requests",
                        "identify",
                        "--repo",
                        "astral-sh/uv",
                        "--repository-id",
                        "699532645",
                        "--github-output",
                        str(github_output),
                        "--summary",
                        str(summary),
                    ]
                )
            self.assertEqual(error.exception.code, 1)
            self.assertEqual(run.call_count, 5)
            self.assertEqual(sleeps, [5] * 4)
            self.assertEqual(
                [record.getMessage() for record in logs.records],
                [
                    f"GitHub mergeability request failed (attempt {attempt}/5); "
                    "retrying..."
                    for attempt in (2, 3, 4)
                ],
            )
            self.assertEqual(output.getvalue(), "")
            self.assertEqual(
                errors.getvalue(), "uv-automations: command failed with status 1\n"
            )
            self.assertEqual(github_output.read_text(), "existing-output\n")
            self.assertEqual(summary.read_text(), "existing-summary\n")

    def test_find_conflicts_rejects_malformed_github_response(self) -> None:
        output = io.StringIO()
        with (
            patch("uv_automations.cli.logging.basicConfig"),
            patch("uv_automations.github.subprocess.run") as run,
            redirect_stdout(output),
            redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as error,
        ):
            run.return_value = subprocess.CompletedProcess([], 0, "{}")
            main(["pull-requests", "conflicts", "--repo", "astral-sh/uv"])
        self.assertEqual(error.exception.code, 2)
        run.assert_called_once()
        self.assertEqual(output.getvalue(), "")

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
