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
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from test_github_promotion_queue import MAIN, OLD, OTHER, PublicHistory
from test_promotion_wakeup import offline_process, outputs
from test_promotion_workflow_boundaries import job

from uv_automations import cli
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.github_promotion_completion import PromotionCompletionGitHub
from uv_automations.github_promotion_queue import (
    PromotionQueueCompletionGitHub,
    PromotionQueueGitHub,
    PromotionSyncCompletionGitHub,
)
from uv_automations.json import as_object
from uv_automations.models import CommitSha
from uv_automations.promotion_models import UV_DEV_REPOSITORY, UV_REPOSITORY

RUN = subprocess.run


class SyncFixture:
    """Exercise the real sync CLI and decoders through a rejecting API boundary."""

    def __init__(self) -> None:
        self.source_main: CommitSha | None = OLD
        self.public = PublicHistory()
        self.public_refs: list[CommitSha | None] = []
        self.public_after_post: PublicHistory | None = None
        self.after_post: CommitSha | None = MAIN
        self.after_post_refs: list[CommitSha | None] = []
        self.fork_refs: list[CommitSha | None] = []
        self.failures: dict[str, list[str]] = {}
        self.headers: dict[str, str] = {}
        self.calls: Counter[str] = Counter()
        self.trace: list[tuple[str, str | float]] = []
        self.credentials: list[tuple[str, str]] = []
        self.wrong_repository = ""
        self.wrong_repository_after_post = ""
        self.wrong_comparison_base = False

    def operation(self, method: str, endpoint: str, payload: object | None) -> str:
        if method == "GET" and endpoint == "repos/astral-sh/uv-dev":
            return "fork-repository"
        if method == "GET" and endpoint == "repos/astral-sh/uv":
            return "public-repository"
        if method == "POST" and endpoint == "graphql":
            data = as_object(payload)
            query = str(data["query"]).strip()
            variables = as_object(data["variables"])
            if (
                not query.startswith("query(")
                or "mutation" in query
                or "ref(qualifiedName: $ref)" not in query
                or variables.get("owner") != "astral-sh"
                or variables.get("ref") != "refs/heads/main"
                or set(variables) != {"owner", "name", "ref"}
            ):
                raise AssertionError("Unexpected GraphQL operation")
            if variables["name"] in {"uv", "uv-dev"}:
                return "public-ref" if variables["name"] == "uv" else "fork-ref"
        if method == "GET" and re.fullmatch(
            r"repos/astral-sh/uv/compare/[0-9a-f]{40}\.\.\.[0-9a-f]{40}\?per_page=1&page=1",
            endpoint,
        ):
            return "compare"
        if (
            method == "POST"
            and endpoint == "repos/astral-sh/uv-dev/merge-upstream"
            and payload == {"branch": "main"}
        ):
            return "merge"
        raise AssertionError(f"Unexpected GitHub endpoint: {method} {endpoint}")

    def api(self, operation: str, endpoint: str) -> object:
        wrong_repository = (
            self.wrong_repository_after_post
            if self.calls["merge"] and self.wrong_repository_after_post
            else self.wrong_repository
        )
        if operation.endswith("repository"):
            repository = (
                UV_DEV_REPOSITORY if operation == "fork-repository" else UV_REPOSITORY
            )
            return {
                "full_name": wrong_repository or str(repository.name),
                "id": repository.database_id,
            }
        if operation.endswith("ref"):
            repository = UV_DEV_REPOSITORY if operation == "fork-ref" else UV_REPOSITORY
            if operation == "public-ref":
                revision = (
                    self.public_refs.pop(0) if self.public_refs else self.public.main
                )
            elif self.fork_refs:
                revision = self.fork_refs.pop(0)
            elif self.calls["merge"] and self.after_post_refs:
                revision = self.after_post_refs.pop(0)
            else:
                revision = self.source_main
            return {
                "data": {
                    "repository": {
                        "nameWithOwner": wrong_repository or str(repository.name),
                        "databaseId": repository.database_id,
                        "ref": {
                            "name": "main",
                            "prefix": "refs/heads/",
                            "target": {"__typename": "Commit", "oid": str(revision)},
                        }
                        if revision is not None
                        else None,
                    }
                }
            }
        if operation == "compare":
            pair = endpoint.split("/compare/", 1)[1].split("?", 1)[0]
            base, head = (CommitSha(value) for value in pair.split("..."))
            comparison = self.public.compare_commits(UV_REPOSITORY, base, head)
            return {
                "base_commit": {
                    "sha": str(OTHER if self.wrong_comparison_base else base)
                },
                "status": comparison.status.value,
                "merge_base_commit": {"sha": str(comparison.merge_base)},
            }
        if operation == "merge":
            self.source_main = self.after_post
            if self.public_after_post is not None:
                self.public = self.public_after_post
            return {"message": "private response"}
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
        operation = self.operation(method, endpoint, payload)
        expected_tail = (["--include"] if "--include" in arguments else []) + (
            ["--input", "-"] if payload is not None else []
        )
        if arguments[5:] != expected_tail:
            raise AssertionError("Unexpected GitHub command options")
        environment = kwargs.get("env")
        token = (
            as_object(environment).get("GH_TOKEN")
            if environment is not None
            else os.environ.get("GH_TOKEN")
        )
        expected_token = (
            "local-upstream"
            if operation in {"public-repository", "public-ref", "compare"}
            else "local-write"
        )
        if token != expected_token:
            raise AssertionError("Unexpected credential source")
        self.credentials.append((operation, expected_token))
        self.calls[operation] += 1
        self.trace.append(("api", operation))
        failures = self.failures.get(operation, [])
        failure = failures.pop(0) if failures else ""
        result = None if failure.startswith("before") else self.api(operation, endpoint)
        status_match = re.search(r"HTTP ([1-5][0-9]{2})", failure)
        status = int(status_match[1]) if status_match else (503 if failure else 200)
        body = "private-invalid-json" if "malformed" in failure else json.dumps(result)
        malformed = "malformed" in failure
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
                {"GH_UPSTREAM_TOKEN": "local-upstream", "GH_TOKEN": "local-write"},
            ),
            patch("subprocess.run", side_effect=self.run),
            patch(
                "uv_automations.workflows.promotion_completion.sleep", side_effect=wait
            ),
        ):
            yield

    def invoke(self, root: Path) -> tuple[int, str, dict[str, str]]:
        section = job("sync-uv-dev.yml", "sync", "replay-queued-promotions")
        script = section.rsplit("        run: |\n", 1)[1]
        arguments = shlex.split(script.replace("\\\n", ""))
        if arguments[:4] != ["$AUTOMATIONS_PYTHON", "-I", "-m", "uv_automations"]:
            raise AssertionError("Unexpected sync entry point")
        arguments = [
            str(root / "output") if value == "$GITHUB_OUTPUT" else value
            for value in arguments[4:]
        ]
        stderr = io.StringIO()
        code = 0
        with self.offline(), redirect_stderr(stderr):
            try:
                cli.main(arguments)
            except SystemExit as error:
                if type(error.code) is not int:
                    raise AssertionError("Unexpected CLI exit") from error
                code = error.code
        output = root / "output"
        return code, stderr.getvalue(), outputs(output) if output.exists() else {}


class PromotionSyncCompletionTests(unittest.TestCase):
    def assert_synced(self, fixture: SyncFixture, expected: CommitSha = MAIN) -> None:
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"main-sha": str(expected)})
            )

    def assert_not_synced(
        self, fixture: SyncFixture, *, writes: int = 1, status: int = 1
    ) -> None:
        with TemporaryDirectory() as temporary:
            code, stderr, output = fixture.invoke(Path(temporary))
        self.assertEqual((code, output), (status, {}), stderr)
        self.assertNotIn("private", stderr)
        self.assertEqual(fixture.calls["merge"], writes)

    def assert_delayed_after_failures(self, fixture: SyncFixture, delay: float) -> None:
        failures = [
            index for index, event in enumerate(fixture.trace) if event[0] == "failed"
        ]
        self.assertTrue(failures)
        for index in failures:
            self.assertEqual(fixture.trace[index + 1], ("sleep", delay))
            self.assertEqual(fixture.trace[index + 2][0], "api")

    def test_committed_sync_with_lost_response_returns_the_verified_main(self) -> None:
        fixture = SyncFixture()
        fixture.failures["merge"] = ["after lost response"]
        with TemporaryDirectory() as temporary:
            self.assertEqual(
                fixture.invoke(Path(temporary)), (0, "", {"main-sha": str(MAIN)})
            )
        self.assertEqual(fixture.calls["merge"], 1)
        self.assertEqual(fixture.source_main, MAIN)

    def test_malformed_success_is_reconciled_without_exposing_response_fields(
        self,
    ) -> None:
        fixture = SyncFixture()
        fixture.failures["merge"] = ["after malformed success"]
        self.assert_synced(fixture)
        self.assertEqual(fixture.calls["merge"], 1)

    def test_failed_uncommitted_sync_never_repeats_the_write(self) -> None:
        fixture = SyncFixture()
        fixture.failures["merge"] = ["before HTTP 503"]
        self.assert_not_synced(fixture)
        self.assertEqual(fixture.source_main, OLD)

    def test_successful_response_is_not_proof_of_the_intended_revision(self) -> None:
        fixture = SyncFixture()
        fixture.after_post = OLD
        self.assert_not_synced(fixture)

    def test_missing_or_divergent_start_is_rejected_before_a_write(self) -> None:
        for source in (None, OTHER):
            with self.subTest(source=source):
                fixture = SyncFixture()
                fixture.source_main = source
                self.assert_not_synced(fixture, writes=0, status=2)
        fixture = SyncFixture()
        fixture.public.main = None
        self.assert_not_synced(fixture, writes=0, status=2)

    def test_missing_or_divergent_result_does_not_complete(self) -> None:
        for observed in (None, OTHER):
            with self.subTest(observed=observed):
                fixture = SyncFixture()
                fixture.after_post = observed
                self.assert_not_synced(fixture, status=1 if observed is None else 2)

    def test_forward_public_history_can_complete_beyond_the_pinned_intent(self) -> None:
        newer = CommitSha("d" * 40)
        fixture = SyncFixture()
        fixture.after_post = newer
        fixture.public_after_post = PublicHistory(
            newer, {(OLD, MAIN), (OLD, newer), (MAIN, newer)}
        )
        fixture.failures["merge"] = ["after HTTP 503"]
        self.assert_synced(fixture, newer)
        self.assertEqual(fixture.calls["merge"], 1)

    def test_rewritten_public_history_cannot_complete(self) -> None:
        fixture = SyncFixture()
        fixture.public_after_post = PublicHistory(OTHER, {(OLD, OTHER)})
        self.assert_not_synced(fixture, status=2)

    def test_fork_movement_during_the_proof_is_rechecked(self) -> None:
        fixture = SyncFixture()
        fixture.after_post_refs = [MAIN, OTHER, OTHER]
        self.assert_not_synced(fixture, status=2)

    def test_already_synchronized_state_needs_no_mutation(self) -> None:
        fixture = SyncFixture()
        fixture.source_main = MAIN
        self.assert_synced(fixture)
        self.assertEqual(fixture.calls["merge"], 0)

    def test_concurrent_public_sync_is_verified_without_another_write(self) -> None:
        newer = CommitSha("d" * 40)
        fixture = SyncFixture()
        fixture.source_main = newer
        fixture.public = PublicHistory(
            newer, {(OLD, MAIN), (OLD, newer), (MAIN, newer)}
        )
        fixture.public_refs = [MAIN]
        self.assert_synced(fixture, newer)
        self.assertEqual(fixture.calls["merge"], 0)

    def test_observed_completion_cannot_regain_write_authority(self) -> None:
        fixture = SyncFixture()
        fixture.fork_refs = [MAIN, None, OLD, OLD]
        self.assert_not_synced(fixture, writes=0)

    def test_every_failed_response_delays_the_next_api_call(self) -> None:
        now = datetime(2026, 9, 24, 12, 0, 0, tzinfo=UTC)
        for target in ("merge", "fork-ref", "public-ref", "compare", "final-ref"):
            for status, header in ((429, "20"), (503, "Thu, 24 Sep 2026 12:00:20 GMT")):
                with self.subTest(target=target, status=status, header=header):
                    fixture = SyncFixture()
                    operation = "fork-ref" if target == "final-ref" else target
                    prefix = (
                        []
                        if target == "merge"
                        else [""] * (2 if target == "final-ref" else 1)
                    )
                    fixture.failures[operation] = prefix + [
                        f"{'after' if target == 'merge' else 'before'} HTTP {status}"
                    ]
                    fixture.headers[operation] = f"Retry-After: {header}\r\n"
                    with patch(
                        "uv_automations.github_promotion_completion.datetime"
                    ) as clock:
                        clock.now.return_value = now
                        self.assert_synced(fixture)
                    self.assert_delayed_after_failures(fixture, 20.0)
                    self.assertEqual(fixture.calls["merge"], 1)

    def test_final_attempt_still_reconciles_the_single_ambiguous_write(self) -> None:
        fixture = SyncFixture()
        fixture.failures["fork-repository"] = ["before HTTP 503"] * 2
        fixture.failures["merge"] = ["after HTTP 429"]
        fixture.headers["fork-repository"] = "Retry-After: 6\r\n"
        fixture.headers["merge"] = "Retry-After: 6\r\n"
        self.assert_synced(fixture)
        self.assert_delayed_after_failures(fixture, 6.0)
        self.assertEqual(fixture.calls["merge"], 1)

    def test_permanent_or_over_budget_error_has_no_followup(self) -> None:
        for target in ("merge", "public-ref", "fork-ref", "compare"):
            for status, header in (
                (401, ""),
                (422, ""),
                (429, "31"),
                (503, "999999999999999999"),
            ):
                with self.subTest(target=target, status=status, header=header):
                    fixture = SyncFixture()
                    prefix = [""] if target in {"fork-ref", "compare"} else []
                    fixture.failures[target] = prefix + [
                        f"{'after' if target == 'merge' else 'before'} HTTP {status}"
                    ]
                    if header:
                        fixture.headers[target] = f"Retry-After: {header}\r\n"
                    self.assert_not_synced(
                        fixture, writes=0 if target == "public-ref" else 1
                    )
                    self.assertEqual(fixture.trace[-1], ("failed", target))
                    self.assertFalse(
                        any(event[0] == "sleep" for event in fixture.trace)
                    )

    def test_repeated_read_errors_exhaust_the_bound_without_another_write(self) -> None:
        fixture = SyncFixture()
        fixture.failures["fork-ref"] = [""] + ["before HTTP 503"] * 3
        self.assert_not_synced(fixture)
        self.assertEqual(fixture.calls["fork-ref"], 4)

    def test_wrong_repository_or_comparison_identity_is_not_admitted(self) -> None:
        fixture = SyncFixture()
        fixture.wrong_repository = "astral-sh/uv-security"
        self.assert_not_synced(fixture, writes=0)
        fixture = SyncFixture()
        fixture.wrong_comparison_base = True
        self.assert_not_synced(fixture, writes=0)
        fixture = SyncFixture()
        fixture.wrong_repository_after_post = "astral-sh/uv-security"
        self.assert_not_synced(fixture)

    def test_completion_transport_is_selected_only_for_the_sync_stage(self) -> None:
        self.assertIs(PromotionQueueGitHub._api, PromotionGitHub._api)
        self.assertIs(
            PromotionQueueCompletionGitHub._api, PromotionCompletionGitHub._api
        )
        self.assertIs(
            PromotionSyncCompletionGitHub._api, PromotionCompletionGitHub._api
        )
        fixture = SyncFixture()
        self.assert_synced(fixture)
        self.assertIn(("merge", "local-write"), fixture.credentials)
        self.assertIn(("compare", "local-upstream"), fixture.credentials)
        self.assertEqual(fixture.calls["merge"], 1)

    def test_sync_and_replay_keep_the_existing_workflow_authority(self) -> None:
        sync = job("sync-uv-dev.yml", "sync", "replay-queued-promotions")
        replay = job("sync-uv-dev.yml", "replay-queued-promotions")
        self.assertIn("main-sha: ${{ steps.sync.outputs.main-sha }}", sync)
        self.assertIn("GH_UPSTREAM_TOKEN: ${{ github.token }}", sync)
        self.assertIn("needs: sync", replay)
        self.assertIn("main-sha: ${{ needs.sync.outputs.main-sha }}", replay)
        self.assertNotIn("merge-upstream", sync)


if __name__ == "__main__":
    unittest.main()
