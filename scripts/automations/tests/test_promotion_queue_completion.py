import copy
import io
import json
import os
import re
import shlex
import subprocess
import unittest
from collections import Counter
from collections.abc import Iterator
from contextlib import contextmanager, redirect_stderr
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from test_promotion_closed import (
    COMMENT_ID,
    HEAD,
    NOW,
    OTHER,
    SOURCE,
    ObservationFixture,
)
from test_promotion_completion import human
from test_promotion_queue import FakeGitHub, source_queue
from test_promotion_wakeup import offline_process, outputs
from test_promotion_workflow_boundaries import job

from uv_automations import cli
from uv_automations.github_promotion import (
    PromotionGitHub,
    decode_promotion_pull_request,
)
from uv_automations.github_promotion_completion import PromotionCompletionGitHub
from uv_automations.github_promotion_queue import (
    PromotionQueueCompletionGitHub,
    PromotionQueueGitHub,
)
from uv_automations.json import as_object
from uv_automations.models import ActorKind, Timestamp
from uv_automations.promotion_models import (
    AUTOMATIONS_APP_ID,
    AUTOMATIONS_APP_SLUG,
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalKind,
    PromotionScope,
    ReadyForReviewEvent,
)
from uv_automations.workflows.promotion_completion import CompletionRequestError
from uv_automations.workflows.promotion_queue import (
    PendingSourceParent,
    QueuedPromotion,
    QueueRecordOutcome,
    record_queue,
)

RUN = subprocess.run


class QueueReceiptFixture(ObservationFixture):
    """Use the real queue decoders and transport with a local GitHub boundary."""

    def __init__(self) -> None:
        super().__init__()
        self.source.update(state="open", user=human())
        self.events: list[dict[str, object]] = [
            {"id": 1, "event": "ready_for_review", "actor": human(), "created_at": NOW}
        ]
        source = decode_promotion_pull_request(self.source, SOURCE)
        event = ReadyForReviewEvent(
            1, PromotionActor("zanieb", 1, ActorKind.USER), Timestamp.parse(NOW)
        )
        parent = decode_promotion_pull_request(
            {
                **self.source,
                "number": 10,
                "html_url": "https://github.com/astral-sh/uv-dev/pull/10",
            },
            PromotionScope(UV_DEV_REPOSITORY, 10),
        )
        self.queued = QueuedPromotion.waiting_for_parent(
            source, PromotionApproval(SOURCE, HEAD, event, 1), parent
        )
        self.comments = []
        self.direct["body"] = self.queued.comment()
        self.graph["body"] = self.queued.comment()
        self.failures: dict[str, list[str]] = {}
        self.headers: dict[str, str] = {}
        self.wire_calls: Counter[str] = Counter()
        self.wire_trace: list[tuple[str, str | float]] = []
        self.comment_response: object | None = None
        self.source_after_post: dict[str, object] | None = None
        self.events_after_post: list[dict[str, object]] | None = None
        self.events_after_proof: list[dict[str, object]] | None = None
        self.pages_after_post: dict[int, list[object]] | None = None
        self.pages_after_proof: dict[int, list[object]] | None = None
        self.authority_failures_after_proof: list[str] = []
        self.awaiting_authority_read = False
        self.wire_credentials: list[tuple[str, str | None]] = []

    @property
    def comment_write(self) -> str:
        return f"POST repos/{SOURCE.repository.name}/issues/{SOURCE.number}/comments"

    @property
    def comment_list(self) -> str:
        return f"GET repos/{SOURCE.repository.name}/issues/{SOURCE.number}/comments"

    @property
    def source_read(self) -> str:
        return f"GET repos/{SOURCE.repository.name}/pulls/{SOURCE.number}"

    @override
    def api(
        self, method: str, endpoint: str, *, payload: object | None = None
    ) -> object:
        path = endpoint.split("?", 1)[0]
        if (
            f"{method} {path}" == self.comment_list
            and self.receipt_read
            and self.pages_after_proof is not None
        ):
            page = int(endpoint.rsplit("page=", 1)[1])
            return copy.deepcopy(self.pages_after_proof.get(page, []))
        if method == "GET" and path == (
            f"repos/{SOURCE.repository.name}/issues/{SOURCE.number}/events"
        ):
            return copy.deepcopy(
                self.events_after_proof
                if self.receipt_read and self.events_after_proof is not None
                else self.events
            )
        if f"{method} {path}" == self.comment_write:
            if payload != {"body": self.queued.comment()}:
                raise AssertionError("Unexpected queue comment")
            self.comments.append(copy.deepcopy(self.direct))
            if self.source_after_post is not None:
                self.source = copy.deepcopy(self.source_after_post)
            if self.events_after_post is not None:
                self.events = copy.deepcopy(self.events_after_post)
            if self.pages_after_post is not None:
                self.pages = copy.deepcopy(self.pages_after_post)
            return copy.deepcopy(
                self.direct if self.comment_response is None else self.comment_response
            )
        return super().api(method, endpoint, payload=payload)

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
        operation = f"{method} {endpoint.split('?', 1)[0]}"
        environment = kwargs.get("env")
        token = (
            as_object(environment).get("GH_TOKEN")
            if environment is not None
            else os.environ.get("GH_TOKEN")
        )
        if token not in {"local-read", "local-write"}:
            raise AssertionError("Unexpected credential source")
        if operation == self.comment_write and token != "local-write":
            raise AssertionError("Queue write used the read credential")
        self.wire_credentials.append((operation, str(token)))
        payload = json.loads(str(kwargs["input"])) if kwargs.get("input") else None
        self.wire_calls[operation] += 1
        self.wire_trace.append(("api", operation))
        if operation == self.source_read and self.awaiting_authority_read:
            self.awaiting_authority_read = False
            failures = self.authority_failures_after_proof or self.failures.get(
                operation, []
            )
        else:
            failures = self.failures.get(operation, [])
        failure = failures.pop(0) if failures else ""
        result = (
            None
            if failure.startswith("before")
            else self.api(method, endpoint, payload=payload)
        )
        status_match = re.search(r"HTTP ([1-5][0-9]{2})", failure)
        status = int(status_match[1]) if status_match else (503 if failure else 200)
        body = json.dumps(result)
        if "--include" in arguments:
            body = f"HTTP/2.0 {status} Status\r\n{self.headers.get(operation, '')}\r\n{body}"
        if failure:
            self.wire_trace.append(("failed", operation))
        elif operation == "POST graphql":
            # Event collection also reads the source. Target the first source
            # read after the receipt proof, not a guessed global call count.
            self.awaiting_authority_read = True
        return subprocess.CompletedProcess(arguments, int(bool(failure)), body, failure)

    @contextmanager
    @override
    def offline(self) -> Iterator[None]:
        def wait(delay: float) -> None:
            self.wire_trace.append(("sleep", delay))

        with (
            offline_process(),
            patch.dict(
                os.environ, {"GH_READ_TOKEN": "local-read", "GH_TOKEN": "local-write"}
            ),
            patch("subprocess.run", side_effect=self.run),
            patch(
                "uv_automations.workflows.promotion_completion.sleep", side_effect=wait
            ),
        ):
            yield

    def invoke(self, root: Path) -> tuple[int, str, dict[str, str]]:
        section = job("promote-pull-request.yml", "queue", "replay-queued-promotion")
        script = section.rsplit("        run: |\n", 1)[1]
        command, separator, input_value = script.strip().rpartition(" <<< ")
        if not separator or input_value != '"$QUEUE_PLAN"':
            raise AssertionError("Unexpected queue input")
        arguments = shlex.split(command.replace("\\\n", ""))
        if arguments[:4] != ["$AUTOMATIONS_PYTHON", "-I", "-m", "uv_automations"]:
            raise AssertionError("Unexpected queue entry point")
        environment = {
            "GITHUB_REPOSITORY": str(SOURCE.repository.name),
            "GITHUB_REPOSITORY_ID": str(SOURCE.repository.database_id),
            "PULL_REQUEST_NUMBER": str(SOURCE.number),
            "GITHUB_OUTPUT": str(root / "output"),
            "GITHUB_STEP_SUMMARY": str(root / "summary"),
        }
        arguments = [
            environment.get(value.removeprefix("$"), value) for value in arguments[4:]
        ]
        stderr = io.StringIO()
        code = 0
        with (
            self.offline(),
            patch("sys.stdin", io.StringIO(json.dumps(self.queued.to_json()))),
            redirect_stderr(stderr),
        ):
            try:
                cli.main(arguments)
            except SystemExit as error:
                if type(error.code) is not int:
                    raise AssertionError("Unexpected CLI exit") from error
                code = error.code
        output = root / "output"
        return code, stderr.getvalue(), outputs(output) if output.exists() else {}


class PromotionQueueCompletionTests(unittest.TestCase):
    def assert_delayed_after_failures(
        self, fixture: QueueReceiptFixture, delay: float
    ) -> None:
        failures = [
            index
            for index, event in enumerate(fixture.wire_trace)
            if event[0] == "failed"
        ]
        self.assertTrue(failures)
        for index in failures:
            self.assertEqual(fixture.wire_trace[index + 1], ("sleep", delay))
            self.assertEqual(fixture.wire_trace[index + 2][0], "api")

    def assert_not_recorded(self, fixture: QueueReceiptFixture) -> None:
        with TemporaryDirectory() as temporary:
            code, stderr, output = fixture.invoke(Path(temporary))
        self.assertEqual(code, 1, stderr)
        self.assertEqual(output, {})
        self.assertNotIn("private", stderr)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_committed_queue_with_lost_response_completes_without_another_post(
        self,
    ) -> None:
        fixture = QueueReceiptFixture()
        fixture.failures[fixture.comment_write] = ["after lost response"]
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
            )
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)
        self.assertEqual(len(fixture.comments), 1)

    def test_committed_queue_with_malformed_success_completes_without_leaking_fields(
        self,
    ) -> None:
        for response in (
            {},
            {"body": "private response"},
            {"user": {"type": "private-invalid-actor"}},
            {**QueueReceiptFixture().direct, "performed_via_github_app": None},
            {**QueueReceiptFixture().direct, "body": "private response"},
        ):
            with self.subTest(response=response):
                fixture = QueueReceiptFixture()
                fixture.comment_response = response
                with TemporaryDirectory() as temporary:
                    self.assertEqual(
                        fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
                    )
                self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_queue_transport_is_scoped_to_receipt_completion(self) -> None:
        self.assertIs(PromotionQueueGitHub._api, PromotionGitHub._api)
        self.assertIs(
            PromotionQueueCompletionGitHub._api, PromotionCompletionGitHub._api
        )
        fixture = QueueReceiptFixture()
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
            )
        self.assertEqual(
            fixture.wire_credentials[0], (fixture.source_read, "local-read")
        )
        self.assertIn((fixture.comment_write, "local-write"), fixture.wire_credentials)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_mutation_and_edit_proof_failures_obey_retry_after_before_readback(
        self,
    ) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for target in ("write", "list", "direct", "graphql"):
            for status, header in ((429, "20"), (503, "Thu, 24 Sep 2026 12:00:20 GMT")):
                with self.subTest(target=target, status=status, header=header):
                    fixture = QueueReceiptFixture()
                    operation = {
                        "write": fixture.comment_write,
                        "list": fixture.comment_list,
                        "direct": f"GET repos/{SOURCE.repository.name}/issues/comments/{COMMENT_ID}",
                        "graphql": "POST graphql",
                    }[target]
                    fixture.failures[operation] = ([""] if target == "list" else []) + [
                        f"{'after' if target == 'write' else 'before'} HTTP {status}"
                    ]
                    fixture.headers[operation] = f"Retry-After: {header}\r\n"
                    with patch(
                        "uv_automations.github_promotion_completion.datetime"
                    ) as clock:
                        clock.now.return_value = now
                        with TemporaryDirectory() as temporary:
                            self.assertEqual(
                                fixture.invoke(Path(temporary)),
                                (0, "", {"recorded": "true"}),
                            )
                    self.assert_delayed_after_failures(fixture, 20.0)
                    self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_final_attempt_still_reconciles_one_ambiguous_write(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.failures[fixture.source_read] = ["before HTTP 503"] * 2
        fixture.failures[fixture.comment_write] = ["after HTTP 429"]
        fixture.headers[fixture.source_read] = "Retry-After: 6\r\n"
        fixture.headers[fixture.comment_write] = "Retry-After: 6\r\n"
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
            )
        self.assert_delayed_after_failures(fixture, 6.0)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_zero_invalid_and_duplicate_retry_after_headers_are_bounded(self) -> None:
        for header in ("0", "NaN"):
            with self.subTest(header=header):
                fixture = QueueReceiptFixture()
                fixture.failures[fixture.comment_write] = ["after HTTP 503"]
                fixture.headers[fixture.comment_write] = f"Retry-After: {header}\r\n"
                with TemporaryDirectory() as temporary:
                    self.assertEqual(
                        fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
                    )
                self.assert_delayed_after_failures(fixture, 1.0)
        fixture = QueueReceiptFixture()
        fixture.failures[fixture.comment_write] = ["after HTTP 503"]
        fixture.headers[fixture.comment_write] = "Retry-After: 1\r\nRetry-After: 2\r\n"
        self.assert_not_recorded(fixture)
        self.assertEqual(fixture.wire_trace[-1], ("failed", fixture.comment_write))

    def test_permanent_or_over_budget_failures_have_no_followup_call(self) -> None:
        for target in ("write", "source", "proof"):
            for status, header in (
                (429, "31"),
                (503, "999999999999999999"),
                (401, ""),
                (422, ""),
            ):
                with self.subTest(target=target, status=status, header=header):
                    fixture = QueueReceiptFixture()
                    operation = {
                        "write": fixture.comment_write,
                        "source": fixture.source_read,
                        "proof": "POST graphql",
                    }[target]
                    fixture.failures[operation] = [
                        f"{'after' if target == 'write' else 'before'} HTTP {status}"
                    ]
                    if header:
                        fixture.headers[operation] = f"Retry-After: {header}\r\n"
                    with TemporaryDirectory() as temporary:
                        code, _, output = fixture.invoke(Path(temporary))
                    self.assertEqual((code, output), (1, {}))
                    self.assertEqual(fixture.wire_trace[-1], ("failed", operation))
                    self.assertFalse(
                        any(event[0] == "sleep" for event in fixture.wire_trace)
                    )
                    self.assertEqual(
                        fixture.wire_calls[fixture.comment_write],
                        0 if target == "source" else 1,
                    )

    def test_uncommitted_write_is_not_repeated(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.failures[fixture.comment_write] = ["before HTTP 503"]
        self.assert_not_recorded(fixture)
        self.assertEqual(fixture.comments, [])

    def test_counterfeit_or_edited_committed_receipt_cannot_complete(self) -> None:
        for target, field, value in (
            ("direct", "user", human()),
            (
                "direct",
                "user",
                {"type": "Bot", "login": "other[bot]", "id": AUTOMATIONS_BOT_ID + 1},
            ),
            ("direct", "performed_via_github_app", None),
            (
                "direct",
                "performed_via_github_app",
                {"id": AUTOMATIONS_APP_ID + 1, "slug": AUTOMATIONS_APP_SLUG},
            ),
            ("direct", "body", "private forged receipt"),
            ("graph", "lastEditedAt", NOW),
            ("graph", "editor", {"__typename": "User"}),
            ("graph", "body", "private edited receipt"),
            ("graph", "fullDatabaseId", str(COMMENT_ID + 1)),
        ):
            with self.subTest(target=target, field=field):
                fixture = QueueReceiptFixture()
                getattr(fixture, target)[field] = value
                fixture.failures[fixture.comment_write] = ["after lost response"]
                self.assert_not_recorded(fixture)

    def test_post_write_comment_collection_stays_bounded_and_unambiguous(self) -> None:
        receipt = QueueReceiptFixture().direct
        page_cases: tuple[dict[int, list[object]], ...] = (
            {1: [None]},
            {1: [receipt] * 2},
            {
                page: [
                    {**receipt, "id": COMMENT_ID + page * 100 + index}
                    for index in range(100)
                ]
                for page in range(1, 11)
            },
        )
        for pages in page_cases:
            with self.subTest(page_count=len(pages)):
                fixture = QueueReceiptFixture()
                fixture.pages_after_post = pages
                fixture.failures[fixture.comment_write] = ["after lost response"]
                self.assert_not_recorded(fixture)
                self.assertLessEqual(fixture.wire_calls[fixture.comment_list], 31)

    def test_exact_source_and_approval_are_rechecked_after_receipt_proof(self) -> None:
        for revision, field, value in (
            (None, "state", "closed"),
            (None, "draft", True),
            ("base", "ref", "other"),
            ("base", "sha", str(OTHER)),
            ("head", "ref", "other"),
            ("head", "sha", str(OTHER)),
            ("head", "repo", {"full_name": "astral-sh/uv", "id": 699532645}),
        ):
            with self.subTest(revision=revision, field=field):
                fixture = QueueReceiptFixture()
                fixture.source_after = copy.deepcopy(fixture.source)
                target = (
                    fixture.source_after
                    if revision is None
                    else as_object(fixture.source_after[revision])
                )
                target[field] = value
                fixture.failures[fixture.comment_write] = ["after lost response"]
                with TemporaryDirectory() as temporary:
                    self.assertEqual(
                        fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
                    )
                self.assertTrue(fixture.receipt_read)
                self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

        fixture = QueueReceiptFixture()
        fixture.events_after_proof = [{**fixture.events[0], "id": 2}]
        fixture.failures[fixture.comment_write] = ["after lost response"]
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
            )
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_label_approval_drift_during_receipt_proof_stays_stale(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.source["labels"] = [{"name": "bot:promote"}]
        fixture.events.append(
            {
                "id": 2,
                "event": "labeled",
                "actor": human(),
                "label": {"name": "bot:promote"},
                "created_at": NOW,
            }
        )
        fixture.queued = replace(
            fixture.queued,
            approval=replace(
                fixture.queued.approval,
                kind=PromotionApprovalKind.LABELED,
                event_id=2,
                ready_event_id=None,
            ),
        )
        fixture.direct["body"] = fixture.queued.comment()
        fixture.graph["body"] = fixture.queued.comment()
        fixture.source_after = copy.deepcopy(fixture.source)
        fixture.source_after["labels"] = []
        fixture.events_after_proof = [
            *fixture.events,
            {
                "id": 3,
                "event": "unlabeled",
                "actor": human(),
                "label": {"name": "bot:promote"},
                "created_at": NOW,
            },
        ]
        fixture.failures[fixture.comment_write] = ["after lost response"]
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
            )
        self.assertTrue(fixture.receipt_read)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_invalid_post_provenance_requires_final_authority_reconciliation(
        self,
    ) -> None:
        for field, value in (
            ("user", human()),
            ("performed_via_github_app", None),
        ):
            with self.subTest(field=field):
                fixture = QueueReceiptFixture()
                fixture.comment_response = {**fixture.direct, field: value}
                fixture.source_after = copy.deepcopy(fixture.source)
                as_object(fixture.source_after["head"])["sha"] = str(OTHER)
                with TemporaryDirectory() as temporary:
                    self.assertEqual(
                        fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
                    )
                self.assertTrue(fixture.receipt_read)
                self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_successful_post_rechecks_authority_after_its_receipt_proof(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.source_after = copy.deepcopy(fixture.source)
        as_object(fixture.source_after["head"])["sha"] = str(OTHER)
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
            )
        self.assertTrue(fixture.receipt_read)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 1)

    def test_conflicting_receipt_after_write_does_not_grant_authority(self) -> None:
        queued = source_queue()
        reader = FakeGitHub()
        other = replace(
            queued, parent=PendingSourceParent(PromotionScope(UV_DEV_REPOSITORY, 11))
        )

        class Writer:
            calls = 0

            def create_queue_comment(self, queued: QueuedPromotion) -> None:
                self.calls += 1
                reader.add_queue(queued, 3001)
                reader.add_queue(other, 3002)
                raise CompletionRequestError(None)

        writer = Writer()
        with (
            patch("uv_automations.workflows.promotion_completion.sleep"),
            self.assertRaisesRegex(ValueError, "Conflicting promotion queue records"),
        ):
            record_queue(reader, writer, queued)
        self.assertEqual(writer.calls, 1)

    def test_existing_receipt_and_stale_source_keep_their_outcomes(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.comments = [copy.deepcopy(fixture.direct)]
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "true"})
            )
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)
        fixture = QueueReceiptFixture()
        fixture.source["state"] = "closed"
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
            )
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)

    def test_existing_receipt_read_retry_rechecks_final_authority(self) -> None:
        fixture = QueueReceiptFixture()
        fixture.comments = [copy.deepcopy(fixture.direct)]
        fixture.failures[fixture.source_read] = ["before HTTP 503"]
        fixture.source_after = copy.deepcopy(fixture.source)
        as_object(fixture.source_after["base"])["sha"] = str(OTHER)
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"recorded": "false"})
            )
        self.assertTrue(fixture.receipt_read)
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)

    def test_proven_existing_receipt_cannot_be_reposted_after_readback_omits_it(
        self,
    ) -> None:
        fixture = QueueReceiptFixture()
        fixture.comments = [copy.deepcopy(fixture.direct)]
        fixture.pages_after_proof = {}
        fixture.authority_failures_after_proof = ["before HTTP 503"]
        fixture.headers[fixture.source_read] = "Retry-After: 20\r\n"
        with TemporaryDirectory() as temporary:
            code, _, output = fixture.invoke(Path(temporary))
        self.assertEqual((code, output), (1, {}))
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)
        self.assertEqual(fixture.wire_calls["POST graphql"], 1)
        self.assertEqual(fixture.wire_calls[fixture.comment_list], 4)
        failure = fixture.wire_trace.index(("failed", fixture.source_read))
        self.assertEqual(fixture.wire_trace[failure - 2], ("api", "POST graphql"))
        self.assertEqual(fixture.wire_trace[failure + 1], ("sleep", 20.0))
        self.assertEqual(fixture.wire_trace[failure + 2][0], "api")

    def test_proven_existing_receipt_stays_read_only_after_repeated_authority_failures(
        self,
    ) -> None:
        fixture = QueueReceiptFixture()
        fixture.comments = [copy.deepcopy(fixture.direct)]
        fixture.authority_failures_after_proof = ["before HTTP 429"] * 3
        fixture.headers[fixture.source_read] = "Retry-After: 20\r\n"
        with TemporaryDirectory() as temporary:
            code, _, output = fixture.invoke(Path(temporary))
        self.assertEqual((code, output), (1, {}))
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)
        self.assertEqual(fixture.wire_calls["POST graphql"], 3)
        failures = [
            index
            for index, event in enumerate(fixture.wire_trace)
            if event == ("failed", fixture.source_read)
        ]
        self.assertEqual(len(failures), 3)
        for index in failures[:-1]:
            self.assertEqual(fixture.wire_trace[index + 1], ("sleep", 20.0))
            self.assertEqual(fixture.wire_trace[index + 2][0], "api")
        self.assertEqual(fixture.wire_trace[-1], ("failed", fixture.source_read))

    def test_proven_existing_receipt_remains_unchanged_when_authority_recovers(
        self,
    ) -> None:
        fixture = QueueReceiptFixture()
        fixture.comments = [copy.deepcopy(fixture.direct)]
        fixture.authority_failures_after_proof = ["before HTTP 503"]
        with fixture.offline():
            self.assertEqual(
                record_queue(
                    PromotionQueueCompletionGitHub(token_variable="GH_READ_TOKEN"),
                    PromotionQueueCompletionGitHub(),
                    fixture.queued,
                ),
                QueueRecordOutcome.UNCHANGED,
            )
        self.assertEqual(fixture.wire_calls[fixture.comment_write], 0)
        self.assertEqual(fixture.wire_calls["POST graphql"], 2)


if __name__ == "__main__":
    unittest.main()
