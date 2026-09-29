"""Narrow writes for promotion queues, fork synchronization, and base copies."""

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.github_promotion import (
    PromotionGitHub,
    PromotionReadError,
    PromotionRevisionReader,
    decode_promotion_comment,
)
from uv_automations.github_promotion_completion import PromotionCompletionGitHub
from uv_automations.models import CommitSha
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    PromotionApprovalClaim,
)
from uv_automations.workflows.promotion_completion import (
    COMPLETION_ATTEMPTS,
    _retry_delay,
)
from uv_automations.workflows.promotion_queue import (
    QueuedPromotion,
    _main_contains,
    is_public_main_revision,
)


class PromotionQueueGitHub(PromotionGitHub):
    def create_queue_comment(self, queued: QueuedPromotion) -> None:
        scope = queued.source
        body = queued.comment()
        try:
            comment = decode_promotion_comment(
                self._api(
                    "POST",
                    f"repos/{scope.repository.name}/issues/{scope.number}/comments",
                    payload={"body": body},
                ),
                scope,
            )
            if not comment.is_automation or comment.body != body:
                raise ValueError("Unexpected promotion queue record")
            original = self.get_unedited_promotion_comment(scope, comment.identifier)
            if original is None or original.comment.body != body:
                raise ValueError("The new promotion queue record was edited")
        except KeyError, TypeError, ValueError:
            # The POST can have committed even when its response or edit proof
            # cannot attest to the receipt. Reconcile without exposing fields.
            raise PromotionReadError("Invalid GitHub promotion response") from None

    def dispatch_promotion(self, approval: PromotionApprovalClaim) -> WorkflowDispatch:
        if approval.source.repository != UV_DEV_REPOSITORY:
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
        intended: CommitSha | None = None
        merge_attempted = False
        already_synchronized = False
        for attempt in range(COMPLETION_ATTEMPTS):
            waited = False
            try:
                if intended is None:
                    self.get_repository(UV_DEV_REPOSITORY)
                    intended = upstream_reader.get_ref(UV_REPOSITORY, "main")
                    if intended is None:
                        raise ValueError("The public uv main branch is missing")
                if not merge_attempted and not already_synchronized:
                    source_main = self.get_ref(UV_DEV_REPOSITORY, "main")
                    if source_main is None:
                        raise ValueError(
                            "The uv-dev main branch is not public uv history"
                        )
                    if _main_contains(
                        upstream_reader, UV_REPOSITORY, intended, source_main
                    ):
                        already_synchronized = source_main == intended
                    elif _main_contains(
                        upstream_reader, UV_REPOSITORY, source_main, intended
                    ):
                        already_synchronized = True
                    else:
                        raise ValueError(
                            "The uv-dev main branch is not public uv history"
                        )
                    if not already_synchronized:
                        # A failed response can follow a completed synchronization.
                        # Once attempted, only read-only reconciliation is allowed.
                        merge_attempted = True
                        try:
                            self._api(
                                "POST",
                                f"repos/{UV_DEV_REPOSITORY.name}/merge-upstream",
                                payload={"branch": "main"},
                            )
                        except PromotionReadError as error:
                            _retry_delay(attempt, error, before_reconciliation=True)
                            waited = True
                main = self._synchronized_main(upstream_reader, intended)
                if main is not None:
                    return main
            except PromotionReadError as error:
                if attempt + 1 == COMPLETION_ATTEMPTS:
                    raise
                _retry_delay(attempt, error)
                continue
            if not waited:
                _retry_delay(attempt)
        raise PromotionReadError("Could not confirm the uv-dev synchronization")

    def _synchronized_main(
        self, upstream_reader: PromotionRevisionReader, intended: CommitSha
    ) -> CommitSha | None:
        main = self.get_ref(UV_DEV_REPOSITORY, "main")
        if main is None:
            return None
        if not is_public_main_revision(upstream_reader, main):
            raise ValueError("The synchronized uv-dev main is not public uv history")
        if not _main_contains(upstream_reader, UV_REPOSITORY, main, intended):
            return None
        if self.get_ref(UV_DEV_REPOSITORY, "main") != main:
            return None
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


class PromotionQueueCompletionGitHub(PromotionQueueGitHub, PromotionCompletionGitHub):
    """Use bounded completion response metadata only for queue receipt recording."""


class PromotionSyncCompletionGitHub(PromotionQueueGitHub, PromotionCompletionGitHub):
    """Use bounded completion response metadata only for fork synchronization."""
