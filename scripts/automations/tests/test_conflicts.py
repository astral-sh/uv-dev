import json
import subprocess
import unittest
from dataclasses import dataclass, field, replace
from unittest.mock import patch

from uv_automations.github import (
    PULL_REQUEST_FIELDS,
    GitHub,
    OpenPullRequestQuery,
    decode_pull_request,
)
from uv_automations.models import (
    CommitSha,
    Mergeability,
    PullRequest,
    PullRequestRef,
    RepositoryName,
)
from uv_automations.workflows.conflicts import (
    MergeabilitySummary,
    conflict_payload,
    find_conflicted_pull_requests,
    summarize_mergeability,
)

REPOSITORY = RepositoryName("astral-sh/uv")
HEAD_REPOSITORY = RepositoryName("astral-sh/uv-dev")
HEAD_SHA = CommitSha("a" * 40)


def pull_request(number: int, mergeability: Mergeability) -> PullRequest:
    return PullRequest(
        reference=PullRequestRef(REPOSITORY, number),
        author="astral-automations-bot",
        url=f"https://github.com/astral-sh/uv/pull/{number}",
        base_ref="main",
        head_ref="some-branch",
        head_sha=HEAD_SHA,
        head_repository=HEAD_REPOSITORY,
        mergeability=mergeability,
    )


@dataclass
class FakeGitHub:
    responses: list[tuple[PullRequest, ...]]
    calls: list[OpenPullRequestQuery] = field(default_factory=list)

    def list_open_pull_requests(
        self, query: OpenPullRequestQuery
    ) -> tuple[PullRequest, ...]:
        self.calls.append(query)
        return self.responses.pop(0)


def github_payload(
    *, mergeability: str = "CONFLICTING", deleted: bool = False, base: str = "main"
) -> dict[str, object]:
    return {
        "number": 123,
        "author": None if deleted else {"login": "astral-automations-bot"},
        "url": "https://github.com/astral-sh/uv/pull/123",
        "baseRefName": base,
        "headRefName": "some-branch",
        "headRefOid": str(HEAD_SHA),
        "headRepository": (
            None if deleted else {"nameWithOwner": str(HEAD_REPOSITORY)}
        ),
        "mergeable": mergeability,
    }


class ConflictTests(unittest.TestCase):
    def test_summarize_mergeability(self) -> None:
        conflicting = pull_request(1, Mergeability.CONFLICTING)
        self.assertEqual(
            summarize_mergeability(
                [
                    conflicting,
                    pull_request(2, Mergeability.MERGEABLE),
                    pull_request(3, Mergeability.UNKNOWN),
                ]
            ),
            MergeabilitySummary((conflicting,), pending=1),
        )

    def test_waits_for_mergeability(self) -> None:
        unknown = pull_request(1, Mergeability.UNKNOWN)
        conflicting = replace(unknown, mergeability=Mergeability.CONFLICTING)
        mergeable = pull_request(2, Mergeability.MERGEABLE)
        github = FakeGitHub([(unknown, mergeable), (conflicting, mergeable)])
        sleeps: list[float] = []
        self.assertEqual(
            find_conflicted_pull_requests(
                github,
                REPOSITORY,
                author="app/astral-automations-bot",
                sleep=sleeps.append,
            ),
            (conflicting,),
        )
        self.assertEqual(
            github.calls,
            [
                OpenPullRequestQuery(
                    REPOSITORY, base="main", author="app/astral-automations-bot"
                )
            ]
            * 2,
        )
        self.assertEqual(sleeps, [5])

    def test_retry_budget(self) -> None:
        conflicting = pull_request(1, Mergeability.CONFLICTING)
        unknown = pull_request(2, Mergeability.UNKNOWN)
        github = FakeGitHub([(conflicting, unknown)] * 5)
        sleeps: list[float] = []
        with self.assertLogs(
            "uv_automations.workflows.conflicts", level="WARNING"
        ) as logs:
            self.assertEqual(
                find_conflicted_pull_requests(github, REPOSITORY, sleep=sleeps.append),
                (conflicting,),
            )
        self.assertEqual(len(github.calls), 5)
        self.assertEqual(sleeps, [5] * 4)
        self.assertEqual(
            [record.getMessage() for record in logs.records],
            ["GitHub could not determine mergeability for 1 pull requests."],
        )

    def test_invalid_retry_policy(self) -> None:
        for max_attempts, retry_delay in [(0, 5), (-1, 5), (1, -1)]:
            with (
                self.subTest(max_attempts=max_attempts, retry_delay=retry_delay),
                self.assertRaises(ValueError),
            ):
                find_conflicted_pull_requests(
                    FakeGitHub([]),
                    REPOSITORY,
                    max_attempts=max_attempts,
                    retry_delay=retry_delay,
                )

    def test_existing_conflict_payload(self) -> None:
        decoded = decode_pull_request(github_payload(), REPOSITORY)
        self.assertEqual(
            conflict_payload(decoded),
            {
                "number": 123,
                "author": "astral-automations-bot",
                "url": "https://github.com/astral-sh/uv/pull/123",
                "base_ref": "main",
                "head_ref": "some-branch",
                "head_sha": str(HEAD_SHA),
                "head_repository": "astral-sh/uv-dev",
            },
        )

    def test_deleted_author_and_head_repository(self) -> None:
        decoded = decode_pull_request(github_payload(deleted=True), REPOSITORY)
        self.assertIsNone(decoded.author)
        self.assertIsNone(decoded.head_repository)
        self.assertIsNone(conflict_payload(decoded)["head_repository"])

    def test_unknown_mergeability_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            decode_pull_request(github_payload(mergeability="NEW_STATE"), REPOSITORY)

    def test_github_cli_transport(self) -> None:
        query = OpenPullRequestQuery(
            REPOSITORY,
            base="release/0.12",
            author="app/astral-automations-bot",
            limit=25,
        )
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess(
                [], 0, json.dumps([github_payload(base=query.base)])
            )
            self.assertEqual(
                GitHub().list_open_pull_requests(query),
                (
                    replace(
                        pull_request(123, Mergeability.CONFLICTING),
                        base_ref=query.base,
                    ),
                ),
            )
            run.assert_called_once_with(
                [
                    "gh",
                    "pr",
                    "list",
                    "--repo",
                    "astral-sh/uv",
                    "--base",
                    "release/0.12",
                    "--state",
                    "open",
                    "--limit",
                    "25",
                    "--json",
                    PULL_REQUEST_FIELDS,
                    "--author",
                    "app/astral-automations-bot",
                ],
                input=None,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                env=None,
                timeout=60,
            )

    def test_invalid_query_limit(self) -> None:
        for limit in [0, -1, True]:
            with self.subTest(limit=limit), self.assertRaises(ValueError):
                OpenPullRequestQuery(REPOSITORY, base="main", limit=limit)
