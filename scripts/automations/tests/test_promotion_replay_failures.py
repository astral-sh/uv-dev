import io
import unittest
from collections.abc import Callable, Sequence
from contextlib import redirect_stderr
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from test_promotion_closed import SOURCE
from test_promotion_queue import CHILD_SCOPE, HUMAN, READY, TIME, comment, pull_request
from test_promotion_wakeup import (
    WakeupFixture,
    offline_process,
    outputs,
    workflow_arguments,
)

from uv_automations import cli
from uv_automations.github_actions import DISPATCH_API_VERSION
from uv_automations.github_promotion import PromotionReadError
from uv_automations.json import as_object
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    PromotionApproval,
    PromotionPullRequest,
    PromotionScope,
    ReadyForReviewEvent,
)
from uv_automations.workflows.promotion_queue import QueuedPromotion

SECOND = PromotionScope(UV_DEV_REPOSITORY, 21)
THIRD = PromotionScope(UV_DEV_REPOSITORY, 22)


class ReplayBatchFixture(WakeupFixture):
    def __init__(self) -> None:
        super().__init__()
        self.queues = {CHILD_SCOPE: self.queued}
        self.unconfirmed_dispatch: PromotionScope | None = None
        self.malformed_dispatch: PromotionScope | None = None
        for scope, digit in ((SECOND, "1"), (THIRD, "2")):
            head = CommitSha(digit * 40)
            parent = self.reader.pull_requests[SOURCE]
            source = pull_request(
                scope,
                base_ref=parent.details.head.ref,
                base_sha=parent.details.head.sha,
                head_ref=f"fixture/child-{scope.number}",
                head_sha=head,
            )
            event = replace(READY, identifier=1000 + scope.number)
            approval = PromotionApproval(scope, head, event, event.identifier)
            queued = QueuedPromotion.waiting_for_parent(source, approval, parent)
            self.queues[scope] = queued
            self.reader.pull_requests[scope] = source
            self.reader.events[scope] = (event,)
            self.reader.comments[scope] = (comment(scope, queued.comment()),)

    @override
    def command(
        self, arguments: Sequence[str], *, payload: object | None = None
    ) -> object:
        if list(arguments) == [
            "api",
            "--method",
            "GET",
            "repos/astral-sh/uv-dev",
        ]:
            return super().command(arguments, payload=payload)
        dispatch_arguments = [
            "api",
            "--method",
            "POST",
            "--header",
            f"X-GitHub-Api-Version: {DISPATCH_API_VERSION}",
            "repos/astral-sh/uv-dev/actions/workflows/promote-pull-request.yml/dispatches",
            "--input",
            "-",
        ]
        if self.dispatch_allowed and list(arguments) == dispatch_arguments:
            for scope, queued in self.queues.items():
                expected: dict[str, object] = {
                    "ref": "main",
                    "inputs": {
                        "pull_request": str(scope.number),
                        "head_sha": str(queued.approval.head),
                        "approval_id": str(queued.approval.event_id),
                    },
                }
                if payload != expected:
                    continue
                self.dispatches.append(expected)
                if scope == self.unconfirmed_dispatch:
                    raise PromotionReadError("injected private response")
                run = 4000 + len(self.dispatches)
                return {
                    "workflow_run_id": run,
                    "html_url": (
                        "https://github.com/astral-sh/uv-dev/actions/runs/1"
                        if scope == self.malformed_dispatch
                        else f"https://github.com/astral-sh/uv-dev/actions/runs/{run}"
                    ),
                    "run_url": f"https://api.github.com/repos/astral-sh/uv-dev/actions/runs/{run}",
                }
        raise AssertionError(f"Unexpected GitHub command: {arguments}")

    def arguments(self, root: Path) -> list[str]:
        return workflow_arguments(
            "replay-promoted-children",
            "recover",
            {
                "GITHUB_REPOSITORY": str(SOURCE.repository.name),
                "GITHUB_REPOSITORY_ID": str(SOURCE.repository.database_id),
                "PARENT_PULL_REQUEST": str(SOURCE.number),
                "GITHUB_STEP_SUMMARY": str(root / "replay-summary"),
            },
        )

    def dispatched_sources(self) -> list[str]:
        return [
            str(as_object(payload["inputs"])["pull_request"])
            for payload in self.dispatches
        ]

    def worker_for(self, scope: PromotionScope, root: Path) -> dict[str, str]:
        queued = self.queues[scope]
        output = root / f"worker-{scope.number}-output"
        arguments = workflow_arguments(
            "prepare",
            "queue",
            {
                "GITHUB_REPOSITORY": str(scope.repository.name),
                "GITHUB_REPOSITORY_ID": str(scope.repository.database_id),
                "PULL_REQUEST_NUMBER": str(scope.number),
                "EXPECTED_HEAD_SHA": str(queued.approval.head),
                "EXPECTED_APPROVAL_ID": str(queued.approval.event_id),
                "GITHUB_OUTPUT": str(output),
                "GITHUB_STEP_SUMMARY": str(root / f"worker-{scope.number}-summary"),
            },
        )
        self.dispatch_allowed = False
        with self.offline_replay():
            cli.run(cli.parse_command(cli.create_parser(), arguments))
        return outputs(output)


class PromotionReplayFailureTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        self.enterContext(offline_process())

    def test_child_read_failure_does_not_strand_an_authorized_sibling(self) -> None:
        fixture = ReplayBatchFixture()
        fixture.dispatch_allowed = True
        read = fixture.reader.get_promotion_pull_request

        def read_source(scope: PromotionScope) -> PromotionPullRequest:
            if scope == SECOND:
                raise PromotionReadError("injected private response")
            return read(scope)

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            preparation = fixture.prepare(root)
            self.assertEqual(preparation, {"action": "observed-closed"})
            stderr = io.StringIO()
            with (
                fixture.offline_replay(),
                patch.object(
                    fixture.reader,
                    "get_promotion_pull_request",
                    side_effect=read_source,
                ),
                redirect_stderr(stderr),
                self.assertRaises(SystemExit) as raised,
            ):
                cli.main(fixture.arguments(root))
            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(fixture.dispatched_sources(), ["20", "22"])
            self.assertEqual(
                (root / "replay-summary").read_text(),
                "Replayed 2 queued promotion(s).\n\n"
                "- [astral-sh/uv-dev#20](https://github.com/astral-sh/uv-dev/actions/runs/4001)\n"
                "- [astral-sh/uv-dev#22](https://github.com/astral-sh/uv-dev/actions/runs/4002)\n\n"
                "Could not confirm 1 queued promotion check(s): astral-sh/uv-dev#21.\n",
            )
            self.assertNotIn("injected private response", stderr.getvalue())
            self.assertEqual(fixture.worker_for(THIRD, root)["action"], "update-parent")
            fixture.reader.events[THIRD] = (
                *fixture.reader.events[THIRD],
                ReadyForReviewEvent(2000, HUMAN, TIME),
            )
            self.assertEqual(fixture.worker_for(THIRD, root)["action"], "stale")
            self.assertEqual(fixture.dispatched_sources(), ["20", "22"])
        self.assertFalse(fixture.reader.created)
        self.assertFalse(fixture.reader.dispatched)

    def test_ambiguous_dispatch_is_not_retried_within_the_batch(self) -> None:
        fixture = ReplayBatchFixture()
        fixture.dispatch_allowed = True
        fixture.unconfirmed_dispatch = SECOND
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            stderr = io.StringIO()
            with (
                fixture.offline_replay(),
                redirect_stderr(stderr),
                self.assertRaises(SystemExit) as raised,
            ):
                cli.main(fixture.arguments(root))
            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(fixture.dispatched_sources(), ["20", "21", "22"])
            summary = (root / "replay-summary").read_text()
            self.assertIn("Replayed 2 queued promotion(s).", summary)
            self.assertIn("actions/runs/4003)", summary)
            self.assertNotIn("actions/runs/4002)", summary)
            self.assertIn("Could not confirm 1 queued promotion check(s)", summary)
            self.assertNotIn("injected private response", stderr.getvalue())

    def test_failure_does_not_upgrade_another_childs_approval(self) -> None:
        fixture = ReplayBatchFixture()
        fixture.dispatch_allowed = True
        fixture.unconfirmed_dispatch = SECOND
        fixture.reader.events[THIRD] = (
            *fixture.reader.events[THIRD],
            ReadyForReviewEvent(2000, HUMAN, TIME),
        )
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                fixture.offline_replay(),
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit) as raised,
            ):
                cli.main(fixture.arguments(root))
            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(fixture.dispatched_sources(), ["20", "21"])
            self.assertIn(
                "Pending or skipped: stale-approval=1.",
                (root / "replay-summary").read_text(),
            )

    def test_parent_and_discovery_failures_stop_before_dispatch(self) -> None:
        for failure in ("parent", "discovery"):
            with self.subTest(failure=failure), TemporaryDirectory() as temporary:
                fixture = ReplayBatchFixture()
                fixture.dispatch_allowed = True
                root = Path(temporary)
                read = fixture.reader.get_promotion_pull_request

                def read_source(
                    scope: PromotionScope,
                    read_pull_request: Callable[
                        [PromotionScope], PromotionPullRequest
                    ] = read,
                ) -> PromotionPullRequest:
                    if scope == SOURCE:
                        raise PromotionReadError("injected private response")
                    return read_pull_request(scope)

                failure_patch = (
                    patch.object(
                        fixture.reader,
                        "get_promotion_pull_request",
                        side_effect=read_source,
                    )
                    if failure == "parent"
                    else patch.object(
                        fixture.reader,
                        "list_pull_requests",
                        side_effect=PromotionReadError("injected private response"),
                    )
                )
                with (
                    fixture.offline_replay(),
                    failure_patch,
                    redirect_stderr(io.StringIO()),
                    self.assertRaises(SystemExit) as raised,
                ):
                    cli.main(fixture.arguments(root))
                self.assertEqual(raised.exception.code, 1)
                self.assertFalse(fixture.dispatches)
                self.assertFalse((root / "replay-summary").exists())

    def test_protocol_violation_is_not_a_recoverable_child_failure(self) -> None:
        fixture = ReplayBatchFixture()
        fixture.dispatch_allowed = True
        fixture.malformed_dispatch = SECOND

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                fixture.offline_replay(),
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit) as raised,
            ):
                cli.main(fixture.arguments(root))
            self.assertEqual(raised.exception.code, 2)
            self.assertEqual(fixture.dispatched_sources(), ["20", "21"])
            self.assertFalse((root / "replay-summary").exists())


if __name__ == "__main__":
    unittest.main()
