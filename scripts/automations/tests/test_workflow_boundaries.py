import json
import unittest
from dataclasses import dataclass, field, replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations.github import OpenPullRequestQuery, PullRequestLabelContext
from uv_automations.models import (
    CommitSha,
    Label,
    Mergeability,
    PullRequest,
    PullRequestDetails,
    PullRequestRef,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
)
from uv_automations.workflows.conflicts import (
    PreviousBase,
    RebaseDispatch,
    identify_conflicts,
    parse_dispatch,
    remove_rebase_label,
)
from uv_automations.workflows.labels import (
    LabelApplyOutcome,
    LabelPlan,
    PreparedLabels,
    SkippedLabels,
    apply_labels,
    prepare_labels,
    validate_label_plan,
)

UV = RepositoryIdentity(RepositoryName("astral-sh/uv"), 699532645)
UV_DEV = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
PREVIOUS = CommitSha("c" * 40)


def details(
    repository: RepositoryIdentity = UV_DEV, number: int = 123
) -> PullRequestDetails:
    return PullRequestDetails(
        reference=PullRequestRef(repository.name, number),
        state=PullRequestState.OPEN,
        url=f"https://github.com/{repository.name}/pull/{number}",
        base=PullRequestRevision(repository, "main", BASE),
        head=PullRequestRevision(repository, "feature", HEAD),
        labels=("bot:rebase",),
    )


def summary(
    number: int,
    head_repository: RepositoryName | None,
    *,
    repository: RepositoryIdentity = UV,
) -> PullRequest:
    return PullRequest(
        reference=PullRequestRef(repository.name, number),
        author="astral-automations-bot",
        url=f"https://github.com/{repository.name}/pull/{number}",
        base_ref="main",
        head_ref=f"branch/{number}",
        head_sha=HEAD,
        head_repository=head_repository,
        mergeability=Mergeability.CONFLICTING,
    )


@dataclass
class FakeGitHub:
    pull_request: PullRequestDetails = field(default_factory=details)
    pull_requests: tuple[PullRequest, ...] = ()
    queries: list[OpenPullRequestQuery] = field(default_factory=list)
    added: list[tuple[PullRequestRef, tuple[str, ...]]] = field(default_factory=list)
    removed: list[tuple[PullRequestRef, str]] = field(default_factory=list)

    def list_open_pull_requests(
        self, query: OpenPullRequestQuery
    ) -> tuple[PullRequest, ...]:
        self.queries.append(query)
        return self.pull_requests

    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails:
        if reference != self.pull_request.reference:
            raise AssertionError("Unexpected pull request")
        return self.pull_request

    def get_label_context(self, reference: PullRequestRef) -> PullRequestLabelContext:
        pull_request = self.get_pull_request(reference)
        return PullRequestLabelContext(
            pull_request.head.sha, json.dumps({"number": reference.number})
        )

    def get_pull_request_labels(self, reference: PullRequestRef) -> tuple[str, ...]:
        return self.get_pull_request(reference).labels

    def list_labels(self, repository: RepositoryName) -> tuple[Label, ...]:
        return (Label("bug", "A defect"), Label("bot:rebase", None))

    def add_labels(self, reference: PullRequestRef, labels: tuple[str, ...]) -> None:
        self.added.append((reference, labels))

    def remove_label(self, reference: PullRequestRef, label: str) -> None:
        self.removed.append((reference, label))
        self.pull_request = replace(
            self.pull_request,
            labels=tuple(name for name in self.pull_request.labels if name != label),
        )


class RebaseBoundaryTests(unittest.TestCase):
    def test_scan_preserves_head_repository_boundary(self) -> None:
        github = FakeGitHub(
            pull_requests=(
                summary(1, UV.name),
                summary(2, UV_DEV.name),
                summary(3, RepositoryName("contributor/uv")),
                summary(4, None),
            )
        )
        plan = identify_conflicts(github, UV, None)
        self.assertEqual(
            github.queries,
            [OpenPullRequestQuery(UV.name, "main", "app/astral-automations-bot")],
        )
        self.assertEqual(len(plan.found), 4)
        self.assertEqual(
            plan.matrix(),
            {
                "include": [
                    {
                        "number": number,
                        "base_ref": "main",
                        "base_previous_sha": "",
                        "head_ref": f"branch/{number}",
                        "head_sha": str(HEAD),
                        "head_repository": str(repository),
                    }
                    for number, repository in [(1, UV.name), (2, UV_DEV.name)]
                ]
            },
        )
        self.assertEqual(
            plan.summary(),
            "## Pull requests to rebase\n\nFound 4 pull requests to rebase.\n\n"
            "Skipping 2 pull requests whose heads cannot be updated by the rebase app token.\n\n"
            "- https://github.com/astral-sh/uv/pull/1\n"
            "- https://github.com/astral-sh/uv/pull/2\n"
            "- https://github.com/astral-sh/uv/pull/3\n"
            "- https://github.com/astral-sh/uv/pull/4\n",
        )

    def test_uv_dev_scan_has_no_author_filter(self) -> None:
        github = FakeGitHub(
            pull_requests=(
                summary(1, UV_DEV.name, repository=UV_DEV),
                summary(2, UV.name, repository=UV_DEV),
            )
        )
        plan = identify_conflicts(github, UV_DEV, None)
        self.assertEqual(github.queries, [OpenPullRequestQuery(UV_DEV.name, "main")])
        self.assertEqual([item.reference.number for item in plan.rebasable], [1])

    def test_dispatch_checks_repository_ids_and_head(self) -> None:
        original = details()
        wrong_id = RepositoryIdentity(UV_DEV.name, 1)
        invalid = [
            replace(original, state=PullRequestState.CLOSED),
            replace(original, base=replace(original.base, repository=wrong_id)),
            replace(original, head=replace(original.head, repository=wrong_id)),
            replace(original, head=replace(original.head, repository=UV)),
            replace(original, head=replace(original.head, repository=None)),
            replace(original, head=replace(original.head, sha=BASE)),
        ]
        dispatch = RebaseDispatch(original.reference, HEAD)
        for pull_request in invalid:
            with self.subTest(pull_request=pull_request), self.assertRaises(ValueError):
                identify_conflicts(FakeGitHub(pull_request), UV_DEV, dispatch)
        plan = identify_conflicts(FakeGitHub(original), UV_DEV, dispatch)
        self.assertEqual(plan.matrix()["include"][0]["head_sha"], str(HEAD))

    def test_previous_base(self) -> None:
        for previous_ref, expected in [("main", ""), ("parent/123", str(PREVIOUS))]:
            with self.subTest(previous_ref=previous_ref):
                dispatch = RebaseDispatch(
                    details().reference, HEAD, PreviousBase(previous_ref, PREVIOUS)
                )
                plan = identify_conflicts(FakeGitHub(), UV_DEV, dispatch)
                self.assertEqual(
                    plan.matrix()["include"][0]["base_previous_sha"], expected
                )
        for repository, previous_ref in [(UV, "parent/123"), (UV_DEV, "../bad")]:
            with self.subTest(repository=repository, previous_ref=previous_ref):
                dispatch = RebaseDispatch(
                    details(repository).reference,
                    HEAD,
                    PreviousBase(previous_ref, PREVIOUS),
                )
                with self.assertRaises(ValueError):
                    identify_conflicts(
                        FakeGitHub(details(repository)), repository, dispatch
                    )

    def test_incomplete_dispatch(self) -> None:
        self.assertIsNone(parse_dispatch(UV.name, None, None, None, None))
        for number, head, previous_ref, previous_sha in [
            (None, HEAD, None, None),
            (1, None, None, None),
            (1, HEAD, "parent", None),
            (1, HEAD, None, PREVIOUS),
        ]:
            with (
                self.subTest(number=number, previous_ref=previous_ref),
                self.assertRaises(ValueError),
            ):
                parse_dispatch(UV.name, number, head, previous_ref, previous_sha)

    def test_matrix_limit(self) -> None:
        github = FakeGitHub(
            pull_requests=tuple(summary(number, UV.name) for number in range(1, 258))
        )
        with self.assertRaises(ValueError):
            identify_conflicts(github, UV, None)

    def test_rebase_label_cleanup_is_idempotent(self) -> None:
        github = FakeGitHub()
        reference = github.pull_request.reference
        remove_rebase_label(github, reference)
        remove_rebase_label(github, reference)
        self.assertEqual(github.removed, [(reference, "bot:rebase")])


class LabelBoundaryTests(unittest.TestCase):
    def test_prepare_replaces_untrusted_symlinks(self) -> None:
        github = FakeGitHub()
        with TemporaryDirectory() as directory:
            checkout = Path(directory) / "checkout"
            checkout.mkdir()
            target = Path(directory) / "untouched"
            target.write_text("original", encoding="utf-8")
            for name in [
                ".pull-request-labels-event.json",
                ".pull-request-labels.json",
            ]:
                (checkout / name).symlink_to(target)
            with patch("uv_automations.workflows.labels.git.head", return_value=HEAD):
                result = prepare_labels(
                    github,
                    github.pull_request.reference,
                    checkout,
                    {"bug"},
                    expected_head=HEAD,
                )
            self.assertEqual(result, PreparedLabels(HEAD))
            self.assertEqual(target.read_text(), "original")
            self.assertEqual(
                json.loads((checkout / ".pull-request-labels.json").read_text()),
                [{"name": "bug", "description": "A defect"}],
            )
            self.assertFalse(
                (checkout / ".pull-request-labels-event.json").is_symlink()
            )

    def test_prepare_skips_changed_head(self) -> None:
        github = FakeGitHub()
        with TemporaryDirectory() as directory:
            checkout = Path(directory)
            with patch("uv_automations.workflows.labels.git.head", return_value=BASE):
                result = prepare_labels(
                    github,
                    github.pull_request.reference,
                    checkout,
                    {"bug"},
                    expected_head=None,
                )
            self.assertIsInstance(result, SkippedLabels)
            self.assertEqual(list(checkout.iterdir()), [])

    def test_apply_checks_head_and_adds_without_replacing(self) -> None:
        github = FakeGitHub()
        reference = github.pull_request.reference
        plan = validate_label_plan(["bug"], {"bug"})
        self.assertEqual(
            apply_labels(github, reference, plan, expected_head=HEAD),
            LabelApplyOutcome.APPLIED,
        )
        self.assertEqual(github.added, [(reference, ("bug",))])

    def test_apply_skips_stale_closed_and_empty(self) -> None:
        original = details()
        for pull_request, plan, expected in [
            (
                replace(original, state=PullRequestState.CLOSED),
                LabelPlan(("bug",)),
                LabelApplyOutcome.STALE,
            ),
            (
                replace(original, head=replace(original.head, sha=BASE)),
                LabelPlan(("bug",)),
                LabelApplyOutcome.STALE,
            ),
            (original, LabelPlan(()), LabelApplyOutcome.EMPTY),
        ]:
            with self.subTest(expected=expected):
                github = FakeGitHub(pull_request)
                self.assertEqual(
                    apply_labels(github, original.reference, plan, expected_head=HEAD),
                    expected,
                )
                self.assertEqual(github.added, [])

    def test_apply_is_restricted_to_uv_dev(self) -> None:
        github = FakeGitHub(details(UV))
        with self.assertRaises(ValueError):
            apply_labels(
                github,
                github.pull_request.reference,
                LabelPlan(("bug",)),
                expected_head=HEAD,
            )
        self.assertEqual(github.added, [])
