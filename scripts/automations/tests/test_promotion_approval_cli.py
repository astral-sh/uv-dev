import argparse
import io
import json
import unittest
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass, field
from unittest.mock import patch

from uv_automations import cli
from uv_automations.models import (
    ActorKind,
    CommitSha,
    PullRequestDetails,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    Timestamp,
)
from uv_automations.promotion_approval_cli import (
    InspectPrivatePromotion,
    VerifyPrivateApproval,
    add_private_approval_command,
    add_request_commands,
    parse_command,
    run,
)
from uv_automations.promotion_models import (
    AUTOMATIONS_APP,
    UV_SECURITY_REPOSITORY,
    ConvertedToDraftEvent,
    LabelAddedEvent,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalKind,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    ReadyForReviewEvent,
    RepositoryPermission,
    UneditedPromotionComment,
)
from uv_automations.workflows.promotion_approval import (
    PRIVATE_PROMOTION_LABEL,
    TRUSTED_DISPATCHER,
    ApprovalUnavailableReason,
    InspectedPrivatePromotion,
    PrivateApprovalReference,
    SkippedPrivatePromotionRequest,
    private_approval_receipt_body,
)

SOURCE = PromotionScope(UV_SECURITY_REPOSITORY, 17)
HEAD = CommitSha("a" * 40)
REFERENCE = PrivateApprovalReference(SOURCE, HEAD, 20, 10, 1000)
HUMAN = PromotionActor("reviewer", 123, ActorKind.USER)
READY = ReadyForReviewEvent(10, HUMAN, Timestamp.parse("2026-09-09T12:00:00Z"))
LABEL = LabelAddedEvent(
    20,
    HUMAN,
    Timestamp.parse("2026-09-09T12:01:00Z"),
    PRIVATE_PROMOTION_LABEL,
)
APPROVAL = PromotionApproval(SOURCE, HEAD, LABEL, READY.identifier)


@dataclass
class ReadOnlyApprovalGitHub:
    receipt_present: bool = True
    events: tuple[PromotionEvent, ...] = (READY, LABEL)
    reads: list[str] = field(default_factory=list)

    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        if scope != SOURCE:
            raise AssertionError("Unexpected pull request")
        self.reads.append("pull_request")
        return PromotionPullRequest(
            SOURCE,
            PullRequestDetails(
                SOURCE.reference,
                PullRequestState.OPEN,
                f"https://github.com/{SOURCE.repository.name}/pull/{SOURCE.number}",
                PullRequestRevision(SOURCE.repository, "main", CommitSha("b" * 40)),
                PullRequestRevision(SOURCE.repository, "private-fix", HEAD),
                (PRIVATE_PROMOTION_LABEL,),
            ),
            False,
            HUMAN,
            "Private title",
            "Private body",
            None,
        )

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]:
        if scope != SOURCE:
            raise AssertionError("Unexpected event scope")
        self.reads.append("events")
        return self.events

    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission:
        if repository != SOURCE.repository or actor != HUMAN:
            raise AssertionError("Unexpected permission subject")
        self.reads.append("permission")
        return RepositoryPermission.WRITE

    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None:
        if scope != SOURCE or identifier != REFERENCE.receipt_id:
            raise AssertionError("Unexpected receipt")
        self.reads.append("receipt")
        if not self.receipt_present:
            return None
        timestamp = Timestamp.parse("2026-09-09T12:02:00Z")
        return UneditedPromotionComment(
            PromotionComment(
                SOURCE,
                identifier,
                TRUSTED_DISPATCHER,
                AUTOMATIONS_APP,
                private_approval_receipt_body(APPROVAL),
                timestamp,
                timestamp,
            )
        )


def source_arguments() -> list[str]:
    return [
        "--repo",
        str(SOURCE.repository.name),
        "--repository-id",
        str(SOURCE.repository.database_id),
        "--pull-request",
        str(SOURCE.number),
        "--expected-head",
        str(HEAD),
    ]


def request_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    add_request_commands(parser)
    return parser


def receipt_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    add_private_approval_command(parser)
    return parser


def receipt_arguments() -> list[str]:
    return [
        *source_arguments(),
        "--approval-kind",
        "labeled",
        "--approval-id",
        "20",
        "--ready-event-id",
        "10",
        "--approval-receipt-id",
        "1000",
    ]


class PrivateApprovalCliTests(unittest.TestCase):
    def test_request_parser_registers_only_inspection(self) -> None:
        parser = request_parser()
        self.assertEqual(
            parse_command(
                parser.parse_args(["inspect", *source_arguments(), "--kind", "labeled"])
            ),
            InspectPrivatePromotion(
                source=SOURCE,
                head=HEAD,
                kind=PromotionApprovalKind.LABELED,
                github_output=None,
                summary=None,
            ),
        )
        for command in ("record", "dispatch"):
            with (
                self.subTest(command=command),
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                parser.parse_args([command])

    def test_verification_parser_preserves_the_exact_receipt(self) -> None:
        parser = receipt_parser()
        self.assertEqual(
            parse_command(parser.parse_args(receipt_arguments())),
            VerifyPrivateApproval(
                reference=REFERENCE, github_output=None, summary=None
            ),
        )
        arguments = receipt_arguments()
        arguments[arguments.index("labeled")] = "ready_for_review"
        with self.assertRaisesRegex(ValueError, "requires a label approval"):
            parse_command(parser.parse_args(arguments))

    def test_inspection_uses_only_the_read_token_and_reports_the_dependency(
        self,
    ) -> None:
        command = InspectPrivatePromotion(
            source=SOURCE,
            head=HEAD,
            kind=PromotionApprovalKind.LABELED,
            github_output=None,
            summary=None,
        )
        output = io.StringIO()
        with (
            patch("uv_automations.promotion_approval_cli.PromotionGitHub") as github,
            patch(
                "uv_automations.promotion_approval_cli.inspect_private_promotion",
                return_value=InspectedPrivatePromotion(
                    PromotionApprovalKind.LABELED, 20, 10
                ),
            ) as inspect,
            patch(
                "uv_automations.promotion_approval_cli.verify_private_approval"
            ) as verify,
            redirect_stdout(output),
        ):
            run(command)
        github.assert_called_once_with(token_variable="GH_READ_TOKEN")
        inspect.assert_called_once_with(
            github.return_value, SOURCE, HEAD, PromotionApprovalKind.LABELED
        )
        verify.assert_not_called()
        self.assertEqual(
            json.loads(output.getvalue()),
            {
                "state": "blocked",
                "reason": "dispatcher_event_metadata_unavailable",
                "current_kind": "labeled",
                "current_event_id": "20",
                "current_ready_event_id": "10",
            },
        )

    def test_unavailable_inspection_is_read_only(self) -> None:
        output = io.StringIO()
        with (
            patch("uv_automations.promotion_approval_cli.PromotionGitHub"),
            patch(
                "uv_automations.promotion_approval_cli.inspect_private_promotion",
                return_value=SkippedPrivatePromotionRequest(
                    ApprovalUnavailableReason.HEAD_CHANGED
                ),
            ),
            redirect_stdout(output),
        ):
            run(
                InspectPrivatePromotion(
                    source=SOURCE,
                    head=HEAD,
                    kind=PromotionApprovalKind.LABELED,
                    github_output=None,
                    summary=None,
                )
            )
        self.assertEqual(
            json.loads(output.getvalue()),
            {"state": "unavailable", "reason": "head_changed"},
        )

    def test_verification_never_dispatches(self) -> None:
        output = io.StringIO()
        with (
            patch("uv_automations.promotion_approval_cli.PromotionGitHub") as github,
            patch(
                "uv_automations.promotion_approval_cli.verify_private_approval",
                return_value=APPROVAL,
            ) as verify,
            patch(
                "uv_automations.promotion_approval_cli.inspect_private_promotion"
            ) as inspect,
            redirect_stdout(output),
        ):
            run(
                VerifyPrivateApproval(
                    reference=REFERENCE, github_output=None, summary=None
                )
            )
        github.assert_called_once_with(token_variable="GH_READ_TOKEN")
        verify.assert_called_once_with(github.return_value, REFERENCE)
        inspect.assert_not_called()
        self.assertEqual(
            json.loads(output.getvalue()),
            {
                **REFERENCE.dispatch_inputs(),
                "promoter": "reviewer",
                "promoter_id": "123",
            },
        )

    def test_central_parser_registers_only_read_only_private_commands(self) -> None:
        parser = cli.create_parser()
        self.assertEqual(
            cli.parse_command(
                parser,
                [
                    "promotions",
                    "request",
                    "inspect",
                    *source_arguments(),
                    "--kind",
                    "labeled",
                ],
            ),
            InspectPrivatePromotion(
                source=SOURCE,
                head=HEAD,
                kind=PromotionApprovalKind.LABELED,
                github_output=None,
                summary=None,
            ),
        )
        self.assertEqual(
            cli.parse_command(
                parser, ["promotions", "private-approval", *receipt_arguments()]
            ),
            VerifyPrivateApproval(
                reference=REFERENCE, github_output=None, summary=None
            ),
        )
        for command in ("record", "dispatch"):
            with (
                self.subTest(command=command),
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                cli.parse_command(parser, ["promotions", "request", command])

    def test_central_inspection_executes_policy_without_a_writer(self) -> None:
        reader = ReadOnlyApprovalGitHub()
        output = io.StringIO()
        with (
            patch(
                "uv_automations.promotion_approval_cli.PromotionGitHub",
                return_value=reader,
            ) as github,
            patch("uv_automations.promotions_cli.PromotionQueueGitHub") as writer,
            redirect_stdout(output),
        ):
            cli.main(
                [
                    "promotions",
                    "request",
                    "inspect",
                    *source_arguments(),
                    "--kind",
                    "labeled",
                ]
            )
        github.assert_called_once_with(token_variable="GH_READ_TOKEN")
        writer.assert_not_called()
        self.assertEqual(reader.reads, ["pull_request", "events", "permission"])
        self.assertEqual(
            json.loads(output.getvalue()),
            {
                "state": "blocked",
                "reason": "dispatcher_event_metadata_unavailable",
                "current_kind": "labeled",
                "current_event_id": "20",
                "current_ready_event_id": "10",
            },
        )

    def test_central_verification_executes_policy_without_a_writer(self) -> None:
        reader = ReadOnlyApprovalGitHub()
        output = io.StringIO()
        with (
            patch(
                "uv_automations.promotion_approval_cli.PromotionGitHub",
                return_value=reader,
            ) as github,
            patch("uv_automations.promotions_cli.PromotionQueueGitHub") as writer,
            redirect_stdout(output),
        ):
            cli.main(["promotions", "private-approval", *receipt_arguments()])
        github.assert_called_once_with(token_variable="GH_READ_TOKEN")
        writer.assert_not_called()
        self.assertEqual(
            reader.reads, ["receipt", "pull_request", "events", "permission"]
        )
        self.assertEqual(
            json.loads(output.getvalue()),
            {
                **REFERENCE.dispatch_inputs(),
                "promoter": HUMAN.login,
                "promoter_id": str(HUMAN.database_id),
            },
        )

    def test_central_verification_fails_closed_without_a_receipt(self) -> None:
        reader = ReadOnlyApprovalGitHub(receipt_present=False)
        output = io.StringIO()
        with (
            patch(
                "uv_automations.promotion_approval_cli.PromotionGitHub",
                return_value=reader,
            ),
            patch("uv_automations.promotions_cli.PromotionQueueGitHub") as writer,
            redirect_stderr(output),
            self.assertRaises(SystemExit) as raised,
        ):
            cli.main(["promotions", "private-approval", *receipt_arguments()])
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(reader.reads, ["receipt"])
        writer.assert_not_called()
        self.assertEqual(
            output.getvalue(),
            "uv-automations: The exact private approval receipt could not be verified\n",
        )

    def test_central_verification_observes_a_newer_draft_transition(self) -> None:
        reader = ReadOnlyApprovalGitHub(
            events=(
                READY,
                LABEL,
                ConvertedToDraftEvent(
                    21, HUMAN, Timestamp.parse("2026-09-09T12:02:00Z")
                ),
            )
        )
        output = io.StringIO()
        with (
            patch(
                "uv_automations.promotion_approval_cli.PromotionGitHub",
                return_value=reader,
            ),
            patch("uv_automations.promotions_cli.PromotionQueueGitHub") as writer,
            redirect_stderr(output),
            self.assertRaises(SystemExit) as raised,
        ):
            cli.main(["promotions", "private-approval", *receipt_arguments()])
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(reader.reads, ["receipt", "pull_request", "events"])
        writer.assert_not_called()
        self.assertEqual(
            output.getvalue(),
            "uv-automations: The private pull request has no current ready-for-review event.\n",
        )
