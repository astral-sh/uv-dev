"""Explicit approval and request stages for private pull request promotion."""

import json
from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, assert_never

from uv_automations.github_actions import WorkflowDispatch
from uv_automations.json import (
    as_object,
    as_positive_integer,
    as_string,
    loads,
    require_keys,
)
from uv_automations.models import ActorKind, CommitSha, RepositoryIdentity, Timestamp
from uv_automations.promotion_models import (
    AUTOMATIONS_APP_SLUG,
    AUTOMATIONS_BOT_ID,
    UV_SECURITY_REPOSITORY,
    LabelAddedEvent,
    PromotionActor,
    PromotionApproval,
    PromotionApprovalClaim,
    PromotionApprovalKind,
    PromotionComment,
    PromotionEvent,
    PromotionPullRequest,
    PromotionScope,
    ReadyForReviewEvent,
    RepositoryPermission,
    UneditedPromotionComment,
    current_ready_event,
    latest_label_event,
)

PRIVATE_PROMOTION_LABEL = "bot:promote"
RECEIPT_TEXT_PREFIX = "Recorded approval to publish commit `"
RECEIPT_MARKER = "<!-- uv-security-promotion-approval:"
RECEIPT_SUFFIX = " -->"
REMINDER_TEXT = (
    "To promote this pull request to astral-sh/uv, add `bot:promote` after "
    "marking it ready for review. If the label was applied before this "
    "ready-for-review event, remove and re-add it."
)
TRUSTED_DISPATCHER = PromotionActor(
    f"{AUTOMATIONS_APP_SLUG}[bot]", AUTOMATIONS_BOT_ID, ActorKind.BOT
)
PRIVATE_APPROVAL_DEPENDENCY = (
    "Private approval capture is not enabled: the dispatcher must first preserve "
    "authenticated original-event metadata. No approval receipt was recorded "
    "and no workflow was dispatched."
)


def _require_private_source(scope: PromotionScope) -> None:
    if scope.repository != UV_SECURITY_REPOSITORY:
        raise ValueError("Private promotion requires the uv-security repository")


class ApprovalUnavailableReason(StrEnum):
    NOT_READY = "not_ready"
    HEAD_CHANGED = "head_changed"
    NO_READINESS = "no_readiness"
    LABEL_MISSING = "label_missing"
    LABEL_NOT_CURRENT = "label_not_current"
    LABEL_PRECEDES_READINESS = "label_precedes_readiness"
    APPROVER_NOT_HUMAN = "approver_not_human"
    APPROVER_CANNOT_WRITE = "approver_cannot_write"
    APPROVAL_CHANGED = "approval_changed"
    EVENT_NOT_CORRELATED = "event_not_correlated"
    RECEIPT_CONFLICT = "receipt_conflict"

    @property
    def message(self) -> str:
        match self:
            case ApprovalUnavailableReason.NOT_READY:
                return "The private pull request is no longer ready for promotion."
            case ApprovalUnavailableReason.HEAD_CHANGED:
                return "The private pull request no longer has the dispatched head."
            case ApprovalUnavailableReason.NO_READINESS:
                return "The private pull request has no current ready-for-review event."
            case ApprovalUnavailableReason.LABEL_MISSING:
                return "The private pull request no longer has bot:promote."
            case ApprovalUnavailableReason.LABEL_NOT_CURRENT:
                return "The current bot:promote application could not be verified."
            case ApprovalUnavailableReason.LABEL_PRECEDES_READINESS:
                return "Apply bot:promote after the latest ready-for-review event."
            case ApprovalUnavailableReason.APPROVER_NOT_HUMAN:
                return "The current promotion label was not applied by a human."
            case ApprovalUnavailableReason.APPROVER_CANNOT_WRITE:
                return "The promotion approver is no longer a repository writer."
            case ApprovalUnavailableReason.APPROVAL_CHANGED:
                return "The private promotion approval changed before publication."
            case ApprovalUnavailableReason.EVENT_NOT_CORRELATED:
                return "The webhook cannot be correlated to one current approval event."
            case ApprovalUnavailableReason.RECEIPT_CONFLICT:
                return "The label event already has a different approval receipt."
        assert_never(self)


class PrivateApprovalUnavailable(ValueError):
    def __init__(self, reason: ApprovalUnavailableReason) -> None:
        self.reason = reason
        super().__init__(reason.message)


@dataclass(frozen=True, slots=True)
class PrivatePromotionRequest:
    """The scalar inputs preserved from one trusted webhook delivery.

    GitHub does not put the issue-event ID in a pull-request webhook. Matching
    the sender and second-resolution timestamp below is a fail-closed
    correlation guard, not an attestation that REST events contain a head SHA.
    The approved head is always the webhook's head, never a replacement fetched
    from the current pull request. The current dispatcher's refreshed
    ``/pull_request/updated_at`` input cannot supply ``original_updated_at``.
    """

    source: PromotionScope
    head: CommitSha
    kind: PromotionApprovalKind
    dispatcher: PromotionActor
    sender: PromotionActor
    original_updated_at: Timestamp

    def __post_init__(self) -> None:
        _require_private_source(self.source)
        if self.dispatcher != TRUSTED_DISPATCHER:
            raise ValueError("Private approval requires the trusted webhook dispatcher")
        if not self.sender.is_human:
            raise ValueError("Private approval requires a human webhook sender")


@dataclass(frozen=True, slots=True)
class PrivateApprovalReference:
    """The exact, caller-preserved identity of an approval receipt."""

    source: PromotionScope
    head: CommitSha
    approval_id: int
    ready_event_id: int
    receipt_id: int

    def __post_init__(self) -> None:
        _require_private_source(self.source)
        as_positive_integer(self.approval_id)
        as_positive_integer(self.ready_event_id)
        as_positive_integer(self.receipt_id)
        if self.ready_event_id >= self.approval_id:
            raise ValueError("Private label approval must follow readiness")

    def dispatch_inputs(self) -> dict[str, str]:
        return {
            "pull_request": str(self.source.number),
            "head_sha": str(self.head),
            "approval_kind": PromotionApprovalKind.LABELED.value,
            "approval_id": str(self.approval_id),
            "ready_event_id": str(self.ready_event_id),
            "approval_receipt_id": str(self.receipt_id),
        }


@dataclass(frozen=True, slots=True)
class PrivateApprovalReceiptClaim:
    """An untrusted receipt payload that must match a fresh approval read."""

    approval: PromotionApprovalClaim
    actor: PromotionActor
    created_at: Timestamp

    def __post_init__(self) -> None:
        _require_private_source(self.approval.source)
        if (
            self.approval.kind != PromotionApprovalKind.LABELED
            or not self.actor.is_human
            or self.actor.database_id != self.approval.actor_id
        ):
            raise ValueError("Invalid private promotion approval claim")

    @classmethod
    def from_approval(cls, approval: PromotionApproval) -> PrivateApprovalReceiptClaim:
        event = _require_label_approval(approval)
        return cls(approval.claim, approval.actor, event.created_at)

    def matches(self, approval: PromotionApproval) -> bool:
        return (
            self.approval.matches(approval)
            and self.actor == approval.actor
            and self.created_at == approval.event.created_at
        )

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "approval": self.approval.to_json(),
            "actor_login": self.actor.login,
            "actor_type": self.actor.kind.value,
            "created_at": str(self.created_at),
        }

    @classmethod
    def from_json(cls, value: object) -> PrivateApprovalReceiptClaim:
        data = as_object(value)
        require_keys(
            data,
            {"version", "approval", "actor_login", "actor_type", "created_at"},
        )
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported private approval receipt version")
        approval = PromotionApprovalClaim.from_json(data["approval"])
        return cls(
            approval,
            PromotionActor(
                as_string(data["actor_login"]),
                approval.actor_id,
                ActorKind(as_string(data["actor_type"])),
            ),
            Timestamp.parse(as_string(data["created_at"])),
        )


@dataclass(frozen=True, slots=True)
class PrivateApprovalReceipt:
    claim: PrivateApprovalReceiptClaim
    comment_id: int

    def __post_init__(self) -> None:
        as_positive_integer(self.comment_id)

    @property
    def reference(self) -> PrivateApprovalReference:
        approval = self.claim.approval
        return PrivateApprovalReference(
            approval.source,
            approval.head,
            approval.event_id,
            approval.ready_event_id,
            self.comment_id,
        )


class PrivateApprovalPolicyReader(Protocol):
    def get_promotion_pull_request(
        self, scope: PromotionScope
    ) -> PromotionPullRequest: ...

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]: ...

    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission: ...


class PrivateApprovalReader(PrivateApprovalPolicyReader, Protocol):
    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]: ...

    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None: ...


class PrivateApprovalRecorder(Protocol):
    def create_approval_receipt(
        self, approval: PromotionApproval
    ) -> PromotionComment: ...

    def create_promotion_reminder(
        self, scope: PromotionScope, ready_event_id: int
    ) -> PromotionComment: ...


class PrivatePromotionDispatcher(Protocol):
    def dispatch_private_promotion(
        self, reference: PrivateApprovalReference
    ) -> WorkflowDispatch: ...


def _require_ready(
    pull_request: PromotionPullRequest, source: PromotionScope, head: CommitSha
) -> None:
    if pull_request.scope != source:
        raise ValueError("GitHub returned a different private pull request")
    if (
        not pull_request.is_open
        or pull_request.draft
        or not pull_request.same_repository
    ):
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.NOT_READY)
    if pull_request.details.head.sha != head:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.HEAD_CHANGED)


def _require_label_approval(approval: PromotionApproval) -> LabelAddedEvent:
    _require_private_source(approval.source)
    match approval.event:
        case LabelAddedEvent() as event if event.label == PRIVATE_PROMOTION_LABEL:
            return event
        case ReadyForReviewEvent() | LabelAddedEvent():
            raise ValueError("Expected a private bot:promote label approval")
    assert_never(approval.event)


def select_private_approval(
    github: PrivateApprovalPolicyReader,
    source: PromotionScope,
    head: CommitSha,
    *,
    approval_id: int | None = None,
    ready_event_id: int | None = None,
) -> PromotionApproval:
    """Select current label authority without inventing an approved revision."""
    _require_private_source(source)
    pull_request = github.get_promotion_pull_request(source)
    _require_ready(pull_request, source, head)
    if PRIVATE_PROMOTION_LABEL not in pull_request.details.labels:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.LABEL_MISSING)

    events = github.list_promotion_events(source)
    ready = current_ready_event(events)
    if ready is None:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.NO_READINESS)
    event = latest_label_event(events, PRIVATE_PROMOTION_LABEL)
    if not isinstance(event, LabelAddedEvent):
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.LABEL_NOT_CURRENT)
    if event.identifier <= ready.identifier:
        raise PrivateApprovalUnavailable(
            ApprovalUnavailableReason.LABEL_PRECEDES_READINESS
        )
    if event.actor is None or not event.actor.is_human:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVER_NOT_HUMAN)
    if not github.get_repository_permission(source.repository, event.actor).can_write:
        raise PrivateApprovalUnavailable(
            ApprovalUnavailableReason.APPROVER_CANNOT_WRITE
        )

    approval = PromotionApproval(source, head, event, ready.identifier)
    if (approval_id is not None and approval_id != approval.event_id) or (
        ready_event_id is not None and ready_event_id != approval.ready_event_id
    ):
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVAL_CHANGED)
    return approval


def _correlate_request_event(
    request: PrivatePromotionRequest,
    events: tuple[PromotionEvent, ...],
) -> ReadyForReviewEvent | LabelAddedEvent:
    def require_unique_current[Event: ReadyForReviewEvent | LabelAddedEvent](
        candidates: tuple[Event, ...], current: PromotionEvent | None
    ) -> Event:
        if len(candidates) != 1 or candidates[0] != current:
            raise PrivateApprovalUnavailable(
                ApprovalUnavailableReason.EVENT_NOT_CORRELATED
            )
        return candidates[0]

    ready = current_ready_event(events)
    if ready is None:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.NO_READINESS)
    match request.kind:
        case PromotionApprovalKind.READY_FOR_REVIEW:
            return require_unique_current(
                tuple(
                    event
                    for event in events
                    if isinstance(event, ReadyForReviewEvent)
                    and event.actor == request.sender
                    and event.created_at == request.original_updated_at
                ),
                ready,
            )
        case PromotionApprovalKind.LABELED:
            return require_unique_current(
                tuple(
                    event
                    for event in events
                    if isinstance(event, LabelAddedEvent)
                    and event.label == PRIVATE_PROMOTION_LABEL
                    and event.actor == request.sender
                    and event.created_at == request.original_updated_at
                ),
                latest_label_event(events, PRIVATE_PROMOTION_LABEL),
            )
    assert_never(request.kind)


def _receipt_body(claim: PrivateApprovalReceiptClaim) -> str:
    encoded = json.dumps(
        claim.to_json(), sort_keys=True, separators=(",", ":"), allow_nan=False
    )
    return (
        f"{RECEIPT_TEXT_PREFIX}{claim.approval.head}`.\n\n"
        f"{RECEIPT_MARKER}{encoded}{RECEIPT_SUFFIX}"
    )


def private_approval_receipt_body(approval: PromotionApproval) -> str:
    return _receipt_body(PrivateApprovalReceiptClaim.from_approval(approval))


def private_approval_receipt(
    comment: PromotionComment,
) -> PrivateApprovalReceipt | None:
    """Decode an exact app-authored receipt; its contents remain mutable."""
    if not comment.is_automation or not comment.body.startswith(RECEIPT_TEXT_PREFIX):
        return None
    _, separator, encoded = comment.body.partition(f"\n\n{RECEIPT_MARKER}")
    if not separator or not encoded.endswith(RECEIPT_SUFFIX):
        return None
    try:
        claim = PrivateApprovalReceiptClaim.from_json(
            loads(encoded[: -len(RECEIPT_SUFFIX)])
        )
        receipt = PrivateApprovalReceipt(claim, comment.identifier)
    except KeyError, TypeError, ValueError:
        return None
    if comment.scope != claim.approval.source or comment.body != _receipt_body(claim):
        return None
    return receipt


def promotion_reminder_body(ready_event_id: int) -> str:
    as_positive_integer(ready_event_id)
    return f"{REMINDER_TEXT} <!-- uv-security-promotion-ready:{ready_event_id} -->"


def _existing_receipt(
    reader: PrivateApprovalReader, approval: PromotionApproval
) -> PrivateApprovalReceipt | None:
    found: PrivateApprovalReceipt | None = None
    for comment in reader.list_promotion_comments(approval.source):
        if comment.scope != approval.source:
            raise ValueError("GitHub returned comments from a different pull request")
        receipt = private_approval_receipt(comment)
        if receipt is None or receipt.claim.approval.event_id != approval.event_id:
            continue
        verified = reader.get_unedited_promotion_comment(
            approval.source, comment.identifier
        )
        if verified is None or verified.comment != comment:
            # Mutable or inconsistent comments are not approval authority. In
            # particular, an edited old receipt cannot poison a fresh label.
            continue
        if not receipt.claim.matches(approval):
            raise PrivateApprovalUnavailable(ApprovalUnavailableReason.RECEIPT_CONFLICT)
        if found is None or receipt.comment_id < found.comment_id:
            found = receipt
    return found


@dataclass(frozen=True, slots=True)
class RecordedPrivateApproval:
    receipt: PrivateApprovalReceipt


@dataclass(frozen=True, slots=True)
class RecordedPromotionReminder:
    ready_event_id: int
    comment_id: int


@dataclass(frozen=True, slots=True)
class SkippedPrivatePromotionRequest:
    reason: ApprovalUnavailableReason


@dataclass(frozen=True, slots=True)
class InspectedPrivatePromotion:
    """Advisory current-event information, not a captured approval."""

    kind: PromotionApprovalKind
    event_id: int
    ready_event_id: int


type PrivatePromotionRequestResult = (
    RecordedPrivateApproval | RecordedPromotionReminder | SkippedPrivatePromotionRequest
)


def inspect_private_promotion(
    reader: PrivateApprovalPolicyReader,
    source: PromotionScope,
    head: CommitSha,
    kind: PromotionApprovalKind,
) -> InspectedPrivatePromotion | SkippedPrivatePromotionRequest:
    """Inspect current policy without capturing or publishing authority."""
    _require_private_source(source)
    try:
        match kind:
            case PromotionApprovalKind.READY_FOR_REVIEW:
                _require_ready(reader.get_promotion_pull_request(source), source, head)
                ready = current_ready_event(reader.list_promotion_events(source))
                if ready is None:
                    raise PrivateApprovalUnavailable(
                        ApprovalUnavailableReason.NO_READINESS
                    )
                return InspectedPrivatePromotion(
                    kind, ready.identifier, ready.identifier
                )
            case PromotionApprovalKind.LABELED:
                approval = select_private_approval(reader, source, head)
                return InspectedPrivatePromotion(
                    kind, approval.event_id, approval.ready_event_id
                )
        assert_never(kind)
    except PrivateApprovalUnavailable as error:
        return SkippedPrivatePromotionRequest(error.reason)


def _created_comment(
    reader: PrivateApprovalReader,
    source: PromotionScope,
    created: PromotionComment,
    body: str,
) -> UneditedPromotionComment | None:
    # A creation response can omit performed_via_github_app. The separate
    # strict read proves the issuer as well as the exact, never-edited body.
    if created.scope != source or created.body != body:
        return None
    verified = reader.get_unedited_promotion_comment(source, created.identifier)
    if (
        verified is None
        or verified.comment.scope != source
        or verified.comment.identifier != created.identifier
        or verified.comment.body != body
    ):
        return None
    return verified


def _record_reminder(
    reader: PrivateApprovalReader,
    writer: PrivateApprovalRecorder,
    request: PrivatePromotionRequest,
    event: ReadyForReviewEvent,
) -> RecordedPromotionReminder:
    body = promotion_reminder_body(event.identifier)
    for comment in reader.list_promotion_comments(request.source):
        if comment.scope != request.source:
            raise ValueError("GitHub returned a different pull request comment")
        if comment.is_automation and comment.body == body:
            return RecordedPromotionReminder(event.identifier, comment.identifier)
    _require_ready(
        reader.get_promotion_pull_request(request.source), request.source, request.head
    )
    if (
        _correlate_request_event(request, reader.list_promotion_events(request.source))
        != event
    ):
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVAL_CHANGED)
    created = writer.create_promotion_reminder(request.source, event.identifier)
    verified = _created_comment(reader, request.source, created, body)
    if verified is None:
        raise ValueError("GitHub returned an unexpected promotion reminder")
    return RecordedPromotionReminder(event.identifier, verified.comment.identifier)


def _record_label_approval(
    reader: PrivateApprovalReader,
    writer: PrivateApprovalRecorder,
    request: PrivatePromotionRequest,
    event: LabelAddedEvent,
) -> RecordedPrivateApproval:
    approval = select_private_approval(
        reader, request.source, request.head, approval_id=event.identifier
    )
    if approval.event != event:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVAL_CHANGED)
    receipt = _existing_receipt(reader, approval)
    if receipt is not None:
        return RecordedPrivateApproval(receipt)
    current = select_private_approval(
        reader,
        request.source,
        request.head,
        approval_id=approval.event_id,
        ready_event_id=approval.ready_event_id,
    )
    if current != approval:
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVAL_CHANGED)
    created = writer.create_approval_receipt(approval)
    verified = _created_comment(
        reader, approval.source, created, private_approval_receipt_body(approval)
    )
    receipt = (
        private_approval_receipt(verified.comment) if verified is not None else None
    )
    if receipt is None or not receipt.claim.matches(approval):
        raise ValueError("GitHub returned an unexpected private approval receipt")
    return RecordedPrivateApproval(receipt)


def _record_private_request(
    reader: PrivateApprovalReader,
    writer: PrivateApprovalRecorder,
    request: PrivatePromotionRequest,
) -> RecordedPrivateApproval | RecordedPromotionReminder:
    _require_ready(
        reader.get_promotion_pull_request(request.source), request.source, request.head
    )
    event = _correlate_request_event(
        request, reader.list_promotion_events(request.source)
    )
    match event:
        case ReadyForReviewEvent():
            return _record_reminder(reader, writer, request, event)
        case LabelAddedEvent():
            return _record_label_approval(reader, writer, request, event)
    assert_never(event)


def record_private_request(
    reader: PrivateApprovalReader,
    writer: PrivateApprovalRecorder,
    request: PrivatePromotionRequest,
) -> PrivatePromotionRequestResult:
    """The proposed capture stage; no live writer or CLI is wired yet."""
    try:
        return _record_private_request(reader, writer, request)
    except PrivateApprovalUnavailable as error:
        return SkippedPrivatePromotionRequest(error.reason)


def verify_private_approval(
    reader: PrivateApprovalReader, reference: PrivateApprovalReference
) -> PromotionApproval:
    """Re-fetch the exact receipt, current label event, head, and permission."""
    verified = reader.get_unedited_promotion_comment(
        reference.source, reference.receipt_id
    )
    receipt = (
        private_approval_receipt(verified.comment) if verified is not None else None
    )
    if receipt is None or receipt.reference != reference:
        raise ValueError("The exact private approval receipt could not be verified")
    approval = select_private_approval(
        reader,
        reference.source,
        reference.head,
        approval_id=reference.approval_id,
        ready_event_id=reference.ready_event_id,
    )
    if not receipt.claim.matches(approval):
        raise PrivateApprovalUnavailable(ApprovalUnavailableReason.APPROVAL_CHANGED)
    return approval


def dispatch_private_promotion(
    reader: PrivateApprovalReader,
    writer: PrivatePromotionDispatcher,
    reference: PrivateApprovalReference,
) -> WorkflowDispatch:
    """The proposed dispatch stage; the publisher must verify the receipt again."""
    verify_private_approval(reader, reference)
    result = writer.dispatch_private_promotion(reference)
    if result.repository != reference.source.repository:
        raise ValueError("GitHub returned a different private promotion dispatch")
    return result
