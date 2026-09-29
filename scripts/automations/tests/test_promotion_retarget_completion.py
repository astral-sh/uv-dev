import io
import json
import os
import subprocess
import unittest
from collections.abc import Callable
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from dataclasses import replace
from unittest.mock import patch

from test_promotion_retarget import (
    MAIN,
    OTHER,
    FakeWriter,
    RetargetFixture,
    pull_request,
)
from test_promotion_retarget_cli import updated_payload

from uv_automations import cli
from uv_automations.github_promotion import PromotionReadError
from uv_automations.github_promotion_retarget import PromotionRetargetGitHub
from uv_automations.models import PullRequestState, RepositoryIdentity
from uv_automations.promotion_models import (
    PROMOTION_LABEL,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    PromotionScope,
)
from uv_automations.workflows.promotion_retarget import Retargeted, retarget

RUN = subprocess.run


class PatchFixture:
    """Exercise the actual retarget writer through an offline CLI boundary."""

    def __init__(self, repository: RepositoryIdentity = UV_DEV_REPOSITORY) -> None:
        self.fixture = RetargetFixture(repository)
        self.failure = "exit"
        self.commit = True
        self.after_patch: Callable[[], None] = lambda: None
        self.writes: list[PromotionScope] = []

    def command(
        self, arguments: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        if (
            len(arguments) == 3
            and arguments[:2] == ["git", "check-ref-format"]
            and arguments[2].startswith("refs/heads/")
        ):
            return RUN(
                arguments, check=False, text=True, capture_output=True, timeout=30
            )
        repository = self.fixture.repository
        number = int(arguments[4].rsplit("/", 1)[1])
        source = PromotionScope(repository, number)
        if arguments != [
            "gh",
            "api",
            "--method",
            "PATCH",
            f"repos/{repository.name}/pulls/{number}",
            "--input",
            "-",
        ] or json.loads(str(kwargs["input"])) != {"base": "main"}:
            raise AssertionError("Unexpected retarget request")
        environment = kwargs["env"]
        if (
            not isinstance(environment, dict)
            or environment.get("GH_TOKEN") != "local-write"
        ):
            raise AssertionError("Unexpected retarget credential")
        self.writes.append(source)
        current = self.fixture.source.pull_requests[source]
        if self.commit:
            current = replace(
                current,
                details=replace(
                    current.details,
                    base=replace(current.details.base, ref="main", sha=MAIN),
                ),
            )
            self.fixture.source.pull_requests[source] = current
        self.after_patch()
        if len(self.writes) == 1:
            match self.failure:
                case "exit":
                    return subprocess.CompletedProcess(
                        arguments, 1, "", "private HTTP 503 response"
                    )
                case "timeout":
                    raise subprocess.TimeoutExpired(
                        arguments, 60, stderr="private response"
                    )
                case "invalid-json":
                    return subprocess.CompletedProcess(arguments, 0, "not-json", "")
                case "invalid-object":
                    return subprocess.CompletedProcess(arguments, 0, "{}", "")
                case "programming":
                    raise ValueError("programming error")
                case "none":
                    pass
                case _:
                    raise AssertionError("Unexpected failure fixture")
        payload = updated_payload(repository)
        payload.update(number=number, html_url=str(current.details.url))
        payload["head"] = {
            "repo": {"full_name": str(repository.name), "id": repository.database_id},
            "ref": current.details.head.ref,
            "sha": str(current.details.head.sha),
        }
        return subprocess.CompletedProcess(arguments, 0, json.dumps(payload), "")

    def run(self) -> tuple[str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with (
            patch.dict(os.environ, {"GH_TOKEN": "local-write"}),
            patch(
                "uv_automations.promotion_retarget_cli.PromotionGitHub",
                side_effect=[self.fixture.source, self.fixture.upstream],
            ),
            patch(
                "uv_automations.github_promotion.subprocess.run",
                side_effect=self.command,
            ),
            redirect_stdout(stdout),
            redirect_stderr(stderr),
        ):
            cli.main(
                [
                    "promotions",
                    "retarget",
                    "apply",
                    "--repo",
                    str(self.fixture.repository.name),
                    "--repository-id",
                    str(self.fixture.repository.database_id),
                    "--main-sha",
                    str(MAIN),
                ]
            )
        return stdout.getvalue(), stderr.getvalue()


class PromotionRetargetCompletionTests(unittest.TestCase):
    def test_cli_recovers_a_committed_patch(self) -> None:
        for repository in (UV_DEV_REPOSITORY, UV_SECURITY_REPOSITORY):
            for failure in ("exit", "timeout", "invalid-json", "invalid-object"):
                with self.subTest(repository=repository, failure=failure):
                    fixture = PatchFixture(repository)
                    fixture.failure = failure
                    stdout, stderr = fixture.run()
                    self.assertEqual(stderr, "")
                    self.assertEqual(
                        stdout,
                        f"Retargeted 1 draft pull requests in `{repository.name}`. 0 were already on main. Left 0 ready children with promotion and skipped 0 unverifiable or changed children.\n",
                    )
                    self.assertEqual(fixture.writes, [fixture.fixture.child.scope])

    def test_cli_continues_to_the_next_child_after_reconciliation(self) -> None:
        fixture = PatchFixture()
        second = pull_request(UV_DEV_REPOSITORY, 125, head_ref="second-child")
        fixture.fixture.source.pull_requests[second.scope] = second
        stdout, stderr = fixture.run()
        self.assertEqual(stderr, "")
        self.assertTrue(stdout.startswith("Retargeted 2 draft pull requests"))
        self.assertEqual(fixture.writes, [fixture.fixture.child.scope, second.scope])

    def test_an_uncommitted_patch_is_not_repeated(self) -> None:
        fixture = PatchFixture()
        fixture.commit = False
        with self.assertRaises(SystemExit) as raised:
            fixture.run()
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(fixture.writes, [fixture.fixture.child.scope])

    def test_reconciliation_requires_the_exact_child_state(self) -> None:
        for change in (
            "head",
            "head-ref",
            "foreign",
            "base",
            "main-sha",
            "ready",
            "label",
            "closed",
        ):
            with self.subTest(change=change):
                fixture = PatchFixture()
                plan = fixture.fixture.plan().plans[0]

                def change_child(
                    fixture: PatchFixture = fixture, change: str = change
                ) -> None:
                    child = fixture.fixture.child
                    match change:
                        case "head":
                            child = replace(
                                child,
                                details=replace(
                                    child.details,
                                    head=replace(child.details.head, sha=OTHER),
                                ),
                            )
                        case "head-ref":
                            child = replace(
                                child,
                                details=replace(
                                    child.details,
                                    head=replace(child.details.head, ref="other"),
                                ),
                            )
                        case "foreign":
                            child = replace(
                                child,
                                details=replace(
                                    child.details,
                                    head=replace(
                                        child.details.head, repository=UV_REPOSITORY
                                    ),
                                ),
                            )
                        case "base":
                            child = replace(
                                child,
                                details=replace(
                                    child.details,
                                    base=replace(child.details.base, ref="other"),
                                ),
                            )
                        case "main-sha":
                            child = replace(
                                child,
                                details=replace(
                                    child.details,
                                    base=replace(child.details.base, sha=OTHER),
                                ),
                            )
                        case "ready":
                            child = replace(child, draft=False)
                        case "label":
                            child = replace(
                                child,
                                details=replace(
                                    child.details, labels=(PROMOTION_LABEL,)
                                ),
                            )
                        case "closed":
                            child = replace(
                                child,
                                details=replace(
                                    child.details, state=PullRequestState.CLOSED
                                ),
                            )
                        case _:
                            raise AssertionError("Unexpected child fixture")
                    fixture.fixture.source.pull_requests[child.scope] = child

                fixture.after_patch = change_child
                with (
                    patch.dict(os.environ, {"GH_TOKEN": "local-write"}),
                    patch(
                        "uv_automations.github_promotion.subprocess.run",
                        side_effect=fixture.command,
                    ),
                    self.assertRaises(PromotionReadError),
                ):
                    retarget(
                        fixture.fixture.source,
                        fixture.fixture.upstream,
                        PromotionRetargetGitHub(token_variable="GH_TOKEN"),
                        plan,
                    )
                self.assertEqual(fixture.writes, [plan.source])

    def test_reconciliation_rechecks_main_and_parent_evidence(self) -> None:
        for change in ("source-main", "public-main", "parent-record", "parent-head"):
            with self.subTest(change=change):
                fixture = PatchFixture()
                plan = fixture.fixture.plan().plans[0]

                def change_evidence(
                    fixture: PatchFixture = fixture, change: str = change
                ) -> None:
                    match change:
                        case "source-main":
                            fixture.fixture.source.refs[UV_DEV_REPOSITORY, "main"] = (
                                OTHER
                            )
                        case "public-main":
                            fixture.fixture.upstream.refs[UV_REPOSITORY, "main"] = None
                        case "parent-record":
                            fixture.fixture.source.comments[
                                fixture.fixture.parent.scope
                            ] = ()
                        case "parent-head":
                            parent = fixture.fixture.parent
                            fixture.fixture.source.pull_requests[parent.scope] = (
                                replace(
                                    parent,
                                    details=replace(
                                        parent.details,
                                        head=replace(parent.details.head, sha=OTHER),
                                    ),
                                )
                            )
                        case _:
                            raise AssertionError("Unexpected evidence fixture")

                fixture.after_patch = change_evidence
                with (
                    patch.dict(os.environ, {"GH_TOKEN": "local-write"}),
                    patch(
                        "uv_automations.github_promotion.subprocess.run",
                        side_effect=fixture.command,
                    ),
                    self.assertRaises(PromotionReadError),
                ):
                    retarget(
                        fixture.fixture.source,
                        fixture.fixture.upstream,
                        PromotionRetargetGitHub(token_variable="GH_TOKEN"),
                        plan,
                    )
                self.assertEqual(fixture.writes, [plan.source])

    def test_failed_reconciliation_read_does_not_repeat_the_patch(self) -> None:
        fixture = PatchFixture()
        plan = fixture.fixture.plan().plans[0]

        with (
            ExitStack() as patches,
            patch.dict(os.environ, {"GH_TOKEN": "local-write"}),
            patch(
                "uv_automations.github_promotion.subprocess.run",
                side_effect=fixture.command,
            ),
            self.assertRaises(PromotionReadError),
        ):

            def fail_reads() -> None:
                patches.enter_context(
                    patch.object(
                        fixture.fixture.source,
                        "get_ref",
                        side_effect=PromotionReadError("read failed"),
                    )
                )

            fixture.after_patch = fail_reads
            retarget(
                fixture.fixture.source,
                fixture.fixture.upstream,
                PromotionRetargetGitHub(token_variable="GH_TOKEN"),
                plan,
            )
        self.assertEqual(fixture.writes, [plan.source])

    def test_successful_response_uses_no_reconciliation_pass(self) -> None:
        fixture = RetargetFixture()
        plan = fixture.plan().plans[0]
        before = len(fixture.source.ref_calls)
        writer = FakeWriter(fixture.source)
        self.assertEqual(
            retarget(fixture.source, fixture.upstream, writer, plan),
            Retargeted(plan.source),
        )
        self.assertEqual(len(fixture.source.ref_calls) - before, 2)
        self.assertEqual(writer.writes, [plan.source])

    def test_unrelated_exceptions_are_not_reconciled(self) -> None:
        fixture = PatchFixture()
        fixture.failure = "programming"
        plan = fixture.fixture.plan().plans[0]
        before = len(fixture.fixture.source.ref_calls)
        with (
            patch.dict(os.environ, {"GH_TOKEN": "local-write"}),
            patch(
                "uv_automations.github_promotion.subprocess.run",
                side_effect=fixture.command,
            ),
            self.assertRaisesRegex(ValueError, "programming error"),
        ):
            retarget(
                fixture.fixture.source,
                fixture.fixture.upstream,
                PromotionRetargetGitHub(token_variable="GH_TOKEN"),
                plan,
            )
        self.assertEqual(len(fixture.fixture.source.ref_calls) - before, 2)
        self.assertEqual(fixture.writes, [plan.source])
