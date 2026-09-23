"""Narrow writes for promotion queues, fork synchronization, and base copies."""

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.github_promotion import (
    PromotionGitHub,
    PromotionReadError,
    PromotionRevisionReader,
    decode_promotion_comment,
)
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    PromotionApprovalClaim,
    PromotionApprovalKind,
)
from uv_automations.workflows.promotion_queue import (
    QueuedPromotion,
    is_public_main_revision,
)


class PromotionQueueGitHub(PromotionGitHub):
    def create_queue_comment(self, queued: QueuedPromotion) -> None:
        scope = queued.source
        body = queued.comment()
        comment = decode_promotion_comment(
            self._api(
                "POST",
                f"repos/{scope.repository.name}/issues/{scope.number}/comments",
                payload={"body": body},
            ),
            scope,
        )
        if comment.body != body:
            raise ValueError(
                "GitHub did not create the expected promotion queue record"
            )
        original = self.get_unedited_promotion_comment(scope, comment.identifier)
        if original is None or original.comment.body != body:
            raise ValueError("The new promotion queue record was edited")

    def dispatch_promotion(self, approval: PromotionApprovalClaim) -> WorkflowDispatch:
        if (
            approval.source.repository != UV_DEV_REPOSITORY
            or approval.kind != PromotionApprovalKind.READY_FOR_REVIEW
            or approval.ready_event_id != approval.event_id
        ):
            raise ValueError("Unexpected automatic promotion approval")
        return self.dispatch_main_workflow(
            approval.source.repository,
            "promote-pull-request.yml",
            {
                "pull_request": str(approval.source.number),
                "head_sha": str(approval.head),
                "approval_id": str(approval.event_id),
            },
        )

    def sync_uv_dev_main(self, upstream_reader: PromotionRevisionReader) -> CommitSha:
        self.get_repository(UV_DEV_REPOSITORY)
        source_main = self.get_ref(UV_DEV_REPOSITORY, "main")
        if source_main is None or not is_public_main_revision(
            upstream_reader, source_main
        ):
            raise ValueError("The uv-dev main branch is not public uv history")
        self._api(
            "POST",
            f"repos/{UV_DEV_REPOSITORY.name}/merge-upstream",
            payload={"branch": "main"},
        )
        main = self.get_ref(UV_DEV_REPOSITORY, "main")
        if main is None:
            raise ValueError("The synchronized uv-dev main branch is missing")
        if not is_public_main_revision(upstream_reader, main):
            raise ValueError("The synchronized uv-dev main is not public uv history")
        return main

    def create_upstream_base(self, destination: BranchRevision) -> None:
        if destination.repository != UV_REPOSITORY:
            raise ValueError("Unexpected promotion base destination")
        self.get_repository(UV_REPOSITORY)
        current = self.get_ref(UV_REPOSITORY, destination.ref)
        if current == destination.sha:
            return
        if current is not None:
            raise ValueError("The upstream base branch changed before creation")
        try:
            self._api(
                "POST",
                f"repos/{UV_REPOSITORY.name}/git/refs",
                payload={
                    "ref": f"refs/heads/{destination.ref}",
                    "sha": str(destination.sha),
                },
            )
        except PromotionReadError:
            # Another identical promotion may have won the create-only race.
            if self.get_ref(UV_REPOSITORY, destination.ref) != destination.sha:
                raise
        if self.get_ref(UV_REPOSITORY, destination.ref) != destination.sha:
            raise ValueError("GitHub did not create the expected upstream base")
