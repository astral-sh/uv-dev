import json
import re
import subprocess
import unittest
from collections import Counter
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from unittest.mock import patch

from uv_automations import cli, promotions_cli
from uv_automations.github_promotion import PromotionReadError
from uv_automations.github_promotion_completion import (
    PromotionCompletionGitHub,
    _response,
    _retry_after,
)
from uv_automations.json import as_object
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    AUTOMATIONS_APP_ID,
    AUTOMATIONS_APP_SLUG,
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    BranchRevision,
    PromotionApprovalKind,
    PromotionScope,
)
from uv_automations.workflows.promotion_completion import (
    CompletionRequestError,
    PromotionCompletion,
    PublicationAction,
    SkippedCompletion,
    close_source,
    complete_metadata,
)

HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
OTHER = CommitSha("c" * 40)
SOURCE = PromotionScope(UV_DEV_REPOSITORY, 2053)
UPSTREAM = PromotionScope(UV_REPOSITORY, 21948)
COMPLETION = PromotionCompletion(
    SOURCE,
    BranchRevision(UV_DEV_REPOSITORY, "main", BASE),
    BranchRevision(UV_DEV_REPOSITORY, "fixture/promotion", HEAD),
    PublicationAction.PROMOTE,
    1,
    PromotionApprovalKind.READY_FOR_REVIEW,
    None,
    UPSTREAM,
    "main",
    HEAD,
)
RUN = subprocess.run


def repository(scope: PromotionScope) -> dict[str, object]:
    return {
        "full_name": str(scope.repository.name),
        "id": scope.repository.database_id,
    }


def human() -> dict[str, object]:
    return {"type": "User", "login": "zanieb", "id": 1}


def pull_request(scope: PromotionScope) -> dict[str, object]:
    return {
        "number": scope.number,
        "html_url": f"https://github.com/{scope.repository.name}/pull/{scope.number}",
        "title": "Fault-injection fixture",
        "body": "",
        "state": "open",
        "draft": False,
        "labels": [{"name": "internal"}, {"name": "bot:promote"}],
        "user": human(),
        "merged_at": None,
        "merge_commit_sha": None,
        "base": {"ref": "main", "sha": str(BASE), "repo": repository(scope)},
        "head": {
            "ref": "fixture/promotion",
            "sha": str(HEAD),
            "repo": repository(scope),
        },
    }


class FaultInjection:
    """Run the real GitHub decoder/writer against an entirely local API boundary."""

    def __init__(self, completion: PromotionCompletion = COMPLETION) -> None:
        self.completion = completion
        self.source = pull_request(completion.source)
        self.upstream = pull_request(completion.upstream)
        self.events = [
            {
                "id": 1,
                "event": "ready_for_review",
                "actor": human(),
                "created_at": "2026-09-09T12:00:00Z",
            }
        ]
        self.comments: list[dict[str, object]] = []
        self.calls: Counter[str] = Counter()
        self.failures: dict[str, list[str]] = {}
        self.headers: dict[str, str] = {}
        self.delays: list[float] = []
        self.trace: list[tuple[str, str | float]] = []
        self.drift_after_labels = False
        self.close_after_record = False
        self.comment_response: object | None = None
        self.permission = "write"

    def receipt(self, body: str | None = None) -> dict[str, object]:
        source = self.completion.source
        return {
            "id": len(self.comments) + 1,
            "issue_url": f"https://api.github.com/repos/{source.repository.name}/issues/{source.number}",
            "user": {
                "type": "Bot",
                "login": f"{AUTOMATIONS_APP_SLUG}[bot]",
                "id": AUTOMATIONS_BOT_ID,
            },
            "performed_via_github_app": {
                "id": AUTOMATIONS_APP_ID,
                "slug": AUTOMATIONS_APP_SLUG,
            },
            "body": body if body is not None else self.completion.comment,
            "created_at": "2026-09-09T12:01:00Z",
            "updated_at": "2026-09-09T12:01:00Z",
        }

    def run(
        self, arguments: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        if len(arguments) == 3 and arguments[:2] == ["git", "check-ref-format"]:
            return RUN(
                arguments, check=False, capture_output=True, text=True, timeout=30
            )
        if arguments[:3] != ["gh", "api", "--method"]:
            raise AssertionError(f"Unexpected external command: {arguments}")
        method, endpoint = arguments[3:5]
        endpoint = endpoint.split("?", 1)[0]
        payload = json.loads(str(kwargs["input"])) if kwargs.get("input") else None
        source = self.completion.source
        source_pr = f"repos/{source.repository.name}/pulls/{source.number}"
        source_issue = f"repos/{source.repository.name}/issues/{source.number}"
        upstream_pr = f"repos/{UV_REPOSITORY.name}/pulls/{UPSTREAM.number}"
        upstream_issue = f"repos/{UV_REPOSITORY.name}/issues/{UPSTREAM.number}"
        operation = f"{method} {endpoint}"
        self.calls[operation] += 1
        self.trace.append(("api", operation))
        failures = self.failures.get(operation, [])
        failure = failures.pop(0) if failures else ""

        def respond(result: object = None) -> subprocess.CompletedProcess[str]:
            body = json.dumps(result)
            status_match = re.search(r"HTTP ([1-5][0-9]{2})", failure)
            status = int(status_match[1]) if status_match else (503 if failure else 200)
            if "--include" in arguments:
                header = self.headers.get(operation, "")
                body = f"HTTP/2.0 {status} Status\r\n{header}\r\n{body}"
            if failure:
                self.trace.append(("failed", operation))
            return subprocess.CompletedProcess(
                arguments, int(bool(failure)), body, failure
            )

        if failure.startswith("before"):
            return respond()
        match method, endpoint:
            case "GET", value if value == source_pr:
                result = self.source
            case "GET", value if value == upstream_pr:
                result = self.upstream
            case "GET", value if value == f"{source_issue}/events":
                result = self.events
            case "GET", value if value == f"{source_issue}/comments":
                result = self.comments
            case "GET", value if value == f"repos/{source.repository.name}":
                result = repository(source)
            case "GET", value if value.endswith("/collaborators/zanieb/permission"):
                result = {"permission": self.permission, "user": human()}
            case "POST", value if value == f"{upstream_issue}/labels":
                if payload != {"labels": ["internal"]}:
                    raise AssertionError(payload)
                if self.drift_after_labels:
                    self.upstream["head"] = {
                        **as_object(self.upstream["head"]),
                        "sha": str(OTHER),
                    }
                result = []
            case "POST", value if value == f"{upstream_issue}/assignees":
                if payload != {"assignees": ["zanieb"]}:
                    raise AssertionError(payload)
                result = self.upstream
            case "POST", value if value == f"{source_issue}/comments":
                if payload != {"body": self.completion.comment}:
                    raise AssertionError(payload)
                result = self.receipt()
                self.comments.append(result)
                if self.close_after_record:
                    self.source["state"] = "closed"
                if self.comment_response is not None:
                    result = self.comment_response
            case "PATCH", value if value == source_pr:
                if payload != {"state": "closed"}:
                    raise AssertionError(payload)
                self.source["state"] = "closed"
                result = self.source
            case _:
                raise AssertionError((method, endpoint, payload))
        return respond(result)

    def operation(self, method: str, resource: str) -> str:
        source = self.completion.source
        if resource == "close":
            return f"{method} repos/{source.repository.name}/pulls/{source.number}"
        if resource == "comments":
            return f"{method} repos/{source.repository.name}/issues/{source.number}/comments"
        return (
            f"{method} repos/{UV_REPOSITORY.name}/issues/{UPSTREAM.number}/{resource}"
        )

    def execute(self, *, close: bool = False) -> SkippedCompletion | None:
        def wait(delay: float) -> None:
            self.delays.append(delay)
            self.trace.append(("sleep", delay))

        with (
            patch(
                "uv_automations.github_promotion.subprocess.run", side_effect=self.run
            ),
            patch(
                "uv_automations.workflows.promotion_completion.sleep",
                side_effect=wait,
            ),
        ):
            function = close_source if close else complete_metadata
            return function(
                PromotionCompletionGitHub(),
                PromotionCompletionGitHub(),
                self.completion,
            )


class PromotionCompletionTests(unittest.TestCase):
    def assert_delayed_after_failures(
        self, fixture: FaultInjection, delay: float
    ) -> None:
        failures = [
            index for index, event in enumerate(fixture.trace) if event[0] == "failed"
        ]
        self.assertTrue(failures)
        for index in failures:
            self.assertEqual(fixture.trace[index + 1], ("sleep", delay))
            self.assertEqual(fixture.trace[index + 2][0], "api")

    def test_mutation_reconciliation_obeys_retry_after_before_every_api_call(
        self,
    ) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for resource, status, header in (
            (resource, status, header)
            for resource in ("comments", "close")
            for status in (429, 503)
            for header in ("20", "Thu, 24 Sep 2026 12:00:20 GMT")
        ):
            with self.subTest(resource=resource, status=status, header=header):
                fixture = FaultInjection()
                operation = fixture.operation(
                    "PATCH" if resource == "close" else "POST", resource
                )
                fixture.failures[operation] = [f"after HTTP {status}"]
                fixture.headers[operation] = f"Retry-After: {header}\r\n"
                with patch(
                    "uv_automations.github_promotion_completion.datetime"
                ) as clock:
                    clock.now.return_value = now
                    self.assertIsNone(fixture.execute(close=True))
                self.assert_delayed_after_failures(fixture, 20.0)
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )
                self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)

    def test_reconciliation_get_and_final_attempt_obey_their_delays(self) -> None:
        fixture = FaultInjection()
        operation = fixture.operation("PATCH", "close")
        fixture.failures[operation] = [
            "before HTTP 503",
            "before HTTP 503",
            "after HTTP 429",
        ]
        fixture.headers[operation] = "Retry-After: 20\r\n"
        self.assertIsNone(fixture.execute(close=True))
        self.assert_delayed_after_failures(fixture, 20.0)
        self.assertEqual(fixture.calls[operation], 3)

        fixture = FaultInjection()
        operation = fixture.operation("GET", "comments")
        # The first read proves absence; the second is the post-POST readback.
        fixture.failures[operation] = ["", "before HTTP 429"]
        fixture.headers[operation] = "Retry-After: 6\r\n"
        self.assertIsNone(fixture.execute(close=True))
        self.assert_delayed_after_failures(fixture, 6.0)
        self.assertEqual(fixture.calls[fixture.operation("POST", "comments")], 1)

        fixture = FaultInjection()
        source_read = fixture.operation("GET", "close")
        comment_write = fixture.operation("POST", "comments")
        fixture.failures[source_read] = ["before HTTP 503", "before HTTP 503"]
        fixture.failures[comment_write] = ["after HTTP 429"]
        fixture.headers[source_read] = "Retry-After: 20\r\n"
        fixture.headers[comment_write] = "Retry-After: 20\r\n"
        self.assertIsNone(fixture.execute(close=True))
        self.assert_delayed_after_failures(fixture, 20.0)
        self.assertEqual(fixture.calls[comment_write], 1)

    def test_over_budget_or_permanent_mutation_failure_has_no_followup_request(
        self,
    ) -> None:
        for resource, status, header in (
            (resource, status, header)
            for resource in ("comments", "close")
            for status, header in (
                (429, "31"),
                (503, "999999999999999999"),
                (401, ""),
                (422, ""),
            )
        ):
            with self.subTest(resource=resource, status=status, header=header):
                fixture = FaultInjection()
                operation = fixture.operation(
                    "PATCH" if resource == "close" else "POST", resource
                )
                fixture.failures[operation] = [f"after HTTP {status}"]
                if header:
                    fixture.headers[operation] = f"Retry-After: {header}\r\n"
                with self.assertRaises(CompletionRequestError):
                    fixture.execute(close=True)
                self.assertEqual(fixture.trace[-1], ("failed", operation))
                self.assertEqual(fixture.calls[operation], 1)
                self.assertFalse(fixture.delays)

        for status, header in ((429, "31"), (401, "")):
            with self.subTest(read_status=status):
                fixture = FaultInjection()
                operation = fixture.operation("GET", "comments")
                fixture.failures[operation] = ["", f"before HTTP {status}"]
                if header:
                    fixture.headers[operation] = f"Retry-After: {header}\r\n"
                with self.assertRaises(CompletionRequestError):
                    fixture.execute(close=True)
                self.assertEqual(fixture.trace[-1], ("failed", operation))
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )
                self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)
                self.assertFalse(fixture.delays)

    def test_retry_after_parsing_is_bounded_and_sanitized(self) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for value, expected in (
            (None, None),
            ("0", 0.0),
            ("12", 12.0),
            ("Thu, 24 Sep 2026 12:00:09 GMT", 9.0),
            ("Thu, 24 Sep 2026 11:59:59 GMT", 0.0),
            ("Thu, 24 Sep 2026 12:00:09", None),
            ("-1", None),
            ("1.5", None),
            ("NaN", None),
            ("１２", None),
            ("x" * 101, None),
        ):
            with self.subTest(value=value):
                self.assertEqual(_retry_after(value, now), expected)
        self.assertFalse(
            CompletionRequestError(429, _retry_after("9" * 80, now)).retryable
        )
        self.assertEqual(
            _response("HTTP/2.0 429 Too Many\r\nretry-after: 3\r\n\r\nprivate"),
            (429, "3", "private"),
        )
        with self.assertRaises(CompletionRequestError) as caught:
            _response(
                "HTTP/2.0 429 Too Many\nRetry-After: 3\nretry-after: 4\n\nprivate"
            )
        self.assertFalse(caught.exception.retryable)
        self.assertEqual(
            _response("private token without an HTTP response"), (None, None, "")
        )

    def test_rate_limit_delay_and_permanent_errors_do_not_escape_the_budget(
        self,
    ) -> None:
        for status in (403, 429, 503):
            with self.subTest(status=status):
                fixture = FaultInjection()
                operation = fixture.operation("POST", "labels")
                fixture.failures[operation] = [f"before HTTP {status}"]
                fixture.headers[operation] = "Retry-After: 7\r\n"
                self.assertIsNone(fixture.execute())
                self.assertEqual(fixture.delays, [7.0])
        for status, header in (
            (401, ""),
            (403, ""),
            (422, ""),
            (429, "Retry-After: 31\r\n"),
        ):
            with self.subTest(status=status, header=header):
                fixture = FaultInjection()
                operation = fixture.operation("POST", "labels")
                fixture.failures[operation] = [f"before HTTP {status} private-token"]
                fixture.headers[operation] = header
                with self.assertRaises(CompletionRequestError) as caught:
                    fixture.execute()
                self.assertFalse(caught.exception.retryable)
                self.assertNotIn("private-token", str(caught.exception))
                self.assertEqual(fixture.calls[operation], 1)
                self.assertFalse(fixture.delays)

    def test_authority_reads_use_the_same_retry_after_boundary(self) -> None:
        fixture = FaultInjection()
        operation = fixture.operation("GET", "close")
        fixture.failures[operation] = ["before HTTP 429"]
        fixture.headers[operation] = "Retry-After: 5\r\n"
        self.assertIsNone(fixture.execute())
        self.assertEqual(fixture.delays, [5.0])
        fixture = FaultInjection()
        operation = fixture.operation("GET", "close")
        fixture.failures[operation] = ["before HTTP 401"]
        with self.assertRaises(CompletionRequestError):
            fixture.execute()
        self.assertEqual(fixture.calls[operation], 1)
        self.assertFalse(fixture.delays)
        self.assertFalse(
            any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
        )

    def test_cli_binds_scope_and_emits_completion_only_after_success(self) -> None:
        for name, close in (("complete-metadata", False), ("close-source", True)):
            with self.subTest(name=name):
                command = cli.parse_command(
                    cli.create_parser(),
                    [
                        "promotions",
                        name,
                        "--repo",
                        str(SOURCE.repository.name),
                        "--repository-id",
                        str(SOURCE.repository.database_id),
                        "--pull-request",
                        str(SOURCE.number),
                        "--github-output",
                        "output",
                        "--summary",
                        "summary",
                    ],
                )
                self.assertEqual(
                    command,
                    promotions_cli.CompletePromotion(
                        source=SOURCE,
                        close=close,
                        github_output=Path("output"),
                        summary=Path("summary"),
                    ),
                )
                with (
                    patch.object(
                        promotions_cli, "_read_json", return_value=COMPLETION.to_json()
                    ),
                    patch.object(
                        promotions_cli, "complete_metadata", return_value=None
                    ) as metadata,
                    patch.object(
                        promotions_cli, "close_source", return_value=None
                    ) as closing,
                    patch.object(promotions_cli, "write_output") as output,
                    patch.object(promotions_cli, "write_json_output") as json_output,
                    patch.object(promotions_cli, "append_summary"),
                ):
                    cli.run(command)
                if close:
                    closing.assert_called_once()
                    metadata.assert_not_called()
                    json_output.assert_called_once_with(Path("output"), "closed", True)
                else:
                    metadata.assert_called_once()
                    closing.assert_not_called()
                    output.assert_called_once_with(Path("output"), "number", "21948")
                    json_output.assert_called_once_with(
                        Path("output"), "completion", COMPLETION.to_json()
                    )
        wrong = promotions_cli.CompletePromotion(
            source=replace(SOURCE, number=9),
            close=False,
            github_output=Path("output"),
            summary=Path("summary"),
        )
        with (
            patch.object(
                promotions_cli, "_read_json", return_value=COMPLETION.to_json()
            ),
            patch.object(promotions_cli, "complete_metadata") as metadata,
            self.assertRaisesRegex(ValueError, "another source"),
        ):
            cli.run(wrong)
        metadata.assert_not_called()

    def test_claim_round_trip_and_identity_rejection(self) -> None:
        self.assertEqual(
            PromotionCompletion.from_json(COMPLETION.to_json()), COMPLETION
        )
        for field, value in (
            ("version", True),
            ("approval_id", False),
            ("replay_approval_id", 2),
            ("promoted_head", str(OTHER)),
            ("action", "reopen"),
        ):
            with self.subTest(field=field), self.assertRaises(ValueError):
                PromotionCompletion.from_json({**COMPLETION.to_json(), field: value})

    def test_metadata_retries_the_observed_404_and_503_failures(self) -> None:
        for resource, failure in (
            ("labels", "before HTTP 404"),
            ("labels", "before HTTP 503"),
            ("assignees", "before HTTP 503"),
            ("labels", "after lost response"),
            ("assignees", "after lost response"),
            ("labels", "after HTTP 200 lost response"),
        ):
            with self.subTest(resource=resource, failure=failure):
                fixture = FaultInjection()
                operation = fixture.operation("POST", resource)
                fixture.failures[operation] = [failure]
                self.assertIsNone(fixture.execute())
                self.assertEqual(fixture.calls[operation], 2)
                self.assertFalse(fixture.comments)

    def test_metadata_retry_budget_and_changed_upstream_fail_closed(self) -> None:
        fixture = FaultInjection()
        operation = fixture.operation("POST", "labels")
        fixture.failures[operation] = ["before HTTP 503"] * 3
        with self.assertRaises(PromotionReadError):
            fixture.execute()
        self.assertEqual(fixture.calls[operation], 3)
        self.assertEqual(fixture.calls[fixture.operation("POST", "assignees")], 0)
        fixture = FaultInjection()
        fixture.drift_after_labels = True
        self.assertIsInstance(fixture.execute(), SkippedCompletion)
        self.assertEqual(fixture.calls[fixture.operation("POST", "assignees")], 0)

    def test_lost_comment_response_is_reconciled_without_reposting(self) -> None:
        fixture = FaultInjection()
        operation = fixture.operation("POST", "comments")
        fixture.failures[operation] = ["after lost response"]
        self.assertIsNone(fixture.execute(close=True))
        self.assertEqual(fixture.calls[operation], 1)
        self.assertEqual(len(fixture.comments), 1)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)

    def test_committed_comment_with_invalid_response_reconciles_safely(self) -> None:
        for response in (
            {},
            {"user": {"type": "private-invalid-actor"}},
            {**FaultInjection().receipt(), "performed_via_github_app": None},
            {**FaultInjection().receipt(), "body": "private-unexpected-body"},
        ):
            with self.subTest(response=response):
                fixture = FaultInjection()
                fixture.comment_response = response
                self.assertIsNone(fixture.execute(close=True))
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )
                self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)
                self.assertEqual(fixture.delays, [1.0])

    def test_uncommitted_comment_failure_is_not_blindly_retried(self) -> None:
        fixture = FaultInjection()
        operation = fixture.operation("POST", "comments")
        fixture.failures[operation] = ["before HTTP 503"]
        with self.assertRaises(PromotionReadError):
            fixture.execute(close=True)
        self.assertEqual(fixture.calls[operation], 1)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)

    def test_close_failure_before_or_after_commit_reuses_one_receipt(self) -> None:
        for failures, closes in (
            (["before HTTP 503"], 2),
            (["after lost response"], 1),
            (["before HTTP 503", "before HTTP 503", "after lost response"], 3),
        ):
            with self.subTest(failures=failures):
                fixture = FaultInjection()
                operation = fixture.operation("PATCH", "close")
                fixture.failures[operation] = list(failures)
                self.assertIsNone(fixture.execute(close=True))
                self.assertEqual(fixture.calls[operation], closes)
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )

    def test_existing_canonical_receipt_is_not_duplicated(self) -> None:
        fixture = FaultInjection()
        fixture.comments.append(fixture.receipt())
        self.assertIsNone(fixture.execute(close=True))
        self.assertEqual(fixture.calls[fixture.operation("POST", "comments")], 0)

    def test_receipt_provenance_and_concurrent_close_do_not_grant_authority(
        self,
    ) -> None:
        for field, value in (("user", human()), ("performed_via_github_app", None)):
            with self.subTest(field=field):
                fixture = FaultInjection()
                fixture.comments.append({**fixture.receipt(), field: value})
                self.assertIsNone(fixture.execute(close=True))
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )
        fixture = FaultInjection()
        fixture.comments.append(
            {**fixture.receipt(), "updated_at": "2026-09-09T12:02:00Z"}
        )
        fixture.events[0]["id"] = 2
        self.assertIsInstance(fixture.execute(close=True), SkippedCompletion)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)
        fixture = FaultInjection()
        fixture.close_after_record = True
        self.assertIsInstance(fixture.execute(close=True), SkippedCompletion)
        self.assertEqual(fixture.calls[fixture.operation("POST", "comments")], 1)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)

    def test_different_receipt_and_initially_closed_source_do_not_write(self) -> None:
        fixture = FaultInjection()
        fixture.comments.append(
            fixture.receipt("Promoted to [#9](https://github.com/astral-sh/uv/pull/9).")
        )
        self.assertIsInstance(fixture.execute(close=True), SkippedCompletion)
        self.assertEqual(fixture.calls[fixture.operation("POST", "comments")], 0)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)
        fixture = FaultInjection()
        fixture.source["state"] = "closed"
        fixture.comments.append(fixture.receipt())
        self.assertIsInstance(fixture.execute(close=True), SkippedCompletion)
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 0)

    def test_source_approval_base_head_and_upstream_identity_drift_do_not_write(
        self,
    ) -> None:
        for target, field, value in (
            ("source", "state", "closed"),
            ("source", "draft", True),
            ("source", "number", 8),
            ("upstream", "state", "closed"),
            ("upstream", "number", 8),
        ):
            with self.subTest(target=target, field=field):
                fixture = FaultInjection()
                getattr(fixture, target)[field] = value
                try:
                    self.assertIsInstance(fixture.execute(), SkippedCompletion)
                except PromotionReadError:
                    pass
                self.assertFalse(
                    any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
                )
        for target, revision, field, value in (
            ("source", "base", "ref", "other"),
            ("source", "head", "ref", "other"),
            ("source", "head", "sha", str(OTHER)),
            ("source", "head", "repo", repository(UPSTREAM)),
            ("upstream", "base", "ref", "other"),
            ("upstream", "head", "sha", str(OTHER)),
            ("upstream", "head", "repo", repository(SOURCE)),
        ):
            with self.subTest(target=target, revision=revision, field=field):
                fixture = FaultInjection()
                data = getattr(fixture, target)
                data[revision] = {**as_object(data[revision]), field: value}
                self.assertIsInstance(fixture.execute(), SkippedCompletion)
                self.assertFalse(
                    any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
                )
        fixture = FaultInjection()
        fixture.events[0]["id"] = 2
        self.assertIsInstance(fixture.execute(), SkippedCompletion)
        self.assertFalse(
            any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
        )

    def test_update_parent_requires_exact_source_base(self) -> None:
        fixture = FaultInjection(
            replace(
                COMPLETION, action=PublicationAction.UPDATE_PARENT, promoted_head=OTHER
            )
        )
        fixture.source["base"] = {
            **as_object(fixture.source["base"]),
            "sha": str(OTHER),
        }
        self.assertIsInstance(fixture.execute(), SkippedCompletion)
        self.assertFalse(
            any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
        )

    def test_update_parent_keeps_source_and_published_revisions_distinct(self) -> None:
        completion = replace(
            COMPLETION, action=PublicationAction.UPDATE_PARENT, promoted_head=OTHER
        )
        fixture = FaultInjection(completion)
        fixture.upstream["head"] = {
            **as_object(fixture.upstream["head"]),
            "sha": str(OTHER),
        }
        self.assertIsNone(fixture.execute())
        self.assertIsNone(fixture.execute(close=True))
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)

    def test_private_label_requires_current_writer_permission(self) -> None:
        completion = replace(
            COMPLETION,
            source=PromotionScope(UV_SECURITY_REPOSITORY, SOURCE.number),
            source_base=replace(
                COMPLETION.source_base, repository=UV_SECURITY_REPOSITORY
            ),
            source_head=replace(
                COMPLETION.source_head, repository=UV_SECURITY_REPOSITORY
            ),
            approval_kind=PromotionApprovalKind.LABELED,
        )
        fixture = FaultInjection(completion)
        fixture.events[0].update(event="labeled", label={"name": "bot:promote"})
        fixture.permission = "read"
        self.assertIsInstance(fixture.execute(), SkippedCompletion)
        self.assertFalse(
            any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
        )


if __name__ == "__main__":
    unittest.main()
