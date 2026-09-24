import copy
import subprocess
import unittest
from collections import Counter
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations import cli, promotions_cli
from uv_automations.github_promotion import PromotionGitHub, PromotionReadError
from uv_automations.json import as_object
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    AUTOMATIONS_APP_ID,
    AUTOMATIONS_APP_SLUG,
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    PromotionScope,
)
from uv_automations.workflows.promotion import PromotionRequest
from uv_automations.workflows.promotion_closed import (
    ObservedClosedPromotion,
    inspect_completed_promotion,
)

HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
OTHER = CommitSha("c" * 40)
SOURCE = PromotionScope(UV_DEV_REPOSITORY, 2053)
UPSTREAM = PromotionScope(UV_REPOSITORY, 21948)
REQUEST = PromotionRequest(SOURCE, HEAD)
COMMENT_ID = 5_000_000_001
NOW = "2026-09-09T12:00:00Z"
COMMENT = (
    f"Promoted to [#{UPSTREAM.number}]"
    f"(https://github.com/astral-sh/uv/pull/{UPSTREAM.number})."
)
RUN = subprocess.run


def repository(scope: PromotionScope) -> dict[str, object]:
    return {
        "full_name": str(scope.repository.name),
        "id": scope.repository.database_id,
    }


def graph_repository(scope: PromotionScope) -> dict[str, object]:
    return {
        "nameWithOwner": str(scope.repository.name),
        "databaseId": scope.repository.database_id,
    }


def bot() -> dict[str, object]:
    return {
        "login": f"{AUTOMATIONS_APP_SLUG}[bot]",
        "id": AUTOMATIONS_BOT_ID,
        "type": "Bot",
    }


def pull_request(scope: PromotionScope, *, closed: bool) -> dict[str, object]:
    return {
        "number": scope.number,
        "html_url": f"https://github.com/{scope.repository.name}/pull/{scope.number}",
        "title": "Promotion observation fixture",
        "body": "",
        "state": "closed" if closed else "open",
        "draft": False,
        "labels": [],
        "user": bot(),
        "merged_at": None,
        "merge_commit_sha": None,
        "base": {"ref": "main", "sha": str(BASE), "repo": repository(scope)},
        "head": {
            "ref": "fixture/promotion",
            "sha": str(HEAD),
            "repo": repository(scope),
        },
    }


def receipt() -> dict[str, object]:
    return {
        "id": COMMENT_ID,
        "node_id": "IC_observation",
        "issue_url": f"https://api.github.com/repos/{SOURCE.repository.name}/issues/{SOURCE.number}",
        "user": bot(),
        "performed_via_github_app": {
            "id": AUTOMATIONS_APP_ID,
            "slug": AUTOMATIONS_APP_SLUG,
        },
        "body": COMMENT,
        "created_at": NOW,
        "updated_at": NOW,
    }


class ObservationFixture:
    """Use the real GitHub decoders with an entirely read-only local transport."""

    def __init__(self) -> None:
        self.source = pull_request(SOURCE, closed=True)
        self.upstream = pull_request(UPSTREAM, closed=False)
        self.comments = [receipt()]
        self.direct = receipt()
        self.graph: dict[str, object] = {
            "__typename": "IssueComment",
            "id": "IC_observation",
            "fullDatabaseId": str(COMMENT_ID),
            "body": COMMENT,
            "lastEditedAt": None,
            "editor": None,
            "repository": graph_repository(SOURCE),
            "pullRequest": {
                "number": SOURCE.number,
                "repository": graph_repository(SOURCE),
            },
        }
        self.source_after: dict[str, object] | None = None
        self.upstream_after: dict[str, object] | None = None
        self.pages: dict[int, list[object]] | None = None
        self.calls: Counter[str] = Counter()
        self.trace: list[str] = []
        self.receipt_read = False

    def api(
        self, method: str, endpoint: str, *, payload: object | None = None
    ) -> object:
        operation = f"{method} {endpoint}"
        self.calls[operation] += 1
        self.trace.append(operation)
        if method == "GET" and endpoint == (
            f"repos/{SOURCE.repository.name}/pulls/{SOURCE.number}"
        ):
            value = (
                self.source_after
                if self.receipt_read and self.source_after is not None
                else self.source
            )
            return copy.deepcopy(value)
        if method == "GET" and endpoint == (
            f"repos/{UPSTREAM.repository.name}/pulls/{UPSTREAM.number}"
        ):
            value = (
                self.upstream_after
                if self.calls[operation] > 1 and self.upstream_after is not None
                else self.upstream
            )
            return copy.deepcopy(value)
        comments = (
            f"repos/{SOURCE.repository.name}/issues/{SOURCE.number}/comments"
            "?per_page=100&page="
        )
        if method == "GET" and endpoint.startswith(comments):
            page = int(endpoint.removeprefix(comments))
            return copy.deepcopy(
                self.pages.get(page, [])
                if self.pages is not None
                else self.comments
                if page == 1
                else []
            )
        if method == "GET" and endpoint == (
            f"repos/{SOURCE.repository.name}/issues/comments/{COMMENT_ID}"
        ):
            return copy.deepcopy(self.direct)
        if method == "POST" and endpoint == "graphql":
            query = as_object(payload)
            if query["variables"] != {"id": "IC_observation"} or not str(
                query["query"]
            ).lstrip().startswith("query($id: ID!)"):
                raise AssertionError("Unexpected GraphQL operation")
            self.receipt_read = True
            return {"data": {"node": copy.deepcopy(self.graph)}}
        raise AssertionError(f"Unexpected GitHub operation: {operation}")

    @contextmanager
    def offline(self) -> Iterator[None]:
        def run(
            arguments: list[str], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            if (
                len(arguments) == 3
                and arguments[:2] == ["git", "check-ref-format"]
                and arguments[2].startswith("refs/heads/")
            ):
                return RUN(
                    arguments, check=False, capture_output=True, text=True, timeout=30
                )
            raise AssertionError(f"Unexpected subprocess: {arguments}")

        with (
            patch.object(PromotionGitHub, "_api", side_effect=self.api),
            patch.object(
                PromotionGitHub,
                "_command",
                side_effect=AssertionError("Unexpected GitHub command"),
            ),
            patch(
                "subprocess.run",
                side_effect=run,
            ),
            patch.object(
                promotions_cli,
                "PromotionCompletionGitHub",
                side_effect=AssertionError("Unexpected completion writer"),
            ),
            patch.object(
                promotions_cli,
                "PromotionQueueGitHub",
                side_effect=AssertionError("Unexpected queue writer"),
            ),
        ):
            yield

    def observe(
        self, request: PromotionRequest = REQUEST
    ) -> ObservedClosedPromotion | None:
        with self.offline():
            return inspect_completed_promotion(PromotionGitHub(), request)


class ClosedPromotionTests(unittest.TestCase):
    def assert_unrecognized(self, fixture: ObservationFixture) -> None:
        try:
            self.assertIsNone(fixture.observe())
        except PromotionReadError:
            pass

    def test_exact_direct_publication_is_observed_without_writes(self) -> None:
        for merged in (False, True):
            with self.subTest(merged=merged):
                fixture = ObservationFixture()
                if merged:
                    fixture.upstream.update(
                        state="closed", merged_at=NOW, merge_commit_sha=str(OTHER)
                    )
                observed = fixture.observe()
                self.assertIsInstance(observed, ObservedClosedPromotion)
                if observed is None:
                    self.fail("The exact publication should be observed")
                self.assertEqual(observed.source.scope, SOURCE)
                self.assertEqual(observed.head, HEAD)
                self.assertEqual(observed.upstream.scope, UPSTREAM)
                self.assertEqual(observed.receipt.comment.identifier, COMMENT_ID)
                self.assertEqual(
                    fixture.trace[-2:],
                    [
                        f"GET repos/{SOURCE.repository.name}/pulls/{SOURCE.number}",
                        f"GET repos/{UPSTREAM.repository.name}/pulls/{UPSTREAM.number}",
                    ],
                )
                self.assertEqual(
                    [call for call in fixture.trace if not call.startswith("GET ")],
                    ["POST graphql"],
                )
                with self.assertRaises(ValueError):
                    replace(observed, head=OTHER)

    def test_historical_observation_does_not_reconstruct_current_approval(self) -> None:
        fixture = ObservationFixture()
        fixture.source.update(draft=True, labels=[])
        self.assertIsInstance(fixture.observe(), ObservedClosedPromotion)
        self.assertFalse(any("/events" in call for call in fixture.trace))

    def test_exact_receipt_and_advancing_base_tips_are_compatible(self) -> None:
        fixture = ObservationFixture()
        fixture.direct["performed_via_github_app"] = None
        fixture.comments.append({**receipt(), "id": COMMENT_ID + 1})
        fixture.source_after = copy.deepcopy(fixture.source)
        fixture.upstream_after = copy.deepcopy(fixture.upstream)
        as_object(fixture.source_after["base"])["sha"] = str(OTHER)
        as_object(fixture.upstream_after["base"])["sha"] = str(OTHER)
        self.assertIsInstance(fixture.observe(), ObservedClosedPromotion)

    def test_queued_and_private_requests_are_not_inspected(self) -> None:
        for request in (
            PromotionRequest(SOURCE, HEAD, 1000),
            PromotionRequest(PromotionScope(UV_SECURITY_REPOSITORY, 2053), HEAD),
        ):
            with self.subTest(request=request):
                fixture = ObservationFixture()
                self.assertIsNone(fixture.observe(request))
                self.assertEqual(fixture.trace, [])

    def test_missing_counterfeit_and_conflicting_receipts_are_not_observed(
        self,
    ) -> None:
        for comments in (
            [],
            [{**receipt(), "user": {"login": "zanieb", "id": 1, "type": "User"}}],
            [{**receipt(), "user": {**bot(), "id": AUTOMATIONS_BOT_ID + 1}}],
            [{**receipt(), "performed_via_github_app": None}],
            [
                {
                    **receipt(),
                    "performed_via_github_app": {
                        "id": AUTOMATIONS_APP_ID + 1,
                        "slug": AUTOMATIONS_APP_SLUG,
                    },
                }
            ],
            [
                receipt(),
                {
                    **receipt(),
                    "id": COMMENT_ID + 1,
                    "body": "Promoted to [#9](https://github.com/astral-sh/uv/pull/9).",
                },
            ],
            [
                {
                    **receipt(),
                    "performed_via_github_app": {
                        "id": AUTOMATIONS_APP_ID,
                        "slug": "other-app",
                    },
                }
            ],
        ):
            with self.subTest(comments=comments):
                fixture = ObservationFixture()
                fixture.comments = comments
                self.assert_unrecognized(fixture)

    def test_selected_receipt_requires_exact_unedited_rest_and_graphql_proof(
        self,
    ) -> None:
        for target, field, value in (
            ("direct", "id", COMMENT_ID + 1),
            (
                "direct",
                "issue_url",
                "https://api.github.com/repos/astral-sh/uv/issues/9",
            ),
            ("direct", "body", "Another comment"),
            ("direct", "user", {**bot(), "id": AUTOMATIONS_BOT_ID + 1}),
            ("direct", "updated_at", "2026-09-09T12:00:01Z"),
            ("graph", "lastEditedAt", NOW),
            ("graph", "editor", {"__typename": "User"}),
            ("graph", "body", "Another comment"),
            ("graph", "id", "IC_other"),
            ("graph", "fullDatabaseId", str(COMMENT_ID + 1)),
            ("graph", "repository", graph_repository(UPSTREAM)),
            (
                "graph",
                "pullRequest",
                {"number": SOURCE.number + 1, "repository": graph_repository(SOURCE)},
            ),
        ):
            with self.subTest(target=target, field=field):
                fixture = ObservationFixture()
                getattr(fixture, target)[field] = value
                self.assert_unrecognized(fixture)

    def test_comment_pages_and_identities_fail_closed(self) -> None:
        fixture = ObservationFixture()
        fixture.comments.append(receipt())
        with self.assertRaises(PromotionReadError):
            fixture.observe()
        fixture = ObservationFixture()
        fixture.pages = {1: [None]}
        with self.assertRaises(PromotionReadError):
            fixture.observe()
        fixture = ObservationFixture()
        fixture.pages = {1: [receipt()] * 101}
        with self.assertRaises(PromotionReadError):
            fixture.observe()
        fixture = ObservationFixture()
        fixture.pages = {
            page: [
                {**receipt(), "id": COMMENT_ID + page * 100 + offset}
                for offset in range(100)
            ]
            for page in range(1, 11)
        }
        with self.assertRaises(PromotionReadError):
            fixture.observe()
        self.assertEqual(
            sum("/comments?" in operation for operation in fixture.trace), 10
        )

    def test_source_and_destination_must_match_the_exact_direct_revision(self) -> None:
        for target, field, value in (
            ("source", "state", "open"),
            ("source", "number", SOURCE.number + 1),
            ("source", "merged_at", NOW),
            ("upstream", "number", UPSTREAM.number + 1),
            ("upstream", "state", "closed"),
            ("upstream", "user", {"login": "zanieb", "id": 1, "type": "User"}),
        ):
            with self.subTest(target=target, field=field):
                fixture = ObservationFixture()
                getattr(fixture, target)[field] = value
                self.assert_unrecognized(fixture)
        for target, revision, field, value in (
            ("source", "base", "repo", repository(UPSTREAM)),
            (
                "source",
                "base",
                "repo",
                {**repository(SOURCE), "id": SOURCE.repository.database_id + 1},
            ),
            ("source", "head", "repo", repository(UPSTREAM)),
            ("source", "head", "repo", None),
            ("source", "head", "sha", str(OTHER)),
            ("source", "head", "ref", "other"),
            ("upstream", "base", "repo", repository(SOURCE)),
            (
                "upstream",
                "base",
                "repo",
                {**repository(UPSTREAM), "id": UPSTREAM.repository.database_id + 1},
            ),
            (
                "upstream",
                "head",
                "repo",
                {**repository(UPSTREAM), "full_name": str(SOURCE.repository.name)},
            ),
            ("upstream", "head", "repo", repository(SOURCE)),
            ("upstream", "head", "repo", None),
            ("upstream", "base", "ref", "other"),
            ("upstream", "head", "ref", "other"),
            ("upstream", "head", "sha", str(OTHER)),
        ):
            with self.subTest(target=target, revision=revision, field=field):
                fixture = ObservationFixture()
                as_object(getattr(fixture, target)[revision])[field] = value
                self.assert_unrecognized(fixture)
        fixture = ObservationFixture()
        self.assertIsNone(fixture.observe(PromotionRequest(SOURCE, OTHER)))
        fixture = ObservationFixture()
        fixture.source.update(merged_at=NOW, merge_commit_sha=str(OTHER))
        self.assertIsNone(fixture.observe())

    def test_final_readback_rejects_source_and_upstream_identity_changes(self) -> None:
        for target, revision, field, value in (
            ("source", "head", "sha", str(OTHER)),
            ("source", "base", "ref", "other"),
            ("upstream", "head", "sha", str(OTHER)),
            ("upstream", "base", "ref", "other"),
            ("upstream", "head", "repo", repository(SOURCE)),
        ):
            with self.subTest(target=target, revision=revision, field=field):
                fixture = ObservationFixture()
                changed = copy.deepcopy(getattr(fixture, target))
                as_object(changed[revision])[field] = value
                setattr(fixture, f"{target}_after", changed)
                self.assert_unrecognized(fixture)
        for target, update in (
            ("source", {"state": "open"}),
            ("source", {"merged_at": NOW, "merge_commit_sha": str(OTHER)}),
            ("upstream", {"state": "closed"}),
            (
                "upstream",
                {"state": "closed", "merged_at": NOW, "merge_commit_sha": str(OTHER)},
            ),
            ("upstream", {"user": {**bot(), "id": AUTOMATIONS_BOT_ID + 1}}),
        ):
            with self.subTest(target=target, update=update):
                fixture = ObservationFixture()
                setattr(
                    fixture, f"{target}_after", {**getattr(fixture, target), **update}
                )
                self.assert_unrecognized(fixture)
        fixture = ObservationFixture()
        fixture.upstream.update(
            state="closed", merged_at=NOW, merge_commit_sha=str(BASE)
        )
        fixture.upstream_after = {
            **fixture.upstream,
            "merge_commit_sha": str(OTHER),
        }
        self.assertIsNone(fixture.observe())

    def test_prepare_reports_the_observed_revision_without_writer_outputs(self) -> None:
        fixture = ObservationFixture()
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            output, summary = root / "output", root / "summary"
            command = cli.parse_command(
                cli.create_parser(),
                [
                    "promotions",
                    "prepare",
                    "--repo",
                    str(SOURCE.repository.name),
                    "--repository-id",
                    str(SOURCE.repository.database_id),
                    "--pull-request",
                    str(SOURCE.number),
                    "--expected-head",
                    str(HEAD),
                    "--approval-id",
                    "",
                    "--github-output",
                    str(output),
                    "--summary",
                    str(summary),
                ],
            )
            with (
                fixture.offline(),
                patch.object(
                    promotions_cli,
                    "plan_promotion",
                    side_effect=AssertionError("Unexpected publication plan"),
                ),
                patch.object(
                    promotions_cli,
                    "plan_queued_promotion",
                    side_effect=AssertionError("Unexpected queued replay plan"),
                ),
            ):
                cli.run(command)
            self.assertEqual(output.read_text(), "action=observed-closed\n")
            self.assertEqual(
                summary.read_text(),
                f"The requested revision `{HEAD}` is already published "
                f"in [astral-sh/uv#{UPSTREAM.number}]"
                f"(https://github.com/astral-sh/uv/pull/{UPSTREAM.number}); "
                f"[astral-sh/uv-dev#{SOURCE.number}]"
                f"(https://github.com/astral-sh/uv-dev/pull/{SOURCE.number}) is closed.\n",
            )

    def test_unrecognized_closed_prepare_keeps_the_existing_stale_result(self) -> None:
        fixture = ObservationFixture()
        fixture.comments = []
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            output, summary = root / "output", root / "summary"
            with fixture.offline():
                cli.run(
                    promotions_cli.PreparePromotion(
                        request=PromotionRequest(SOURCE, HEAD),
                        github_output=output,
                        summary=summary,
                    )
                )
            self.assertEqual(
                output.read_text(),
                f"source_base_ref=main\nsource_base_sha={BASE}\n"
                "head_ref=fixture/promotion\naction=stale\n",
            )
            self.assertEqual(
                summary.read_text(),
                "Promotion skipped: The source is no longer ready.\n",
            )


if __name__ == "__main__":
    unittest.main()
