import io
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from uv_automations import cli, promotions_cli
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.models import ActorKind, CommitSha, Timestamp
from uv_automations.promotion_models import (
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    ConvertedToDraftEvent,
    PromotionActor,
    PromotionScope,
    ReadyForReviewEvent,
)
from uv_automations.workflows.promotion import PromotionRequest

HEAD = CommitSha("a" * 40)
SOURCE = PromotionScope(UV_DEV_REPOSITORY, 20)
TIME = Timestamp.parse("2026-09-09T12:00:00Z")
HUMAN = PromotionActor("zanieb", 101, ActorKind.USER)
BOT = PromotionActor("astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT)
READY = ReadyForReviewEvent(1000, HUMAN, TIME)
REVOKED = (
    READY,
    ConvertedToDraftEvent(1001, HUMAN, TIME),
    ReadyForReviewEvent(1002, BOT, TIME),
)


class PromotionCliTests(unittest.TestCase):
    def test_current_approval_parses_an_exact_optional_replay_event(self) -> None:
        arguments = [
            "promotions",
            "current-approval",
            "--repo",
            str(UV_DEV_REPOSITORY.name),
            "--repository-id",
            str(UV_DEV_REPOSITORY.database_id),
            "--pull-request",
            str(SOURCE.number),
            "--expected-head",
            str(HEAD),
            "--approval-id",
            "1000",
        ]
        self.assertEqual(
            cli.parse_command(cli.create_parser(), arguments),
            promotions_cli.ReadPromotionApproval(
                request=PromotionRequest(SOURCE, HEAD, 1000)
            ),
        )

    def test_explicit_replay_cannot_recover_a_withdrawn_approval(self) -> None:
        for approval_id, expected in ((None, "1000\n"), (1000, "")):
            with self.subTest(approval_id=approval_id):
                output = io.StringIO()
                command = promotions_cli.ReadPromotionApproval(
                    request=PromotionRequest(SOURCE, HEAD, approval_id)
                )
                with (
                    patch.object(
                        PromotionGitHub, "list_promotion_events", return_value=REVOKED
                    ),
                    redirect_stdout(output),
                ):
                    cli.run(command)
                self.assertEqual(output.getvalue(), expected)

    def test_prepare_uses_the_durable_queue_route_only_for_explicit_replays(
        self,
    ) -> None:
        for approval_id in (None, 1000):
            with self.subTest(approval_id=approval_id):
                request = PromotionRequest(SOURCE, HEAD, approval_id)
                command = promotions_cli.PreparePromotion(
                    request=request,
                    github_output=Path("output"),
                    summary=Path("summary"),
                )
                with (
                    patch.object(promotions_cli, "plan_promotion") as ordinary,
                    patch.object(promotions_cli, "plan_queued_promotion") as replay,
                    patch.object(promotions_cli, "_write_plan") as write,
                ):
                    cli.run(command)
                selected, skipped = (
                    (ordinary, replay) if approval_id is None else (replay, ordinary)
                )
                selected.assert_called_once()
                self.assertEqual(selected.call_args.args[1], request)
                skipped.assert_not_called()
                write.assert_called_once_with(
                    selected.return_value, command.github_output, command.summary
                )

    def test_a_new_ready_event_does_not_upgrade_an_old_replay(self) -> None:
        output = io.StringIO()
        command = promotions_cli.ReadPromotionApproval(
            request=PromotionRequest(SOURCE, HEAD, 1000)
        )
        with (
            patch.object(
                PromotionGitHub,
                "list_promotion_events",
                return_value=(*REVOKED, ReadyForReviewEvent(1003, HUMAN, TIME)),
            ),
            redirect_stdout(output),
        ):
            cli.run(command)
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
