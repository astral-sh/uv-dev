import unittest
from dataclasses import dataclass, field, replace
from unittest.mock import patch

from uv_automations.comment_models import ActorKind
from uv_automations.github_promotion import PromotionGitHub
from uv_automations.models import (
    CommitSha,
    PullRequestDetails,
    PullRequestRevision,
    PullRequestState,
    Timestamp,
)
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    BranchRevision,
    PromotionActor,
    PromotionApproval,
    PromotionPullRequest,
    PromotionScope,
    ReadyForReviewEvent,
)
from uv_automations.workflows.promotion import (
    AlreadyPublished,
    CopyUpstreamBase,
    Publish,
    Rejected,
)
from uv_automations.workflows.promotion_publish import (
    BaseCopyOutcome,
    ensure_upstream_base,
)

HEAD = CommitSha("a" * 40)
BASE = CommitSha("b" * 40)
MAIN = CommitSha("c" * 40)
OTHER = CommitSha("d" * 40)
ACTOR = PromotionActor("zanieb", 1, ActorKind.USER)
EVENT = ReadyForReviewEvent(2, ACTOR, Timestamp.parse("2026-09-09T12:00:00Z"))
SOURCE_SCOPE = PromotionScope(UV_DEV_REPOSITORY, 10)
PARENT_SCOPE = PromotionScope(UV_REPOSITORY, 20)
DESTINATION = BranchRevision(UV_REPOSITORY, "parent", BASE)


def pull_request(
    scope: PromotionScope, base: PullRequestRevision, head: PullRequestRevision
) -> PromotionPullRequest:
    return PromotionPullRequest(
        scope,
        PullRequestDetails(
            scope.reference,
            PullRequestState.OPEN,
            f"https://github.com/{scope.repository.name}/pull/{scope.number}",
            base,
            head,
            (),
        ),
        False,
        ACTOR,
        "Example",
        "",
        None,
    )


SOURCE = pull_request(
    SOURCE_SCOPE,
    PullRequestRevision(UV_DEV_REPOSITORY, "parent", BASE),
    PullRequestRevision(UV_DEV_REPOSITORY, "child", HEAD),
)
PARENT = pull_request(
    PARENT_SCOPE,
    PullRequestRevision(UV_REPOSITORY, "main", MAIN),
    PullRequestRevision(UV_DEV_REPOSITORY, "parent", BASE),
)
APPROVAL = PromotionApproval(SOURCE_SCOPE, HEAD, EVENT, EVENT.identifier)
COPY = CopyUpstreamBase(SOURCE, APPROVAL, PARENT, DESTINATION)


@dataclass
class Writer:
    created: list[BranchRevision] = field(default_factory=list)

    def create_upstream_base(self, destination: BranchRevision) -> None:
        self.created.append(destination)


class PromotionBaseCopyTests(unittest.TestCase):
    def execute(self, plan: object, writer: Writer) -> BaseCopyOutcome:
        with patch(
            "uv_automations.workflows.promotion_publish.plan_promotion",
            return_value=plan,
        ):
            return ensure_upstream_base(
                PromotionGitHub(), PromotionGitHub(), writer, COPY.claim
            )

    def test_fresh_complete_claim_creates_only_its_exact_ref(self) -> None:
        writer = Writer()
        self.assertEqual(
            self.execute(Publish(SOURCE, APPROVAL, DESTINATION, COPY), writer),
            BaseCopyOutcome.CREATED,
        )
        self.assertEqual(writer.created, [DESTINATION])

    def test_successful_creation_is_idempotent(self) -> None:
        writer = Writer()
        self.assertEqual(
            self.execute(Publish(SOURCE, APPROVAL, DESTINATION), writer),
            BaseCopyOutcome.UNCHANGED,
        )
        self.assertFalse(writer.created)

    def test_current_existing_publication_is_idempotent(self) -> None:
        upstream = pull_request(
            PromotionScope(UV_REPOSITORY, 30),
            PullRequestRevision(UV_REPOSITORY, "parent", BASE),
            PullRequestRevision(UV_REPOSITORY, "child", HEAD),
        )
        writer = Writer()
        self.assertEqual(
            self.execute(
                AlreadyPublished(SOURCE, APPROVAL, upstream, DESTINATION), writer
            ),
            BaseCopyOutcome.UNCHANGED,
        )
        self.assertFalse(writer.created)

    def test_changed_approval_parent_or_destination_does_not_write(self) -> None:
        new_event = replace(EVENT, identifier=3)
        new_approval = PromotionApproval(
            SOURCE_SCOPE, HEAD, new_event, new_event.identifier
        )
        changed_parent = replace(
            PARENT,
            details=replace(
                PARENT.details, base=replace(PARENT.details.base, sha=OTHER)
            ),
        )
        plans = (
            Publish(SOURCE, new_approval, DESTINATION),
            Publish(
                SOURCE,
                APPROVAL,
                DESTINATION,
                CopyUpstreamBase(SOURCE, APPROVAL, changed_parent, DESTINATION),
            ),
            Publish(SOURCE, APPROVAL, replace(DESTINATION, sha=OTHER)),
            Rejected(SOURCE, "Parent no longer matches", APPROVAL),
        )
        for plan in plans:
            with self.subTest(plan=plan):
                writer = Writer()
                self.assertEqual(self.execute(plan, writer), BaseCopyOutcome.STALE)
                self.assertFalse(writer.created)


if __name__ == "__main__":
    unittest.main()
