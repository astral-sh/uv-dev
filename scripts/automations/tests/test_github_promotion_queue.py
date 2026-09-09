import unittest
from dataclasses import dataclass, field
from unittest.mock import patch

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.github_promotion import PromotionReadError
from uv_automations.github_promotion_queue import PromotionQueueGitHub
from uv_automations.models import ActorKind, CommitSha, RepositoryIdentity, Timestamp
from uv_automations.promotion_models import (
    AUTOMATIONS_APP,
    AUTOMATIONS_BOT_ID,
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    CommitComparison,
    ComparisonStatus,
    PromotionActor,
    PromotionApproval,
    PromotionComment,
    PromotionScope,
    ReadyForReviewEvent,
    UneditedPromotionComment,
)
from uv_automations.workflows.promotion_queue import (
    PendingSourceParent,
    QueuedPromotion,
)

OLD = CommitSha("a" * 40)
MAIN = CommitSha("b" * 40)
OTHER = CommitSha("c" * 40)
TIME = Timestamp.parse("2026-09-09T12:00:00Z")


@dataclass
class PublicHistory:
    main: CommitSha | None = MAIN
    ancestors: set[tuple[CommitSha, CommitSha]] = field(
        default_factory=lambda: {(OLD, MAIN)}
    )

    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        if repository != UV_REPOSITORY or ref != "main":
            raise AssertionError("Unexpected public revision query")
        return self.main

    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        if repository != UV_REPOSITORY:
            raise AssertionError("Unexpected public history query")
        if base == head:
            return CommitComparison(
                repository, base, head, ComparisonStatus.IDENTICAL, base
            )
        if (base, head) in self.ancestors:
            return CommitComparison(
                repository, base, head, ComparisonStatus.AHEAD, base
            )
        return CommitComparison(
            repository, base, head, ComparisonStatus.DIVERGED, CommitSha("0" * 40)
        )


class PromotionSyncTests(unittest.TestCase):
    def test_sync_returns_only_a_verified_public_revision(self) -> None:
        github = PromotionQueueGitHub()
        with (
            patch.object(
                PromotionQueueGitHub, "get_repository", return_value=UV_DEV_REPOSITORY
            ),
            patch.object(PromotionQueueGitHub, "get_ref", side_effect=(OLD, MAIN)),
            patch.object(PromotionQueueGitHub, "_api") as api,
        ):
            self.assertEqual(github.sync_uv_dev_main(PublicHistory()), MAIN)
        api.assert_called_once_with(
            "POST", "repos/astral-sh/uv-dev/merge-upstream", payload={"branch": "main"}
        )

    def test_diverged_or_missing_source_is_rejected_before_any_write(self) -> None:
        for source in (OTHER, None):
            with self.subTest(source=source):
                github = PromotionQueueGitHub()
                with (
                    patch.object(
                        PromotionQueueGitHub,
                        "get_repository",
                        return_value=UV_DEV_REPOSITORY,
                    ),
                    patch.object(PromotionQueueGitHub, "get_ref", return_value=source),
                    patch.object(PromotionQueueGitHub, "_api") as api,
                    self.assertRaisesRegex(ValueError, "not public uv history"),
                ):
                    github.sync_uv_dev_main(PublicHistory())
                api.assert_not_called()

    def test_source_only_merge_result_is_not_exposed_as_synced_main(self) -> None:
        github = PromotionQueueGitHub()
        with (
            patch.object(
                PromotionQueueGitHub, "get_repository", return_value=UV_DEV_REPOSITORY
            ),
            patch.object(PromotionQueueGitHub, "get_ref", side_effect=(OLD, OTHER)),
            patch.object(PromotionQueueGitHub, "_api") as api,
            self.assertRaisesRegex(ValueError, "not public uv history"),
        ):
            github.sync_uv_dev_main(PublicHistory())
        api.assert_called_once()


class PromotionQueueWriterTests(unittest.TestCase):
    def test_new_receipt_uses_the_separate_app_and_edit_proof(self) -> None:
        source = PromotionScope(UV_DEV_REPOSITORY, 20)
        actor = PromotionActor("zanieb", 101, ActorKind.USER)
        event = ReadyForReviewEvent(1000, actor, TIME)
        queued = QueuedPromotion(
            PromotionApproval(source, MAIN, event, event.identifier).claim,
            BranchRevision(UV_DEV_REPOSITORY, "parent", OLD),
            BranchRevision(UV_DEV_REPOSITORY, "child", MAIN),
            PendingSourceParent(PromotionScope(UV_DEV_REPOSITORY, 10)),
        )
        bot = PromotionActor(
            "astral-automations-bot[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT
        )
        proof = UneditedPromotionComment(
            PromotionComment(
                source, 3000, bot, AUTOMATIONS_APP, queued.comment(), TIME, TIME
            )
        )
        payload = {
            "id": 3000,
            "issue_url": "https://api.github.com/repos/astral-sh/uv-dev/issues/20",
            "user": {"login": bot.login, "id": bot.database_id, "type": "Bot"},
            "performed_via_github_app": None,
            "body": queued.comment(),
            "created_at": str(TIME),
            "updated_at": str(TIME),
        }
        with (
            patch.object(PromotionQueueGitHub, "_api", return_value=payload) as api,
            patch.object(
                PromotionQueueGitHub,
                "get_unedited_promotion_comment",
                return_value=proof,
            ) as receipt,
        ):
            PromotionQueueGitHub().create_queue_comment(queued)
        api.assert_called_once_with(
            "POST",
            "repos/astral-sh/uv-dev/issues/20/comments",
            payload={"body": queued.comment()},
        )
        receipt.assert_called_once_with(source, 3000)
        with (
            patch.object(PromotionQueueGitHub, "_api", return_value=payload),
            patch.object(
                PromotionQueueGitHub,
                "get_unedited_promotion_comment",
                return_value=None,
            ),
            self.assertRaisesRegex(ValueError, "record was edited"),
        ):
            PromotionQueueGitHub().create_queue_comment(queued)

    def test_replay_dispatch_is_fixed_to_main_and_exact_original_approval(self) -> None:
        source = PromotionScope(UV_DEV_REPOSITORY, 20)
        event = ReadyForReviewEvent(
            1000, PromotionActor("zanieb", 101, ActorKind.USER), TIME
        )
        approval = PromotionApproval(source, OLD, event, event.identifier).claim
        dispatched = WorkflowDispatch(UV_DEV_REPOSITORY, 4000)
        github = PromotionQueueGitHub()
        with patch.object(
            PromotionQueueGitHub, "dispatch_main_workflow", return_value=dispatched
        ) as dispatch:
            self.assertEqual(github.dispatch_promotion(approval), dispatched)
        dispatch.assert_called_once_with(
            UV_DEV_REPOSITORY,
            "promote-pull-request.yml",
            {"pull_request": "20", "head_sha": str(OLD), "approval_id": "1000"},
        )

    def test_existing_upstream_base_is_never_overwritten(self) -> None:
        github = PromotionQueueGitHub()
        destination = BranchRevision(UV_REPOSITORY, "parent", OLD)
        with (
            patch.object(
                PromotionQueueGitHub, "get_repository", return_value=UV_REPOSITORY
            ),
            patch.object(PromotionQueueGitHub, "get_ref", return_value=OTHER),
            patch.object(PromotionQueueGitHub, "_api") as api,
            self.assertRaisesRegex(ValueError, "changed before creation"),
        ):
            github.create_upstream_base(destination)
        api.assert_not_called()

    def test_existing_exact_upstream_base_is_idempotent(self) -> None:
        github = PromotionQueueGitHub()
        destination = BranchRevision(UV_REPOSITORY, "parent", OLD)
        with (
            patch.object(
                PromotionQueueGitHub, "get_repository", return_value=UV_REPOSITORY
            ),
            patch.object(PromotionQueueGitHub, "get_ref", return_value=OLD),
            patch.object(PromotionQueueGitHub, "_api") as api,
        ):
            github.create_upstream_base(destination)
        api.assert_not_called()

    def test_create_only_base_race_accepts_only_the_same_commit(self) -> None:
        destination = BranchRevision(UV_REPOSITORY, "parent", OLD)
        for observed in (OLD, OTHER):
            with self.subTest(observed=observed):
                with (
                    patch.object(
                        PromotionQueueGitHub,
                        "get_repository",
                        return_value=UV_REPOSITORY,
                    ),
                    patch.object(
                        PromotionQueueGitHub,
                        "get_ref",
                        side_effect=(None, observed, observed),
                    ),
                    patch.object(
                        PromotionQueueGitHub,
                        "_api",
                        side_effect=PromotionReadError(
                            "GitHub promotion request failed"
                        ),
                    ) as api,
                ):
                    if observed == OLD:
                        PromotionQueueGitHub().create_upstream_base(destination)
                    else:
                        with self.assertRaises(PromotionReadError):
                            PromotionQueueGitHub().create_upstream_base(destination)
                api.assert_called_once_with(
                    "POST",
                    "repos/astral-sh/uv/git/refs",
                    payload={"ref": "refs/heads/parent", "sha": str(OLD)},
                )


if __name__ == "__main__":
    unittest.main()
