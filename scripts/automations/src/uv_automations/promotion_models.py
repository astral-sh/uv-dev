"""Immutable identities and evidence shared by promotion workflows."""

import re
from dataclasses import dataclass
from enum import StrEnum
from typing import ClassVar, assert_never

from uv_automations.git import check_branch
from uv_automations.json import (
    as_object,
    as_positive_integer,
    as_string,
    require_keys,
)
from uv_automations.models import (
    ActorKind,
    CommitSha,
    ManagedRepository,
    PullRequestDetails,
    PullRequestRef,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

MAX_PROMOTION_PAGE_SIZE = 100
MAX_PROMOTION_PAGES = 10
MAX_PROMOTION_COMMENT_LENGTH = 65_536

UV_REPOSITORY = RepositoryIdentity(
    RepositoryName(ManagedRepository.UV.value), 699532645
)
UV_DEV_REPOSITORY = RepositoryIdentity(
    RepositoryName(ManagedRepository.UV_DEV.value), 1302176231
)
UV_SECURITY_REPOSITORY = RepositoryIdentity(
    RepositoryName(ManagedRepository.UV_SECURITY.value), 1333576902
)
AUTOMATIONS_BOT_ID = 305554984
AUTOMATIONS_APP_ID = 4307167
AUTOMATIONS_APP_SLUG = "astral-automations-bot"


def require_promotion_repository(repository: RepositoryIdentity) -> None:
    if repository not in (UV_REPOSITORY, UV_DEV_REPOSITORY, UV_SECURITY_REPOSITORY):
        raise ValueError("Unexpected promotion repository identity")


def require_promotion_source(repository: RepositoryIdentity) -> None:
    if repository not in (UV_DEV_REPOSITORY, UV_SECURITY_REPOSITORY):
        raise ValueError("Unexpected promotion source repository identity")


def check_promotion_branch(ref: str) -> None:
    as_string(ref)
    try:
        check_branch(ref)
    except ValueError:
        # A branch name from uv-security must not leak into a public sync log.
        raise ValueError("Invalid promotion branch name") from None


@dataclass(frozen=True, slots=True)
class PromotionScope:
    repository: RepositoryIdentity
    number: int

    def __post_init__(self) -> None:
        require_promotion_repository(self.repository)
        as_positive_integer(self.number)

    @property
    def reference(self) -> PullRequestRef:
        return PullRequestRef(self.repository.name, self.number)

    def to_json(self) -> dict[str, object]:
        return {
            "repository": str(self.repository.name),
            "repository_id": self.repository.database_id,
            "pull_request": self.number,
        }

    @classmethod
    def from_json(cls, value: object) -> PromotionScope:
        data = as_object(value)
        require_keys(data, {"repository", "repository_id", "pull_request"})
        return cls(
            RepositoryIdentity(
                RepositoryName(as_string(data["repository"])),
                as_positive_integer(data["repository_id"]),
            ),
            as_positive_integer(data["pull_request"]),
        )


@dataclass(frozen=True, slots=True)
class BranchRevision:
    repository: RepositoryIdentity
    ref: str
    sha: CommitSha

    def __post_init__(self) -> None:
        require_promotion_repository(self.repository)
        check_promotion_branch(self.ref)

    def to_json(self) -> dict[str, object]:
        return {
            "repository": str(self.repository.name),
            "repository_id": self.repository.database_id,
            "ref": self.ref,
            "sha": str(self.sha),
        }

    @classmethod
    def from_json(cls, value: object) -> BranchRevision:
        data = as_object(value)
        require_keys(data, {"repository", "repository_id", "ref", "sha"})
        return cls(
            RepositoryIdentity(
                RepositoryName(as_string(data["repository"])),
                as_positive_integer(data["repository_id"]),
            ),
            as_string(data["ref"]),
            CommitSha(as_string(data["sha"])),
        )


@dataclass(frozen=True, slots=True)
class PromotionActor:
    login: str
    database_id: int
    kind: ActorKind

    def __post_init__(self) -> None:
        as_positive_integer(self.database_id)
        if not isinstance(self.kind, ActorKind):
            raise TypeError("Expected a GitHub actor kind")
        if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*(?:\[bot\])?", self.login) is None:
            raise ValueError("Invalid promotion actor login")

    @property
    def is_human(self) -> bool:
        return self.kind == ActorKind.USER and self.login not in {
            AUTOMATIONS_APP_SLUG,
            f"{AUTOMATIONS_APP_SLUG}[bot]",
        }


@dataclass(frozen=True, slots=True)
class GitHubAppIdentity:
    database_id: int
    slug: str

    def __post_init__(self) -> None:
        as_positive_integer(self.database_id)
        if not self.slug or len(self.slug) > 100:
            raise ValueError("Invalid GitHub App slug")


AUTOMATIONS_APP = GitHubAppIdentity(AUTOMATIONS_APP_ID, AUTOMATIONS_APP_SLUG)


class PullRequestSelection(StrEnum):
    OPEN = "open"
    CLOSED = "closed"
    ALL = "all"


@dataclass(frozen=True, slots=True)
class PullRequestMerge:
    sha: CommitSha
    merged_at: Timestamp


@dataclass(frozen=True, slots=True)
class PromotionPullRequest:
    scope: PromotionScope
    details: PullRequestDetails
    draft: bool
    author: PromotionActor | None
    title: str
    body: str
    merge: PullRequestMerge | None

    def __post_init__(self) -> None:
        if (
            self.details.reference != self.scope.reference
            or self.details.base.repository != self.scope.repository
        ):
            raise ValueError("Unexpected promotion pull request identity")
        if type(self.draft) is not bool:
            raise TypeError("Expected a pull request draft boolean")
        check_promotion_branch(self.details.base.ref)
        check_promotion_branch(self.details.head.ref)
        if self.merge is not None and self.details.state != PullRequestState.CLOSED:
            raise ValueError("A merged pull request must be closed")

    @property
    def is_open(self) -> bool:
        return self.details.is_open

    @property
    def same_repository(self) -> bool:
        return self.details.head.repository == self.scope.repository


class PromotionEventKind(StrEnum):
    READY_FOR_REVIEW = "ready_for_review"
    CONVERT_TO_DRAFT = "convert_to_draft"
    LABELED = "labeled"
    UNLABELED = "unlabeled"


@dataclass(frozen=True, slots=True)
class PromotionEventData:
    identifier: int
    actor: PromotionActor | None
    created_at: Timestamp

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)


@dataclass(frozen=True, slots=True)
class ReadyForReviewEvent(PromotionEventData):
    kind: ClassVar[PromotionEventKind] = PromotionEventKind.READY_FOR_REVIEW


@dataclass(frozen=True, slots=True)
class ConvertedToDraftEvent(PromotionEventData):
    kind: ClassVar[PromotionEventKind] = PromotionEventKind.CONVERT_TO_DRAFT


@dataclass(frozen=True, slots=True)
class LabelAddedEvent(PromotionEventData):
    label: str
    kind: ClassVar[PromotionEventKind] = PromotionEventKind.LABELED


@dataclass(frozen=True, slots=True)
class LabelRemovedEvent(PromotionEventData):
    label: str
    kind: ClassVar[PromotionEventKind] = PromotionEventKind.UNLABELED


type PromotionEvent = (
    ReadyForReviewEvent | ConvertedToDraftEvent | LabelAddedEvent | LabelRemovedEvent
)


class PromotionApprovalKind(StrEnum):
    READY_FOR_REVIEW = "ready_for_review"
    LABELED = "labeled"


@dataclass(frozen=True, slots=True)
class PromotionApproval:
    """A human event tied to a caller-supplied, exact approved revision.

    Issue-event REST responses do not attest to the pull request head. The head
    must come from the trusted dispatch and be rechecked against the current PR.
    """

    source: PromotionScope
    head: CommitSha
    event: ReadyForReviewEvent | LabelAddedEvent
    ready_event_id: int

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        as_positive_integer(self.ready_event_id)
        if self.event.actor is None or not self.event.actor.is_human:
            raise ValueError("Promotion approval requires a human actor")
        match self.event:
            case ReadyForReviewEvent(identifier=identifier):
                if self.ready_event_id != identifier:
                    raise ValueError("Readiness approval has a different event ID")
                return
            case LabelAddedEvent(identifier=identifier):
                if self.ready_event_id >= identifier:
                    raise ValueError("Label approval must follow the readiness event")
                return
        assert_never(self.event)

    @property
    def kind(self) -> PromotionApprovalKind:
        match self.event:
            case ReadyForReviewEvent():
                return PromotionApprovalKind.READY_FOR_REVIEW
            case LabelAddedEvent():
                return PromotionApprovalKind.LABELED
        assert_never(self.event)

    @property
    def event_id(self) -> int:
        return self.event.identifier

    @property
    def actor(self) -> PromotionActor:
        actor = self.event.actor
        if actor is None:
            raise ValueError("Promotion approval has no actor")
        return actor

    @property
    def claim(self) -> PromotionApprovalClaim:
        return PromotionApprovalClaim(
            self.source,
            self.head,
            self.kind,
            self.event_id,
            self.ready_event_id,
            self.actor.database_id,
        )

    def to_json(self) -> dict[str, object]:
        """Serialize claimed authority; readers must fetch the events again."""
        return self.claim.to_json()


@dataclass(frozen=True, slots=True)
class PromotionApprovalClaim:
    """An untrusted identity claim, not evidence that approval remains valid."""

    source: PromotionScope
    head: CommitSha
    kind: PromotionApprovalKind
    event_id: int
    ready_event_id: int
    actor_id: int

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        as_positive_integer(self.event_id)
        as_positive_integer(self.ready_event_id)
        as_positive_integer(self.actor_id)
        match self.kind:
            case PromotionApprovalKind.READY_FOR_REVIEW:
                if self.ready_event_id != self.event_id:
                    raise ValueError("Readiness claim has a different event ID")
                return
            case PromotionApprovalKind.LABELED:
                if self.ready_event_id >= self.event_id:
                    raise ValueError("Label claim must follow the readiness event")
                return
        assert_never(self.kind)

    def matches(self, approval: PromotionApproval) -> bool:
        return self == approval.claim

    def to_json(self) -> dict[str, object]:
        return {
            "source": self.source.to_json(),
            "head": str(self.head),
            "kind": self.kind.value,
            "event_id": self.event_id,
            "ready_event_id": self.ready_event_id,
            "actor_id": self.actor_id,
        }

    @classmethod
    def from_json(cls, value: object) -> PromotionApprovalClaim:
        data = as_object(value)
        require_keys(
            data,
            {"source", "head", "kind", "event_id", "ready_event_id", "actor_id"},
        )
        return cls(
            PromotionScope.from_json(data["source"]),
            CommitSha(as_string(data["head"])),
            PromotionApprovalKind(as_string(data["kind"])),
            as_positive_integer(data["event_id"]),
            as_positive_integer(data["ready_event_id"]),
            as_positive_integer(data["actor_id"]),
        )


def latest_ready_event(
    events: tuple[PromotionEvent, ...], *, human_only: bool = False
) -> ReadyForReviewEvent | None:
    latest: ReadyForReviewEvent | None = None
    for event in events:
        if (
            isinstance(event, ReadyForReviewEvent)
            and (not human_only or (event.actor is not None and event.actor.is_human))
            and (latest is None or event.identifier > latest.identifier)
        ):
            latest = event
    return latest


def current_ready_event(
    events: tuple[PromotionEvent, ...], *, human_only: bool = False
) -> ReadyForReviewEvent | None:
    """Return readiness only when it is the latest ready/draft transition."""
    latest: ReadyForReviewEvent | ConvertedToDraftEvent | None = None
    for event in events:
        match event:
            case ReadyForReviewEvent() | ConvertedToDraftEvent():
                if latest is None or event.identifier > latest.identifier:
                    latest = event
            case LabelAddedEvent() | LabelRemovedEvent():
                pass
            case _:
                assert_never(event)
    match latest:
        case ReadyForReviewEvent(actor=actor) if not human_only or (
            actor is not None and actor.is_human
        ):
            return latest
        case ReadyForReviewEvent() | ConvertedToDraftEvent() | None:
            return None
    assert_never(latest)


def latest_label_event(
    events: tuple[PromotionEvent, ...], label: str
) -> LabelAddedEvent | LabelRemovedEvent | None:
    latest: LabelAddedEvent | LabelRemovedEvent | None = None
    for event in events:
        if (
            isinstance(event, (LabelAddedEvent, LabelRemovedEvent))
            and event.label == label
            and (latest is None or event.identifier > latest.identifier)
        ):
            latest = event
    return latest


def ready_approval(
    scope: PromotionScope, head: CommitSha, events: tuple[PromotionEvent, ...]
) -> PromotionApproval | None:
    event = latest_ready_event(events, human_only=True)
    return (
        PromotionApproval(scope, head, event, event.identifier)
        if event is not None
        else None
    )


def current_ready_approval(
    scope: PromotionScope, head: CommitSha, events: tuple[PromotionEvent, ...]
) -> PromotionApproval | None:
    """Require the latest readiness transition to be a human approval."""
    event = current_ready_event(events, human_only=True)
    return (
        PromotionApproval(scope, head, event, event.identifier)
        if event is not None
        else None
    )


@dataclass(frozen=True, slots=True)
class PromotionComment:
    scope: PromotionScope
    identifier: int
    author: PromotionActor | None
    app: GitHubAppIdentity | None
    body: str
    created_at: Timestamp
    updated_at: Timestamp

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        if len(self.body) > MAX_PROMOTION_COMMENT_LENGTH:
            raise ValueError("Promotion comment exceeds the body budget")
        if self.updated_at < self.created_at:
            raise ValueError("Promotion comment predates its creation")

    @property
    def is_automation(self) -> bool:
        return (
            self.author is not None
            and self.author.database_id == AUTOMATIONS_BOT_ID
            and self.author.kind == ActorKind.BOT
            and self.app == AUTOMATIONS_APP
        )


@dataclass(frozen=True, slots=True)
class UneditedPromotionComment:
    """A bot-issued comment whose current body has separate never-edited proof."""

    comment: PromotionComment

    def __post_init__(self) -> None:
        if not self.comment.is_automation:
            raise ValueError("An authoritative promotion comment requires the bot App")


@dataclass(frozen=True, slots=True)
class PromotionRecord:
    source: PromotionScope
    upstream: PromotionScope
    comment_id: int

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        if self.upstream.repository != UV_REPOSITORY:
            raise ValueError("Promotion record has an unexpected upstream repository")
        as_positive_integer(self.comment_id)


class AmbiguousPromotionRecord(ValueError):
    """More than one distinct bot-issued upstream identity was recorded."""


def promotion_record(comment: PromotionComment) -> PromotionRecord | None:
    """Read legacy historical evidence, not fresh consent to publish a head."""
    if not comment.is_automation:
        return None
    match = re.fullmatch(
        r"Promoted to \[#([1-9][0-9]*)\]"
        r"\(https://github\.com/astral-sh/uv/pull/([1-9][0-9]*)\)\.",
        comment.body,
    )
    if match is None or match[1] != match[2]:
        return None
    return PromotionRecord(
        comment.scope, PromotionScope(UV_REPOSITORY, int(match[1])), comment.identifier
    )


def unique_promotion_record(
    source: PromotionScope, comments: tuple[PromotionComment, ...]
) -> PromotionRecord | None:
    records: dict[PromotionScope, PromotionRecord] = {}
    identifiers: set[int] = set()
    for comment in comments:
        if comment.scope != source or comment.identifier in identifiers:
            raise ValueError("Promotion comments have inconsistent identities")
        identifiers.add(comment.identifier)
        record = promotion_record(comment)
        if record is not None:
            previous = records.get(record.upstream)
            if previous is None or record.comment_id < previous.comment_id:
                records[record.upstream] = record
    if len(records) > 1:
        raise AmbiguousPromotionRecord(
            "Found multiple distinct upstream promotion records"
        )
    return next(iter(records.values()), None)


class RepositoryPermission(StrEnum):
    ADMIN = "admin"
    WRITE = "write"
    READ = "read"
    NONE = "none"

    @property
    def can_write(self) -> bool:
        # GitHub maps maintain to write and triage to read in `permission`.
        match self:
            case RepositoryPermission.ADMIN | RepositoryPermission.WRITE:
                return True
            case RepositoryPermission.READ | RepositoryPermission.NONE:
                return False
        assert_never(self)


class ComparisonStatus(StrEnum):
    AHEAD = "ahead"
    BEHIND = "behind"
    DIVERGED = "diverged"
    IDENTICAL = "identical"


@dataclass(frozen=True, slots=True)
class CommitComparison:
    repository: RepositoryIdentity
    base: CommitSha
    head: CommitSha
    status: ComparisonStatus
    merge_base: CommitSha

    def __post_init__(self) -> None:
        require_promotion_repository(self.repository)
        match self.status:
            case ComparisonStatus.IDENTICAL:
                if self.base != self.head or self.merge_base != self.base:
                    raise ValueError("Inconsistent identical-commit comparison")
                return
            case ComparisonStatus.AHEAD:
                if self.base == self.head or self.merge_base != self.base:
                    raise ValueError("Inconsistent descendant-commit comparison")
                return
            case ComparisonStatus.BEHIND:
                if self.base == self.head or self.merge_base != self.head:
                    raise ValueError("Inconsistent ancestor-commit comparison")
                return
            case ComparisonStatus.DIVERGED:
                if self.merge_base in (self.base, self.head):
                    raise ValueError("Inconsistent diverged-commit comparison")
                return
        assert_never(self.status)

    @property
    def is_ancestor(self) -> bool:
        """Whether the requested base is an ancestor of, or identical to, head."""
        match self.status:
            case ComparisonStatus.AHEAD | ComparisonStatus.IDENTICAL:
                return True
            case ComparisonStatus.BEHIND | ComparisonStatus.DIVERGED:
                return False
        assert_never(self.status)


@dataclass(frozen=True, slots=True)
class HeadForcePush:
    identifier: str
    before: CommitSha | None
    after: CommitSha | None
    created_at: Timestamp

    def __post_init__(self) -> None:
        if not self.identifier or len(self.identifier) > 200:
            raise ValueError("Invalid force-push event identity")

    def contains(self, sha: CommitSha) -> bool:
        return sha == self.before or sha == self.after


@dataclass(frozen=True, slots=True)
class PromotedParentEvidence:
    source: PromotionPullRequest
    upstream: PromotionPullRequest
    record: PromotionRecord

    def __post_init__(self) -> None:
        author = self.upstream.author
        if (
            self.record.source != self.source.scope
            or self.record.upstream != self.upstream.scope
            or self.source.is_open
            or not self.source.same_repository
            or not self.upstream.same_repository
            or self.upstream.details.head.ref != self.source.details.head.ref
            or author is None
            or author.database_id != AUTOMATIONS_BOT_ID
            or author.kind != ActorKind.BOT
        ):
            raise ValueError("Inconsistent promoted-parent evidence")

    @property
    def original_head(self) -> CommitSha:
        return self.source.details.head.sha


@dataclass(frozen=True, slots=True)
class OpenPromotedParent(PromotedParentEvidence):
    def __post_init__(self) -> None:
        PromotedParentEvidence.__post_init__(self)
        if not self.upstream.is_open:
            raise ValueError("Promoted upstream parent is not open")


@dataclass(frozen=True, slots=True)
class ClosedPromotedParent(PromotedParentEvidence):
    def __post_init__(self) -> None:
        PromotedParentEvidence.__post_init__(self)
        if self.upstream.is_open or self.upstream.merge is not None:
            raise ValueError("Promoted upstream parent is not closed without merging")


@dataclass(frozen=True, slots=True)
class MergedPromotedParent(PromotedParentEvidence):
    merge: PullRequestMerge

    def __post_init__(self) -> None:
        PromotedParentEvidence.__post_init__(self)
        if self.upstream.merge is None or self.upstream.merge != self.merge:
            raise ValueError("Promoted upstream parent has a different merge")


type VerifiedPromotedParent = (
    OpenPromotedParent | ClosedPromotedParent | MergedPromotedParent
)


@dataclass(frozen=True, slots=True)
class UnrecordedMergedParent:
    """An exact-head manual promotion path, not automatic replay authority."""

    upstream: PromotionPullRequest
    original_head: CommitSha

    def __post_init__(self) -> None:
        if (
            self.upstream.scope.repository != UV_REPOSITORY
            or not self.upstream.same_repository
            or self.upstream.details.head.sha != self.original_head
            or self.upstream.merge is None
        ):
            raise ValueError("Unrecorded merged-parent evidence is incomplete")

    @property
    def merge(self) -> PullRequestMerge:
        merge = self.upstream.merge
        if merge is None:
            raise ValueError("Unrecorded parent is not merged")
        return merge


@dataclass(frozen=True, slots=True)
class SynchronizedParent:
    parent: MergedPromotedParent
    main: BranchRevision


@dataclass(frozen=True, slots=True)
class UnsynchronizedParent:
    parent: MergedPromotedParent
    main: BranchRevision
