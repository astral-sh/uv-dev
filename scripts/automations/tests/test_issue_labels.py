import json
import subprocess
import sys
import unittest
from dataclasses import dataclass, field, replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations.github import ISSUE_LABEL_FIELDS, GitHub, decode_issue_details
from uv_automations.models import (
    Issue,
    IssueDetails,
    IssueRef,
    IssueState,
    Label,
    RepositoryName,
)
from uv_automations.workflows.issue_labels import (
    IssueRevision,
    IssueType,
    PreparedIssueLabels,
    apply_issue_labels,
    plan_issue_labels,
    prepare_issue_labels,
    triage_type,
)
from uv_automations.workflows.labels import (
    MAX_LABELS,
    LabelApplyOutcome,
    LabelPlan,
    LabelRecommendation,
    SkippedLabels,
)

REPOSITORY = Path(__file__).resolve().parents[3]
REFERENCE = IssueRef(RepositoryName("astral-sh/uv"), 123)
ISSUE = Issue(REFERENCE, "Resolver error", "A report", None)
DETAILS = IssueDetails(ISSUE, IssueState.OPEN, ())
ALLOWED = frozenset({*IssueType, "area:resolver", "area:windows", "performance"})


@dataclass
class FakeGitHub:
    details: IssueDetails = DETAILS
    available: frozenset[str] = ALLOWED
    reads: list[IssueRef] = field(default_factory=list)
    added: list[tuple[IssueRef, tuple[str, ...]]] = field(default_factory=list)

    def get_issue_details(self, reference: IssueRef) -> IssueDetails:
        self.reads.append(reference)
        return self.details

    def list_labels(self, repository: RepositoryName) -> tuple[Label, ...]:
        return tuple(Label(name, None) for name in sorted(self.available))

    def add_labels(self, reference: IssueRef, labels: tuple[str, ...]) -> None:
        self.added.append((reference, labels))


def issue_payload() -> dict[str, object]:
    return {
        **ISSUE.to_payload(),
        "state": "OPEN",
        "labels": [{"name": "area:resolver"}],
    }


class IssueLabelTests(unittest.TestCase):
    def test_shared_issue_decoder_and_transport(self) -> None:
        expected = replace(DETAILS, labels=("area:resolver",))
        self.assertEqual(decode_issue_details(issue_payload(), REFERENCE), expected)
        with patch("uv_automations.github.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess(
                [], 0, json.dumps(issue_payload())
            )
            self.assertEqual(GitHub().get_issue_details(REFERENCE), expected)
            run.assert_called_once_with(
                [
                    "gh",
                    "issue",
                    "view",
                    "123",
                    "--repo",
                    "astral-sh/uv",
                    "--json",
                    ISSUE_LABEL_FIELDS,
                ],
                input=None,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                env=None,
                timeout=60,
            )

    def test_issue_identity_rejects_pull_requests(self) -> None:
        for changed in [
            {"number": 456},
            {"url": "https://github.com/astral-sh/uv-dev/issues/123"},
            {"url": "https://github.com/astral-sh/uv/pull/123"},
            {"state": "unknown"},
        ]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                decode_issue_details({**issue_payload(), **changed}, REFERENCE)

    def test_schema_and_issue_type_contracts(self) -> None:
        schema = json.loads(
            (REPOSITORY / "agents/schemas/issue-labels.json").read_text()
        )
        self.assertEqual(set(schema["required"]), {"labels", "summary"})
        self.assertEqual(set(schema["properties"]), {"labels", "summary"})
        self.assertIs(schema["additionalProperties"], False)
        self.assertEqual(schema["properties"]["labels"]["maxItems"], MAX_LABELS)
        triage = json.loads(
            (REPOSITORY / "agents/schemas/issue-triage.json").read_text()
        )
        self.assertEqual(set(triage["properties"]["type"]["enum"]), set(IssueType))
        self.assertIsNone(triage_type(None))
        self.assertEqual(
            triage_type({"type": "bug", "subtype": "regression"}), IssueType.BUG
        )
        with self.assertRaises(ValueError):
            triage_type({"type": "bot:rebase"})

    def test_issue_policy_is_semantic(self) -> None:
        allowed = json.loads(
            (REPOSITORY / ".github/allowed-issue-labels.json").read_text()
        )
        self.assertEqual(allowed, sorted(set(allowed)))
        self.assertTrue(set(IssueType).issubset(allowed))
        forbidden = {
            "codex",
            "do-not-merge",
            "good first issue",
            "help wanted",
            "needs-decision",
            "needs-design",
            "needs-mre",
            "wish",
            "wontfix",
        }
        self.assertFalse(set(allowed).intersection(forbidden))
        self.assertFalse(
            any(label.startswith(("bot:", "build:", "test:")) for label in allowed)
        )

    def test_prepare_reuses_triage_and_filters_label_catalog(self) -> None:
        github = FakeGitHub(
            details=replace(DETAILS, labels=("question",)),
            available=ALLOWED | {"bot:rebase"},
        )
        triage = {"type": "bug", "summary": "A confirmed defect"}
        with TemporaryDirectory() as directory:
            checkout = Path(directory)
            prepared = prepare_issue_labels(
                github, REFERENCE, checkout, ALLOWED, triage=triage, triaged_issue=ISSUE
            )
            self.assertEqual(
                prepared,
                PreparedIssueLabels(
                    IssueRevision.for_issue(ISSUE), ("question",), IssueType.BUG
                ),
            )
            self.assertEqual(
                json.loads((checkout / ".issue-labels-event.json").read_text()),
                {**ISSUE.to_payload(), "labels": ["question"]},
            )
            self.assertEqual(
                json.loads((checkout / ".issue-labels-triage.json").read_text()), triage
            )
            catalog = json.loads((checkout / ".issue-labels.json").read_text())
            self.assertEqual({item["name"] for item in catalog}, ALLOWED)

    def test_prepare_skips_closed_and_refuses_existing_context(self) -> None:
        with TemporaryDirectory() as directory:
            checkout = Path(directory)
            github = FakeGitHub(details=replace(DETAILS, state=IssueState.CLOSED))
            self.assertEqual(
                prepare_issue_labels(
                    github,
                    REFERENCE,
                    checkout,
                    ALLOWED,
                    triage=None,
                    triaged_issue=None,
                ),
                SkippedLabels("The issue is closed"),
            )
            self.assertEqual(list(checkout.iterdir()), [])
            target = checkout / "target"
            target.write_text("unchanged")
            (checkout / ".issue-labels-event.json").symlink_to(target)
            with self.assertRaises(FileExistsError):
                prepare_issue_labels(
                    FakeGitHub(),
                    REFERENCE,
                    checkout,
                    ALLOWED,
                    triage=None,
                    triaged_issue=None,
                )
            self.assertEqual(target.read_text(), "unchanged")

    def test_prepare_rejects_stale_or_unpaired_triage(self) -> None:
        with TemporaryDirectory() as directory:
            checkout = Path(directory)
            github = FakeGitHub(
                details=replace(DETAILS, issue=replace(ISSUE, body="Edited"))
            )
            self.assertEqual(
                prepare_issue_labels(
                    github,
                    REFERENCE,
                    checkout,
                    ALLOWED,
                    triage={"type": "bug"},
                    triaged_issue=ISSUE,
                ),
                SkippedLabels("The issue changed after triage"),
            )
            self.assertEqual(list(checkout.iterdir()), [])
            for triage, issue in [({"type": "bug"}, None), (None, ISSUE)]:
                with self.subTest(triage=triage), self.assertRaises(ValueError):
                    prepare_issue_labels(
                        github,
                        REFERENCE,
                        checkout,
                        ALLOWED,
                        triage=triage,
                        triaged_issue=issue,
                    )

    def test_primary_classification_and_existing_labels(self) -> None:
        cases = [
            (("area:resolver",), (), IssueType.BUG, ("bug", "area:resolver")),
            (
                ("bug", "area:resolver"),
                ("question",),
                IssueType.BUG,
                ("area:resolver",),
            ),
            (("bug", "area:resolver"), ("bug", "area:resolver"), IssueType.BUG, ()),
            (("question",), (), None, ("question",)),
            (("bug",), ("enhancement",), None, ()),
        ]
        for labels, existing, issue_type, expected in cases:
            with self.subTest(labels=labels, existing=existing, issue_type=issue_type):
                self.assertEqual(
                    plan_issue_labels(
                        LabelRecommendation(labels, ""),
                        ALLOWED,
                        existing=existing,
                        issue_type=issue_type,
                    ),
                    LabelPlan(expected),
                )

    def test_rejects_conflicting_duplicate_and_disallowed_labels(self) -> None:
        cases = [
            (("question",), IssueType.BUG),
            (("bug", "enhancement"), None),
            (("bug", "bug"), None),
            (("bot:rebase",), None),
            (("area:resolver", "area:windows", "performance"), IssueType.BUG),
        ]
        for labels, issue_type in cases:
            with self.subTest(labels=labels), self.assertRaises(ValueError):
                plan_issue_labels(
                    LabelRecommendation(labels, ""),
                    ALLOWED,
                    existing=(),
                    issue_type=issue_type,
                )

    def test_apply_is_additive_and_preserves_new_maintainer_classification(
        self,
    ) -> None:
        github = FakeGitHub(
            details=replace(DETAILS, labels=("question", "area:windows"))
        )
        outcome = apply_issue_labels(
            github,
            REFERENCE,
            LabelPlan(("bug", "area:resolver", "area:windows")),
            ALLOWED,
            expected_revision=IssueRevision.for_issue(ISSUE),
            issue_type=IssueType.BUG,
        )
        self.assertEqual(outcome, LabelApplyOutcome.APPLIED)
        self.assertEqual(github.added, [(REFERENCE, ("area:resolver",))])

    def test_apply_skips_stale_and_closed_issues(self) -> None:
        for details in [
            replace(DETAILS, state=IssueState.CLOSED),
            replace(DETAILS, issue=replace(ISSUE, body="Edited")),
        ]:
            with self.subTest(details=details):
                github = FakeGitHub(details=details)
                self.assertEqual(
                    apply_issue_labels(
                        github,
                        REFERENCE,
                        LabelPlan(("bug",)),
                        ALLOWED,
                        expected_revision=IssueRevision.for_issue(ISSUE),
                        issue_type=IssueType.BUG,
                    ),
                    LabelApplyOutcome.STALE,
                )
                self.assertEqual(github.added, [])

    def test_apply_revalidates_policy_and_repository(self) -> None:
        for reference, plan, available in [
            (IssueRef(RepositoryName("other/repo"), 123), LabelPlan(("bug",)), ALLOWED),
            (REFERENCE, LabelPlan(("bot:rebase",)), ALLOWED),
            (REFERENCE, LabelPlan(("question",)), ALLOWED),
            (REFERENCE, LabelPlan(("area:resolver",)), ALLOWED - {"area:resolver"}),
        ]:
            with self.subTest(reference=reference, plan=plan):
                github = FakeGitHub(available=available)
                with self.assertRaises(ValueError):
                    apply_issue_labels(
                        github,
                        reference,
                        plan,
                        ALLOWED,
                        expected_revision=IssueRevision.for_issue(ISSUE),
                        issue_type=IssueType.BUG,
                    )
                self.assertEqual(github.added, [])

    def test_apply_is_idempotent(self) -> None:
        github = FakeGitHub(details=replace(DETAILS, labels=("bug", "area:resolver")))
        self.assertEqual(
            apply_issue_labels(
                github,
                REFERENCE,
                LabelPlan(("bug", "area:resolver")),
                ALLOWED,
                expected_revision=IssueRevision.for_issue(ISSUE),
                issue_type=IssueType.BUG,
            ),
            LabelApplyOutcome.EMPTY,
        )
        self.assertEqual(github.added, [])

    def test_cli_validation(self) -> None:
        with TemporaryDirectory() as directory:
            allowed = Path(directory) / "allowed.json"
            allowed.write_text(json.dumps(sorted(ALLOWED)))
            output = Path(directory) / "github-output"
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "uv_automations",
                    "issue-labels",
                    "validate",
                    "--allowed",
                    str(allowed),
                    "--existing",
                    "[]",
                    "--issue-type",
                    "bug",
                    "--github-output",
                    str(output),
                ],
                input='{"labels":["area:resolver"],"summary":"Reason"}',
                text=True,
                capture_output=True,
                check=True,
            )
            self.assertEqual(result.stdout, "")
            self.assertEqual(result.stderr, "")
            self.assertEqual(output.read_text(), 'labels=["bug","area:resolver"]\n')

    def test_sts_policy_binds_caller_and_callee(self) -> None:
        policy = json.loads((REPOSITORY / ".github/ost-simple-sts.json").read_text())
        rules = [
            rule
            for rule in policy["rules"]
            if rule.get("caller_workflow") == "issue-labels.yml"
            or rule.get("reusable_workflow") == "issue-labels.yml"
        ]
        self.assertEqual(
            rules,
            [
                {
                    "caller": "uv",
                    "environment": "automations",
                    "caller_workflow": "issue-triage.yml",
                    "reusable_workflow": "issue-labels.yml",
                    "on": ["issues", "workflow_dispatch"],
                    "permissions": {"issues": "write"},
                    "target": "uv",
                },
                {
                    "caller": "uv",
                    "environment": "automations",
                    "caller_workflow": "issue-labels.yml",
                    "on": ["workflow_dispatch"],
                    "permissions": {"issues": "write"},
                    "target": "uv",
                },
            ],
        )
