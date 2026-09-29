import io
import json
import os
import re
import shlex
import subprocess
import sys
import unittest
from collections import Counter
from collections.abc import Iterator
from contextlib import contextmanager, redirect_stderr
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from test_promotion import (
    BASE,
    HEAD,
    READY,
    SOURCE,
    UPDATED,
    UPSTREAM_PARENT,
    event_payload,
    graphql_repository,
    pull_request,
    pull_request_payload,
    repository_payload,
)
from test_promotion_wakeup import offline_process, outputs
from test_promotion_workflow_boundaries import job

from uv_automations import cli
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.github_promotion_completion import PromotionCompletionGitHub
from uv_automations.github_promotion_queue import (
    PromotionBaseCompletionGitHub,
    PromotionQueueGitHub,
)
from uv_automations.json import as_object
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    PromotionApproval,
)
from uv_automations.workflows.promotion import CopyUpstreamBase

RUN = subprocess.run


class BaseFixture:
    """Exercise the real base-copy CLI and claim through an offline API boundary."""

    def __init__(self) -> None:
        self.source = pull_request(base_ref="parent")
        self.parent = pull_request(
            UPSTREAM_PARENT,
            head_ref="parent",
            head_sha=BASE,
            head_repository=UV_DEV_REPOSITORY,
        )
        approval = PromotionApproval(SOURCE, HEAD, READY, READY.identifier)
        self.claim = CopyUpstreamBase(
            self.source,
            approval,
            self.parent,
            BranchRevision(UV_REPOSITORY, "parent", BASE),
        ).claim
        self.destination: CommitSha | None = None
        self.after_post: CommitSha | None = BASE
        self.writer_refs: list[CommitSha | None] = []
        self.after_post_refs: list[CommitSha | None] = []
        self.events = [event_payload(READY.identifier, "ready_for_review")]
        self.claim_data: object | None = None
        self.wrong_repository = False
        self.wrong_ref_after_post = False
        self.wrong_target_after_post = False
        self.failures: dict[str, list[str]] = {}
        self.headers: dict[str, str] = {}
        self.calls: Counter[str] = Counter()
        self.trace: list[tuple[str, str | float]] = []

    def operation(
        self, method: str, endpoint: str, payload: object | None, token: object
    ) -> str:
        if token not in {"local-read", "local-write"}:
            raise AssertionError("Unexpected credential source")
        prefix = "read" if token == "local-read" else "write"
        if method == "GET" and endpoint == "repos/astral-sh/uv":
            return prefix + "-repository"
        if method == "POST" and endpoint == "graphql":
            data = as_object(payload)
            query = str(data["query"]).strip()
            if (
                not query.startswith("query(")
                or "mutation" in query
                or "ref(qualifiedName: $ref)" not in query
                or as_object(data["variables"])
                != {"owner": "astral-sh", "name": "uv", "ref": "refs/heads/parent"}
            ):
                raise AssertionError("Unexpected GraphQL operation")
            return prefix + "-ref"
        if token == "local-read" and method == "GET":
            if endpoint == "repos/astral-sh/uv-dev/pulls/101":
                return "source"
            if (
                endpoint
                == "repos/astral-sh/uv-dev/issues/101/events?per_page=100&page=1"
            ):
                return "events"
            if endpoint == (
                "repos/astral-sh/uv/pulls?state=open&sort=created&direction=asc"
                "&head=astral-sh%3Aparent&per_page=100&page=1"
            ):
                return "parents"
            if endpoint == (
                "repos/astral-sh/uv/pulls?state=all&sort=created&direction=asc"
                "&head=astral-sh%3Afeature&per_page=100&page=1"
            ):
                return "publications"
        if (
            token == "local-write"
            and method == "POST"
            and endpoint == "repos/astral-sh/uv/git/refs"
            and payload == {"ref": "refs/heads/parent", "sha": str(BASE)}
        ):
            return "create"
        raise AssertionError(f"Unexpected GitHub endpoint: {method} {endpoint}")

    def api(self, operation: str) -> object:
        if operation.endswith("repository"):
            if self.wrong_repository and operation == "write-repository":
                return repository_payload(UV_DEV_REPOSITORY)
            return repository_payload(UV_REPOSITORY)
        if operation.endswith("ref"):
            revision = self.destination
            if operation == "write-ref":
                if self.writer_refs:
                    revision = self.writer_refs.pop(0)
                elif self.calls["create"] and self.after_post_refs:
                    revision = self.after_post_refs.pop(0)
            ref = (
                "other"
                if self.calls["create"] and self.wrong_ref_after_post
                else "parent"
            )
            target_kind = (
                "Tag"
                if self.calls["create"] and self.wrong_target_after_post
                else "Commit"
            )
            return {
                "data": {
                    "repository": {
                        **graphql_repository(UV_REPOSITORY),
                        "ref": {
                            "name": ref,
                            "prefix": "refs/heads/",
                            "target": {
                                "__typename": target_kind,
                                "oid": str(revision),
                            },
                        }
                        if revision is not None
                        else None,
                    }
                }
            }
        if operation == "source":
            return pull_request_payload(self.source)
        if operation == "events":
            return self.events
        if operation == "parents":
            return [pull_request_payload(self.parent)]
        if operation == "publications":
            return []
        if operation == "create":
            self.destination = self.after_post
            return {"ref": "refs/heads/parent", "object": {"sha": str(BASE)}}
        raise AssertionError("Unexpected fixture operation")

    def run(
        self, arguments: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        if (
            len(arguments) == 3
            and arguments[:2] == ["git", "check-ref-format"]
            and arguments[2].startswith("refs/heads/")
        ):
            return RUN(
                arguments, check=False, capture_output=True, text=True, timeout=30
            )
        if arguments[:3] != ["gh", "api", "--method"]:
            raise AssertionError(f"Unexpected external command: {arguments}")
        method, endpoint = arguments[3:5]
        payload = json.loads(str(kwargs["input"])) if kwargs.get("input") else None
        environment = kwargs.get("env")
        token = (
            as_object(environment).get("GH_TOKEN")
            if environment is not None
            else os.environ.get("GH_TOKEN")
        )
        operation = self.operation(method, endpoint, payload, token)
        expected_tail = (["--include"] if "--include" in arguments else []) + (
            ["--input", "-"] if payload is not None else []
        )
        if arguments[5:] != expected_tail:
            raise AssertionError("Unexpected GitHub command options")
        self.calls[operation] += 1
        self.trace.append(("api", operation))
        failures = self.failures.get(operation, [])
        failure = failures.pop(0) if failures else ""
        result = None if failure.startswith("before") else self.api(operation)
        status_match = re.search(r"HTTP ([1-5][0-9]{2})", failure)
        status = int(status_match[1]) if status_match else (503 if failure else 200)
        malformed = "malformed" in failure
        body = "private-invalid-json" if malformed else json.dumps(result)
        if malformed:
            status = 200
        if "--include" in arguments:
            body = f"HTTP/2.0 {status} Status\r\n{self.headers.get(operation, '')}\r\n{body}"
        if failure:
            self.trace.append(("failed", operation))
        return subprocess.CompletedProcess(
            arguments, int(bool(failure) and not malformed), body, failure
        )

    @contextmanager
    def offline(self) -> Iterator[None]:
        def wait(delay: float) -> None:
            self.trace.append(("sleep", delay))

        with (
            offline_process(),
            patch.dict(
                os.environ,
                {"GH_READ_TOKEN": "local-read", "GH_TOKEN": "local-write"},
            ),
            patch("subprocess.run", side_effect=self.run),
            patch(
                "uv_automations.workflows.promotion_completion.sleep", side_effect=wait
            ),
        ):
            yield

    def invoke(self, root: Path) -> tuple[int, str, dict[str, str]]:
        section = job("promote-pull-request.yml", "promote", "replay-promoted-children")
        section = section.split("      - name: Ensure upstream base\n", 1)[1]
        section = section.split("      - name: Promote pull request\n", 1)[0]
        script = section.rsplit("        run: |\n", 1)[1]
        arguments = shlex.split(script.replace("\\\n", ""))
        if arguments[:4] != [
            "$AUTOMATIONS_PYTHON",
            "-I",
            "-m",
            "uv_automations",
        ] or arguments[-2:] != ["<<<", "$COPY_BASE"]:
            raise AssertionError("Unexpected ensure-base entry point")
        environment = {
            "GITHUB_REPOSITORY": str(SOURCE.repository.name),
            "GITHUB_REPOSITORY_ID": str(SOURCE.repository.database_id),
            "PULL_REQUEST_NUMBER": str(SOURCE.number),
            "GITHUB_OUTPUT": str(root / "output"),
            "GITHUB_STEP_SUMMARY": str(root / "summary"),
        }
        arguments = [
            environment[value.removeprefix("$")] if value.startswith("$") else value
            for value in arguments[4:-2]
        ]
        stderr = io.StringIO()
        code = 0
        with (
            self.offline(),
            patch.object(
                sys,
                "stdin",
                io.StringIO(
                    json.dumps(
                        self.claim.to_json()
                        if self.claim_data is None
                        else self.claim_data
                    )
                ),
            ),
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


class PromotionBaseCompletionTests(unittest.TestCase):
    def assert_ready(self, fixture: BaseFixture, *, writes: int = 1) -> None:
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"ready": "true"})
            )
        self.assertEqual(fixture.calls["create"], writes)

    def assert_not_ready(
        self, fixture: BaseFixture, *, writes: int = 1, status: int = 1
    ) -> None:
        with TemporaryDirectory() as temporary:
            code, stderr, output = fixture.invoke(Path(temporary))
        self.assertEqual((code, output), (status, {}), stderr)
        self.assertNotIn("private", stderr)
        self.assertEqual(fixture.calls["create"], writes)

    def assert_delayed_after_failures(self, fixture: BaseFixture, delay: float) -> None:
        failures = [
            index for index, event in enumerate(fixture.trace) if event[0] == "failed"
        ]
        self.assertTrue(failures)
        for index in failures:
            self.assertEqual(fixture.trace[index + 1], ("sleep", delay))
            self.assertEqual(fixture.trace[index + 2][0], "api")

    def test_committed_create_with_transient_read_returns_ready(self) -> None:
        fixture = BaseFixture()
        fixture.failures["create"] = ["after HTTP 503"]
        fixture.failures["write-ref"] = ["", "before HTTP 429"]
        fixture.headers["write-ref"] = "Retry-After: 7\r\n"
        with TemporaryDirectory() as temporary:
            result = fixture.invoke(Path(temporary))
        self.assertEqual(fixture.calls["create"], 1)
        self.assertEqual(fixture.destination, BASE)
        self.assertEqual(result, (0, "", {"ready": "true"}))

    def test_successful_create_and_malformed_response_are_reconciled(self) -> None:
        for failure in ("", "after malformed success"):
            with self.subTest(failure=failure):
                fixture = BaseFixture()
                fixture.failures["create"] = [failure]
                self.assert_ready(fixture)
                self.assertEqual(fixture.destination, BASE)

    def test_existing_exact_destination_and_identical_race_do_not_write(self) -> None:
        fixture = BaseFixture()
        fixture.destination = BASE
        self.assert_ready(fixture, writes=0)
        fixture = BaseFixture()
        fixture.writer_refs = [BASE]
        self.assert_ready(fixture, writes=0)

    def test_identical_post_create_conflict_is_reconciled(self) -> None:
        for status in (409, 422):
            with self.subTest(status=status):
                fixture = BaseFixture()
                fixture.failures["create"] = [f"after HTTP {status}"]
                self.assert_ready(fixture)
                self.assertEqual(fixture.destination, BASE)
                self.assertEqual(fixture.calls["write-ref"], 2)
                self.assert_delayed_after_failures(fixture, 1.0)

    def test_post_create_conflict_cannot_admit_another_or_missing_commit(self) -> None:
        for status in (409, 422):
            with self.subTest(status=status):
                fixture = BaseFixture()
                fixture.after_post = UPDATED
                fixture.failures["create"] = [f"after HTTP {status}"]
                self.assert_not_ready(fixture)
                self.assertEqual(fixture.destination, UPDATED)
                fixture = BaseFixture()
                fixture.failures["create"] = [f"before HTTP {status}"]
                self.assert_not_ready(fixture)
                self.assertIsNone(fixture.destination)
                self.assertEqual(fixture.calls["write-ref"], 4)

    def test_post_create_conflict_honors_retry_after(self) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for status in (409, 422):
            for header in ("20", "Thu, 24 Sep 2026 12:00:20 GMT"):
                with self.subTest(status=status, header=header):
                    fixture = BaseFixture()
                    fixture.failures["create"] = [f"after HTTP {status}"]
                    fixture.headers["create"] = f"Retry-After: {header}\r\n"
                    with patch(
                        "uv_automations.github_promotion_completion.datetime"
                    ) as clock:
                        clock.now.return_value = now
                        self.assert_ready(fixture)
                    self.assert_delayed_after_failures(fixture, 20.0)

    def test_existing_different_destination_is_never_overwritten(self) -> None:
        fixture = BaseFixture()
        fixture.writer_refs = [UPDATED]
        self.assert_not_ready(fixture, writes=0, status=2)

    def test_failed_uncommitted_create_never_repeats_the_write(self) -> None:
        fixture = BaseFixture()
        fixture.failures["create"] = ["before HTTP 503"]
        self.assert_not_ready(fixture)
        self.assertIsNone(fixture.destination)
        self.assertEqual(fixture.calls["write-ref"], 4)

    def test_successful_response_is_not_proof_of_the_expected_destination(self) -> None:
        fixture = BaseFixture()
        fixture.after_post = None
        self.assert_not_ready(fixture)
        fixture = BaseFixture()
        fixture.after_post = UPDATED
        self.assert_not_ready(fixture, status=2)

    def test_ambiguous_create_cannot_admit_a_different_commit(self) -> None:
        fixture = BaseFixture()
        fixture.after_post = UPDATED
        fixture.failures["create"] = ["after HTTP 503"]
        self.assert_not_ready(fixture)

    def test_temporarily_missing_ref_can_complete_without_another_write(self) -> None:
        fixture = BaseFixture()
        fixture.after_post_refs = [None, None, BASE]
        self.assert_ready(fixture)
        self.assertEqual(fixture.calls["write-ref"], 4)

    def test_every_failed_response_delays_the_next_api_call(self) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for target in ("write-repository", "write-ref", "create", "post-ref"):
            for status, header in ((429, "20"), (503, "Thu, 24 Sep 2026 12:00:20 GMT")):
                with self.subTest(target=target, status=status, header=header):
                    fixture = BaseFixture()
                    operation = "write-ref" if target == "post-ref" else target
                    fixture.failures[operation] = (
                        [""] if target == "post-ref" else []
                    ) + [f"{'after' if target == 'create' else 'before'} HTTP {status}"]
                    fixture.headers[operation] = f"Retry-After: {header}\r\n"
                    with patch(
                        "uv_automations.github_promotion_completion.datetime"
                    ) as clock:
                        clock.now.return_value = now
                        self.assert_ready(fixture)
                    self.assert_delayed_after_failures(fixture, 20.0)

    def test_final_attempt_still_reconciles_the_single_ambiguous_write(self) -> None:
        fixture = BaseFixture()
        fixture.failures["write-repository"] = ["before HTTP 503"] * 2
        fixture.failures["create"] = ["after HTTP 429"]
        fixture.headers["write-repository"] = "Retry-After: 6\r\n"
        fixture.headers["create"] = "Retry-After: 6\r\n"
        self.assert_ready(fixture)
        self.assert_delayed_after_failures(fixture, 6.0)

    def test_permanent_or_over_budget_error_has_no_followup(self) -> None:
        for target in ("write-repository", "write-ref", "create", "post-ref"):
            for status, header in (
                (401, ""),
                (403, ""),
                (409, ""),
                (409, "31"),
                (422, ""),
                (422, "31"),
                (429, "31"),
                (503, "999999999999999999"),
            ):
                if target == "create" and status in {409, 422} and not header:
                    continue
                with self.subTest(target=target, status=status, header=header):
                    fixture = BaseFixture()
                    operation = "write-ref" if target == "post-ref" else target
                    fixture.failures[operation] = (
                        [""] if target == "post-ref" else []
                    ) + [f"{'after' if target == 'create' else 'before'} HTTP {status}"]
                    if header:
                        fixture.headers[operation] = f"Retry-After: {header}\r\n"
                    self.assert_not_ready(
                        fixture, writes=int(target in {"create", "post-ref"})
                    )
                    self.assertEqual(fixture.trace[-1], ("failed", operation))
                    self.assertFalse(
                        any(event[0] == "sleep" for event in fixture.trace)
                    )

    def test_repeated_read_errors_exhaust_without_another_write(self) -> None:
        fixture = BaseFixture()
        fixture.failures["write-ref"] = [""] + ["before HTTP 503"] * 3
        self.assert_not_ready(fixture)
        self.assertEqual(fixture.calls["write-ref"], 4)

    def test_wrong_repository_or_ref_identity_is_not_admitted(self) -> None:
        fixture = BaseFixture()
        fixture.wrong_repository = True
        self.assert_not_ready(fixture, writes=0)
        for attribute in ("wrong_ref_after_post", "wrong_target_after_post"):
            with self.subTest(attribute=attribute):
                fixture = BaseFixture()
                setattr(fixture, attribute, True)
                self.assert_not_ready(fixture)

    def test_changed_approval_source_or_parent_does_not_write(self) -> None:
        for changed in ("approval", "source", "parent"):
            with self.subTest(changed=changed):
                fixture = BaseFixture()
                if changed == "approval":
                    fixture.events = []
                elif changed == "source":
                    fixture.source = replace(
                        fixture.source,
                        details=replace(
                            fixture.source.details,
                            head=replace(fixture.source.details.head, sha=UPDATED),
                        ),
                    )
                else:
                    fixture.parent = replace(
                        fixture.parent,
                        details=replace(
                            fixture.parent.details,
                            head=replace(fixture.parent.details.head, sha=UPDATED),
                        ),
                    )
                with TemporaryDirectory() as temporary:
                    code, stderr, output = fixture.invoke(Path(temporary))
                self.assertEqual((code, stderr, output.get("ready")), (0, "", "false"))
                self.assertIn("rejection_reason", output)
                self.assertEqual(fixture.calls["create"], 0)

    def test_inconsistent_destination_claim_is_rejected_before_any_api(self) -> None:
        for key, value in (
            ("repository", "astral-sh/uv-dev"),
            ("repository_id", UV_DEV_REPOSITORY.database_id),
            ("ref", "other"),
            ("sha", str(UPDATED)),
        ):
            with self.subTest(key=key):
                fixture = BaseFixture()
                claim = fixture.claim.to_json()
                claim["destination"] = {**as_object(claim["destination"]), key: value}
                fixture.claim_data = claim
                self.assert_not_ready(fixture, writes=0, status=2)
                self.assertFalse(fixture.trace)

    def test_completion_transport_keeps_the_existing_token_boundary(self) -> None:
        self.assertIs(PromotionQueueGitHub._api, PromotionGitHub._api)
        self.assertIs(
            PromotionBaseCompletionGitHub._api, PromotionCompletionGitHub._api
        )
        fixture = BaseFixture()
        self.assert_ready(fixture)
        self.assertGreater(fixture.calls["source"], 0)
        self.assertGreater(fixture.calls["read-ref"], 0)
        self.assertGreater(fixture.calls["write-ref"], 0)

    def test_base_copy_keeps_the_existing_workflow_authority(self) -> None:
        publisher = job(
            "promote-pull-request.yml", "promote", "replay-promoted-children"
        )
        self.assertIn("GH_READ_TOKEN: ${{ steps.read-token.outputs.token }}", publisher)
        self.assertIn("GH_TOKEN: ${{ steps.token.outputs.token }}", publisher)
        self.assertIn("steps.base.outputs.ready != 'false'", publisher)


if __name__ == "__main__":
    unittest.main()
