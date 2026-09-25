import io
import json
import subprocess
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import call, patch

from uv_automations.cli import main
from uv_automations.github import LABEL_CONTEXT_FIELDS, GitHub
from uv_automations.models import CommitSha, PullRequestRef, RepositoryName
from uv_automations.workflows.labels import prepare_labels

HEAD = CommitSha("a" * 40)
REFERENCE = PullRequestRef(RepositoryName("astral-sh/uv-dev"), 123)
CONTEXT_COMMAND = [
    "gh",
    "pr",
    "view",
    "123",
    "--repo",
    "astral-sh/uv-dev",
    "--json",
    LABEL_CONTEXT_FIELDS,
]
LABELS_COMMAND = [
    "gh",
    "label",
    "list",
    "--repo",
    "astral-sh/uv-dev",
    "--limit",
    "1000",
    "--json",
    "name,description",
]


def context_payload(*, head: CommitSha = HEAD, number: int = 123) -> str:
    return json.dumps(
        {
            "number": number,
            "title": "Example",
            "body": "Description",
            "author": {"login": "contributor"},
            "baseRefName": "main",
            "headRefName": "change",
            "headRefOid": str(head),
            "isDraft": True,
            "labels": [],
            "files": [],
            "additions": 1,
            "deletions": 0,
            "changedFiles": 1,
        }
    )


def read_failure(
    command: list[str], *, status: int = 1
) -> subprocess.CalledProcessError:
    return subprocess.CalledProcessError(
        status,
        command,
        output='{"partial":true}',
        stderr="HTTP 503: No server is currently available to service your request.",
    )


class LabelContextRetryTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        temporary = self.enterContext(TemporaryDirectory())
        self.root = Path(temporary)
        self.checkout = self.root / "checkout"
        self.checkout.mkdir()
        self.allowed = self.root / "allowed.json"
        self.allowed.write_text('["bug"]', encoding="utf-8")
        self.output = self.root / "github-output"
        self.contexts: list[str | Exception] = [context_payload()]
        self.labels: list[str | Exception] = [
            (
                '[{"name":"bug","description":"A defect"},'
                '{"name":"bot:rebase","description":null}]'
            )
        ]
        self.calls: list[list[str]] = []
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.enterContext(patch("uv_automations.cli.logging.basicConfig"))
        self.enterContext(
            patch("uv_automations.workflows.labels.git.head", return_value=HEAD)
        )
        self.enterContext(
            patch("uv_automations.github.subprocess.run", side_effect=self.run_github)
        )
        self.enterContext(
            patch("subprocess.Popen", side_effect=AssertionError("Unexpected process"))
        )
        self.sleep = self.enterContext(patch("time.sleep"))

    def run_github(
        self, command: list[str], **options: object
    ) -> subprocess.CompletedProcess[str]:
        self.assertEqual(
            options,
            {
                "input": None,
                "check": True,
                "text": True,
                "stdout": subprocess.PIPE,
                "timeout": 60,
            },
        )
        self.calls.append(command)
        if command == CONTEXT_COMMAND:
            response = self.contexts.pop(0)
        elif command == LABELS_COMMAND:
            response = self.labels.pop(0)
        else:
            raise AssertionError(f"Unexpected GitHub operation: {command!r}")
        if isinstance(response, Exception):
            raise response
        return subprocess.CompletedProcess(command, 0, response)

    def invoke(self, *, expected_head: CommitSha = HEAD) -> None:
        with redirect_stdout(self.stdout), redirect_stderr(self.stderr):
            main(
                [
                    "labels",
                    "prepare",
                    "--repo",
                    str(REFERENCE.repository),
                    "--pull-request",
                    str(REFERENCE.number),
                    "--checkout",
                    str(self.checkout),
                    "--allowed",
                    str(self.allowed),
                    "--expected-head",
                    str(expected_head),
                    "--github-output",
                    str(self.output),
                ]
            )

    def assert_prepared(self) -> None:
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertEqual(self.stderr.getvalue(), "")
        self.assertEqual(self.output.read_text(), f"ready=true\nhead-sha={HEAD}\n")
        self.assertEqual(
            json.loads((self.checkout / ".pull-request-labels-event.json").read_text()),
            json.loads(context_payload()),
        )
        self.assertEqual(
            json.loads((self.checkout / ".pull-request-labels.json").read_text()),
            [{"name": "bug", "description": "A defect"}],
        )

    def assert_no_outputs(self) -> None:
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.checkout.iterdir()), [])

    def test_context_read_recovers(self) -> None:
        self.contexts.insert(0, read_failure(CONTEXT_COMMAND))
        self.invoke()
        self.assert_prepared()
        self.assertEqual(self.calls, [CONTEXT_COMMAND, CONTEXT_COMMAND, LABELS_COMMAND])
        self.sleep.assert_called_once_with(5)

    def test_label_list_recovers(self) -> None:
        self.labels.insert(0, read_failure(LABELS_COMMAND))
        self.invoke()
        self.assert_prepared()
        self.assertEqual(self.calls, [CONTEXT_COMMAND, LABELS_COMMAND, LABELS_COMMAND])
        self.sleep.assert_called_once_with(5)

    def test_read_budgets_are_independent(self) -> None:
        self.contexts[:0] = [read_failure(CONTEXT_COMMAND)] * 2
        self.labels[:0] = [read_failure(LABELS_COMMAND)] * 2
        self.invoke()
        self.assert_prepared()
        self.assertEqual(self.calls, [CONTEXT_COMMAND] * 3 + [LABELS_COMMAND] * 3)
        self.assertEqual(self.sleep.call_args_list, [call(5), call(10)] * 2)

    def test_context_exhaustion_has_no_outputs(self) -> None:
        self.contexts = [
            read_failure(CONTEXT_COMMAND, status=status) for status in (1, 2, 7)
        ]
        with self.assertRaises(SystemExit) as error:
            self.invoke()
        self.assertEqual(error.exception.code, 1)
        self.assertEqual(
            self.stderr.getvalue(), "uv-automations: command failed with status 7\n"
        )
        self.assertEqual(self.calls, [CONTEXT_COMMAND] * 3)
        self.assertEqual(self.sleep.call_args_list, [call(5), call(10)])
        self.assert_no_outputs()

    def test_label_exhaustion_has_no_outputs(self) -> None:
        failures = [read_failure(LABELS_COMMAND, status=status) for status in (1, 2, 7)]
        self.labels = list(failures)
        with self.assertRaises(subprocess.CalledProcessError) as error:
            prepare_labels(
                GitHub(), REFERENCE, self.checkout, {"bug"}, expected_head=HEAD
            )
        self.assertIs(error.exception, failures[-1])
        self.assertEqual(self.calls, [CONTEXT_COMMAND] + [LABELS_COMMAND] * 3)
        self.assertEqual(self.sleep.call_args_list, [call(5), call(10)])
        self.assert_no_outputs()

    def test_changed_head_after_retry_is_skipped(self) -> None:
        self.contexts = [
            read_failure(CONTEXT_COMMAND),
            context_payload(head=CommitSha("b" * 40)),
        ]
        self.invoke()
        self.assertEqual(self.output.read_text(), "ready=false\n")
        self.assertEqual(self.calls, [CONTEXT_COMMAND] * 2)
        self.assertEqual(list(self.checkout.iterdir()), [])
        self.sleep.assert_called_once_with(5)

    def test_dispatched_head_mismatch_does_not_read(self) -> None:
        self.invoke(expected_head=CommitSha("b" * 40))
        self.assertEqual(self.output.read_text(), "ready=false\n")
        self.assertEqual(self.calls, [])
        self.sleep.assert_not_called()

    def test_malformed_context_is_not_retried(self) -> None:
        self.contexts = ["not JSON"]
        with self.assertRaises(SystemExit) as error:
            self.invoke()
        self.assertEqual(error.exception.code, 2)
        self.assertEqual(self.calls, [CONTEXT_COMMAND])
        self.sleep.assert_not_called()
        self.assert_no_outputs()

    def test_wrong_pull_request_is_not_retried(self) -> None:
        self.contexts = [context_payload(number=456)]
        with self.assertRaises(SystemExit) as error:
            self.invoke()
        self.assertEqual(error.exception.code, 2)
        self.assertEqual(self.calls, [CONTEXT_COMMAND])
        self.sleep.assert_not_called()
        self.assert_no_outputs()

    def test_malformed_labels_are_not_retried(self) -> None:
        self.labels = ['[{"name":1,"description":null}]']
        with self.assertRaises(SystemExit) as error:
            self.invoke()
        self.assertEqual(error.exception.code, 2)
        self.assertEqual(self.calls, [CONTEXT_COMMAND, LABELS_COMMAND])
        self.sleep.assert_not_called()
        self.assert_no_outputs()

    def test_timeout_is_not_retried(self) -> None:
        self.contexts = [subprocess.TimeoutExpired(CONTEXT_COMMAND, 60)]
        with self.assertRaises(SystemExit) as error:
            self.invoke()
        self.assertEqual(error.exception.code, 1)
        self.assertEqual(
            self.stderr.getvalue(),
            "uv-automations: command timed out after 60 seconds\n",
        )
        self.assertEqual(self.calls, [CONTEXT_COMMAND])
        self.sleep.assert_not_called()
        self.assert_no_outputs()

    def test_os_error_is_not_retried(self) -> None:
        self.contexts = [OSError("unavailable")]
        with self.assertRaisesRegex(OSError, "unavailable"):
            self.invoke()
        self.assertEqual(self.calls, [CONTEXT_COMMAND])
        self.sleep.assert_not_called()
        self.assert_no_outputs()
