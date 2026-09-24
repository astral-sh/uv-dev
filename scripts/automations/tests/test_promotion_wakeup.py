import os
import shlex
import socket
import subprocess
import unittest
from collections.abc import Callable, Iterator, Sequence
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from test_promotion_closed import (
    HEAD,
    NOW,
    OTHER,
    SOURCE,
    UPSTREAM,
    ObservationFixture,
)
from test_promotion_queue import (
    BOT,
    CHILD_SCOPE,
    HUMAN,
    MAIN,
    READY,
    TIME,
    FakeGitHub,
    comment,
    pull_request,
)
from test_promotion_workflow_boundaries import job, wakeup_is_enabled

from uv_automations import cli, promotions_cli
from uv_automations.github_actions import DISPATCH_API_VERSION
from uv_automations.github_promotion import (
    PromotionGitHub,
    decode_promotion_comment,
    decode_promotion_pull_request,
    verified_promoted_parent,
)
from uv_automations.json import as_object
from uv_automations.models import CommitSha, PullRequestState
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    ConvertedToDraftEvent,
    LabelAddedEvent,
    LabelRemovedEvent,
    MergedPromotedParent,
    PromotionApproval,
    PromotionScope,
    PullRequestMerge,
    ReadyForReviewEvent,
)
from uv_automations.workflows.promotion_queue import (
    PendingSourceParent,
    QueuedPromotion,
)

CHILD_HEAD = CommitSha("f" * 40)
RUN = subprocess.run
RUN_ID = 4000
TOKEN_VARIABLES = {
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_READ_TOKEN",
    "GH_SOURCE_TOKEN",
    "GH_UPSTREAM_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
}


def workflow_arguments(
    name: str, next_name: str, environment: dict[str, str]
) -> list[str]:
    """Read the actual trusted CLI invocation without executing its shell."""
    section = job("promote-pull-request.yml", name, next_name)
    script = section.rsplit("        run: |\n", 1)[1]
    arguments = shlex.split(script.replace("\\\n", ""))
    if arguments[:4] != ["$AUTOMATIONS_PYTHON", "-I", "-m", "uv_automations"]:
        raise ValueError("Unexpected automation entry point")
    result: list[str] = []
    for argument in arguments[4:]:
        if argument.startswith("$"):
            result.append(environment[argument.removeprefix("$")])
        elif "$" in argument or argument in {";", "|", "&&", "||", "<<<"}:
            raise ValueError("Unexpected automation shell syntax")
        else:
            result.append(argument)
    return result


def outputs(path: Path) -> dict[str, str]:
    return dict(line.split("=", 1) for line in path.read_text().splitlines())


@contextmanager
def offline_process() -> Iterator[None]:
    def run(arguments: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        if (
            len(arguments) == 3
            and arguments[:2] == ["git", "check-ref-format"]
            and arguments[2].startswith("refs/heads/")
        ):
            return RUN(
                arguments, check=False, capture_output=True, text=True, timeout=30
            )
        raise AssertionError(f"Unexpected subprocess: {arguments}")

    environment = {
        key: value for key, value in os.environ.items() if key not in TOKEN_VARIABLES
    }
    blocked = AssertionError("Unexpected network access")
    with (
        patch.dict(os.environ, environment, clear=True),
        patch("subprocess.run", side_effect=run),
        patch.object(socket, "create_connection", side_effect=blocked),
        patch.object(socket.socket, "connect", side_effect=blocked),
        patch.object(socket.socket, "connect_ex", side_effect=blocked),
    ):
        yield


class WakeupFixture:
    """Carry the real closed observation into the existing queue and worker."""

    def __init__(self, *, merged: bool = True) -> None:
        self.observation = ObservationFixture()
        if merged:
            self.observation.upstream.update(
                state="closed", merged_at=NOW, merge_commit_sha=str(OTHER)
            )
        parent = decode_promotion_pull_request(self.observation.source, SOURCE)
        upstream = decode_promotion_pull_request(self.observation.upstream, UPSTREAM)
        child = pull_request(
            CHILD_SCOPE,
            base_ref=parent.details.head.ref,
            base_sha=parent.details.head.sha,
            head_ref="fixture/child",
            head_sha=CHILD_HEAD,
        )
        self.approval = PromotionApproval(
            CHILD_SCOPE, CHILD_HEAD, READY, READY.identifier
        )
        self.queued = QueuedPromotion.waiting_for_parent(child, self.approval, parent)
        self.reader = FakeGitHub(
            pull_requests={SOURCE: parent, UPSTREAM: upstream, CHILD_SCOPE: child},
            events={CHILD_SCOPE: (READY,)},
            comments={
                SOURCE: (decode_promotion_comment(self.observation.direct, SOURCE),),
                CHILD_SCOPE: (comment(CHILD_SCOPE, self.queued.comment()),),
            },
            ancestors={(OTHER, MAIN)},
        )
        self.dispatches: list[dict[str, object]] = []
        self.dispatch_allowed = False

    def replace_queue(self, queued: QueuedPromotion) -> None:
        self.queued = queued
        self.reader.comments[CHILD_SCOPE] = (comment(CHILD_SCOPE, queued.comment()),)

    def command(
        self, arguments: Sequence[str], *, payload: object | None = None
    ) -> object:
        if (
            list(arguments)
            == [
                "api",
                "--method",
                "GET",
                "repos/astral-sh/uv-dev",
            ]
            and payload is None
        ):
            return {
                "full_name": str(UV_DEV_REPOSITORY.name),
                "id": UV_DEV_REPOSITORY.database_id,
            }
        expected: dict[str, object] = {
            "ref": "main",
            "inputs": {
                "pull_request": str(CHILD_SCOPE.number),
                "head_sha": str(self.queued.approval.head),
                "approval_id": str(self.queued.approval.event_id),
            },
        }
        if (
            self.dispatch_allowed
            and list(arguments)
            == [
                "api",
                "--method",
                "POST",
                "--header",
                f"X-GitHub-Api-Version: {DISPATCH_API_VERSION}",
                (
                    "repos/astral-sh/uv-dev/actions/workflows/"
                    "promote-pull-request.yml/dispatches"
                ),
                "--input",
                "-",
            ]
            and payload == expected
        ):
            self.dispatches.append(expected)
            return {
                "workflow_run_id": RUN_ID,
                "html_url": f"https://github.com/astral-sh/uv-dev/actions/runs/{RUN_ID}",
                "run_url": f"https://api.github.com/repos/astral-sh/uv-dev/actions/runs/{RUN_ID}",
            }
        raise AssertionError(f"Unexpected GitHub command: {arguments}")

    @contextmanager
    def offline_replay(self) -> Iterator[None]:
        def reader(*, token_variable: str | None = None) -> FakeGitHub:
            if token_variable not in {None, "GH_SOURCE_TOKEN", "GH_UPSTREAM_TOKEN"}:
                raise AssertionError("Unexpected reader credential")
            return self.reader

        with (
            offline_process(),
            patch.object(promotions_cli, "PromotionGitHub", side_effect=reader),
            patch.object(PromotionGitHub, "_command", side_effect=self.command),
            patch.object(
                promotions_cli,
                "PromotionCompletionGitHub",
                side_effect=AssertionError("Unexpected completion writer"),
            ),
        ):
            yield

    def prepare(
        self,
        root: Path,
        *,
        source: PromotionScope = SOURCE,
        approval_id: str = "",
    ) -> dict[str, str]:
        output, summary = root / "prepare-output", root / "prepare-summary"
        arguments = workflow_arguments(
            "prepare",
            "queue",
            {
                "GITHUB_REPOSITORY": str(source.repository.name),
                "GITHUB_REPOSITORY_ID": str(source.repository.database_id),
                "PULL_REQUEST_NUMBER": str(source.number),
                "EXPECTED_HEAD_SHA": str(HEAD),
                "EXPECTED_APPROVAL_ID": approval_id,
                "GITHUB_OUTPUT": str(output),
                "GITHUB_STEP_SUMMARY": str(summary),
            },
        )
        reader = (
            self.observation.offline()
            if source == SOURCE and not approval_id
            else self.offline_replay()
        )
        with offline_process(), reader:
            cli.run(cli.parse_command(cli.create_parser(), arguments))
        return outputs(output)

    def wake(self, root: Path, preparation: dict[str, str]) -> str | None:
        if not wakeup_is_enabled(action=preparation.get("action", "")):
            return None
        summary = root / "replay-summary"
        arguments = workflow_arguments(
            "replay-promoted-children",
            "recover",
            {
                "GITHUB_REPOSITORY": str(SOURCE.repository.name),
                "GITHUB_REPOSITORY_ID": str(SOURCE.repository.database_id),
                "PARENT_PULL_REQUEST": str(SOURCE.number),
                "GITHUB_STEP_SUMMARY": str(summary),
            },
        )
        with self.offline_replay():
            cli.run(cli.parse_command(cli.create_parser(), arguments))
        return summary.read_text()

    def worker(self, root: Path) -> dict[str, str]:
        output, summary = root / "worker-output", root / "worker-summary"
        inputs = as_object(self.dispatches[0]["inputs"])
        arguments = workflow_arguments(
            "prepare",
            "queue",
            {
                "GITHUB_REPOSITORY": str(UV_DEV_REPOSITORY.name),
                "GITHUB_REPOSITORY_ID": str(UV_DEV_REPOSITORY.database_id),
                "PULL_REQUEST_NUMBER": str(inputs["pull_request"]),
                "EXPECTED_HEAD_SHA": str(inputs["head_sha"]),
                "EXPECTED_APPROVAL_ID": str(inputs["approval_id"]),
                "GITHUB_OUTPUT": str(output),
                "GITHUB_STEP_SUMMARY": str(summary),
            },
        )
        self.dispatch_allowed = False
        with self.offline_replay():
            cli.run(cli.parse_command(cli.create_parser(), arguments))
        return outputs(output)


class PromotionWakeupTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        self.enterContext(offline_process())

    def test_observed_close_runs_the_real_parent_scoped_cli_and_worker(self) -> None:
        fixture = WakeupFixture()
        fixture.dispatch_allowed = True
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            preparation = fixture.prepare(root)
            self.assertEqual(preparation, {"action": "observed-closed"})
            self.assertIn(str(HEAD), (root / "prepare-summary").read_text())
            summary = fixture.wake(root, preparation)
            self.assertEqual(
                summary,
                "Replayed 1 queued promotion(s).\n\n"
                f"- [astral-sh/uv-dev#{CHILD_SCOPE.number}]"
                f"(https://github.com/astral-sh/uv-dev/actions/runs/{RUN_ID})\n",
            )
            self.assertEqual(len(fixture.dispatches), 1)
            self.assertEqual(fixture.worker(root)["action"], "update-parent")
            self.assertEqual(len(fixture.dispatches), 1)
        self.assertFalse(fixture.reader.created)
        self.assertFalse(fixture.reader.dispatched)

    def test_observed_open_upstream_cannot_dispatch_children(self) -> None:
        fixture = WakeupFixture(merged=False)
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            preparation = fixture.prepare(root)
            self.assertEqual(preparation, {"action": "observed-closed"})
            self.assertEqual(
                fixture.wake(root, preparation),
                "Replayed 0 queued promotion(s).\n\n"
                "Pending or skipped: parent-not-merged=1.\n",
            )
        self.assertFalse(fixture.dispatches)

    def test_wakeup_retry_does_not_upgrade_a_child_approval(self) -> None:
        fixture = WakeupFixture()
        fixture.dispatch_allowed = True
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            preparation = fixture.prepare(root)
            fixture.wake(root, preparation)
            fixture.wake(root, preparation)
            self.assertEqual(len(fixture.dispatches), 2)
            self.assertEqual(fixture.dispatches[0], fixture.dispatches[1])
            fixture.dispatch_allowed = False
            fixture.reader.events[CHILD_SCOPE] = (
                READY,
                ReadyForReviewEvent(1002, HUMAN, TIME),
            )
            fixture.wake(root, preparation)
            self.assertEqual(len(fixture.dispatches), 2)

    def test_explicit_replay_and_private_source_do_not_gain_wakeup_authority(
        self,
    ) -> None:
        fixture = WakeupFixture()
        private = PromotionScope(UV_SECURITY_REPOSITORY, SOURCE.number)
        parent = fixture.reader.pull_requests[SOURCE]
        fixture.reader.pull_requests[private] = replace(
            parent,
            scope=private,
            details=replace(
                parent.details,
                reference=private.reference,
                url=f"https://github.com/{private.repository.name}/pull/{private.number}",
                base=replace(parent.details.base, repository=private.repository),
                head=replace(parent.details.head, repository=private.repository),
            ),
        )
        for source, approval_id in ((SOURCE, "1000"), (private, "")):
            with (
                self.subTest(source=source, approval_id=approval_id),
                TemporaryDirectory() as temporary,
            ):
                root = Path(temporary)
                preparation = fixture.prepare(
                    root, source=source, approval_id=approval_id
                )
                self.assertEqual(preparation["action"], "stale")
                self.assertFalse(
                    wakeup_is_enabled(
                        repository=str(source.repository.name),
                        action=preparation["action"],
                    )
                )
                self.assertIsNone(fixture.wake(root, preparation))
        self.assertFalse(fixture.dispatches)

    def test_unrecognized_parent_never_reaches_the_dispatch_stage(self) -> None:
        cases: tuple[Callable[[ObservationFixture], None], ...] = (
            lambda observation: observation.comments.clear(),
            lambda observation: observation.graph.update(lastEditedAt=NOW),
            lambda observation: as_object(observation.upstream["head"]).update(
                sha=str(CHILD_HEAD)
            ),
            lambda observation: observation.upstream.update(
                state="closed", merged_at=None, merge_commit_sha=None
            ),
        )
        for change in cases:
            with self.subTest(change=change):
                fixture = WakeupFixture()
                change(fixture.observation)
                with TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    preparation = fixture.prepare(root)
                    self.assertEqual(preparation["action"], "stale")
                    self.assertIsNone(fixture.wake(root, preparation))
                self.assertFalse(fixture.dispatches)

    def test_child_authority_is_not_inherited_from_the_parent_observation(self) -> None:
        def wrong_parent(fixture: WakeupFixture) -> None:
            other = PromotionScope(UV_DEV_REPOSITORY, SOURCE.number + 1)
            fixture.replace_queue(
                replace(fixture.queued, parent=PendingSourceParent(other))
            )

        def ambiguous_queue(fixture: WakeupFixture) -> None:
            conflicting = replace(
                fixture.queued, head=replace(fixture.queued.head, ref="other")
            )
            fixture.reader.add_queue(conflicting, 2001)

        def changed_child(fixture: WakeupFixture, field: str) -> None:
            child = fixture.reader.pull_requests[CHILD_SCOPE]
            details = child.details
            if field == "base":
                details = replace(details, base=replace(details.base, sha=OTHER))
            elif field == "head":
                details = replace(details, head=replace(details.head, sha=OTHER))
            elif field == "state":
                details = replace(details, state=PullRequestState.CLOSED)
            else:
                raise AssertionError("Unexpected child change")
            fixture.reader.pull_requests[CHILD_SCOPE] = replace(child, details=details)

        def changed_parent(fixture: WakeupFixture) -> None:
            parent = fixture.reader.pull_requests[SOURCE]
            fixture.reader.pull_requests[SOURCE] = replace(
                parent,
                details=replace(
                    parent.details, head=replace(parent.details.head, sha=OTHER)
                ),
            )

        def changed_merge(fixture: WakeupFixture) -> None:
            parent = verified_promoted_parent(
                fixture.reader, fixture.reader.pull_requests[SOURCE]
            )
            if not isinstance(parent, MergedPromotedParent):
                raise TypeError("Expected a merged parent fixture")
            fixture.replace_queue(
                QueuedPromotion.waiting_for_sync(
                    fixture.reader.pull_requests[CHILD_SCOPE], fixture.approval, parent
                )
            )
            fixture.reader.pull_requests[UPSTREAM] = replace(
                parent.upstream, merge=PullRequestMerge(CHILD_HEAD, TIME)
            )
            fixture.reader.ancestors.add((CHILD_HEAD, MAIN))

        def non_public_main(fixture: WakeupFixture) -> None:
            fixture.reader.main = CHILD_HEAD
            fixture.reader.ancestors.add((MAIN, CHILD_HEAD))

        cases: tuple[tuple[str, Callable[[WakeupFixture], None]], ...] = (
            ("wrong parent", wrong_parent),
            (
                "no queue",
                lambda fixture: fixture.reader.comments.update({CHILD_SCOPE: ()}),
            ),
            ("edited queue", lambda fixture: fixture.reader.edited.add(2000)),
            ("ambiguous queue", ambiguous_queue),
            (
                "withdrawn approval",
                lambda fixture: fixture.reader.events.update(
                    {CHILD_SCOPE: (READY, ConvertedToDraftEvent(1001, HUMAN, TIME))}
                ),
            ),
            (
                "new approval",
                lambda fixture: fixture.reader.events.update(
                    {CHILD_SCOPE: (READY, ReadyForReviewEvent(1002, HUMAN, TIME))}
                ),
            ),
            (
                "bot approval",
                lambda fixture: fixture.reader.events.update(
                    {CHILD_SCOPE: (replace(READY, actor=BOT),)}
                ),
            ),
            ("changed base", lambda fixture: changed_child(fixture, "base")),
            ("changed head", lambda fixture: changed_child(fixture, "head")),
            ("closed child", lambda fixture: changed_child(fixture, "state")),
            ("changed parent", changed_parent),
            (
                "no promotion record",
                lambda fixture: fixture.reader.comments.update({SOURCE: ()}),
            ),
            (
                "non-bot promotion record",
                lambda fixture: fixture.reader.comments.update(
                    {
                        SOURCE: (
                            replace(fixture.reader.comments[SOURCE][0], author=HUMAN),
                        )
                    }
                ),
            ),
            ("changed merge", changed_merge),
            ("not synchronized", lambda fixture: fixture.reader.ancestors.clear()),
            ("non-public main", non_public_main),
        )
        for name, change in cases:
            with self.subTest(name=name):
                fixture = WakeupFixture()
                with TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    preparation = fixture.prepare(root)
                    self.assertEqual(preparation, {"action": "observed-closed"})
                    change(fixture)
                    summary = fixture.wake(root, preparation)
                    if summary is None:
                        self.fail("Expected a parent-scoped replay summary")
                    self.assertTrue(summary.startswith("Replayed 0"))
                self.assertFalse(fixture.dispatches)
                self.assertFalse(fixture.reader.created)
                self.assertFalse(fixture.reader.dispatched)

    def test_worker_revalidates_after_a_successful_dispatch(self) -> None:
        changes: tuple[tuple[str, Callable[[WakeupFixture], None]], ...] = (
            (
                "withdrawn approval",
                lambda fixture: fixture.reader.events.update(
                    {CHILD_SCOPE: (READY, ConvertedToDraftEvent(1001, HUMAN, TIME))}
                ),
            ),
            (
                "replaced approval",
                lambda fixture: fixture.reader.events.update(
                    {CHILD_SCOPE: (READY, ReadyForReviewEvent(1002, HUMAN, TIME))}
                ),
            ),
            ("edited queue", lambda fixture: fixture.reader.edited.add(2000)),
            ("lost merge", lambda fixture: fixture.reader.ancestors.clear()),
        )
        for name, change in changes:
            with self.subTest(name=name):
                fixture = WakeupFixture()
                fixture.dispatch_allowed = True
                with TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    preparation = fixture.prepare(root)
                    fixture.wake(root, preparation)
                    self.assertEqual(len(fixture.dispatches), 1)
                    change(fixture)
                    worker = fixture.worker(root)
                    self.assertEqual(worker["action"], "stale")
                    self.assertNotIn("approval_id", worker)
                    self.assertNotIn("queue", worker)
                    self.assertEqual(len(fixture.dispatches), 1)

    def test_labeled_draft_keeps_its_exact_child_approval(self) -> None:
        fixture = WakeupFixture()
        child = fixture.reader.pull_requests[CHILD_SCOPE]
        child = replace(
            child, draft=True, details=replace(child.details, labels=("bot:promote",))
        )
        label = LabelAddedEvent(2001, HUMAN, TIME, "bot:promote")
        fixture.approval = PromotionApproval(CHILD_SCOPE, CHILD_HEAD, label, None)
        fixture.reader.pull_requests[CHILD_SCOPE] = child
        fixture.reader.events[CHILD_SCOPE] = (label,)
        fixture.replace_queue(
            QueuedPromotion.waiting_for_parent(
                child, fixture.approval, fixture.reader.pull_requests[SOURCE]
            )
        )
        fixture.dispatch_allowed = True
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture.wake(root, fixture.prepare(root))
            self.assertEqual(len(fixture.dispatches), 1)
            fixture.reader.events[CHILD_SCOPE] = (
                label,
                LabelRemovedEvent(2002, HUMAN, TIME, "bot:promote"),
            )
            self.assertEqual(fixture.worker(root)["action"], "stale")


if __name__ == "__main__":
    unittest.main()
