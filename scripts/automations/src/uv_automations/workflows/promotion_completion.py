"""Bounded completion of an already-created, exact promotion."""

import re
from dataclasses import dataclass
from enum import StrEnum
from time import sleep
from typing import Protocol

from uv_automations.github_promotion import PromotionReader, PromotionReadError
from uv_automations.json import as_object, as_positive_integer, as_string, require_keys
from uv_automations.models import CommitSha, PullRequestState
from uv_automations.promotion_models import (
    PROMOTION_LABEL,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    BranchRevision,
    PromotionApproval,
    PromotionApprovalKind,
    PromotionPullRequest,
    PromotionScope,
    check_promotion_branch,
    current_promotion_approval,
    require_promotion_source,
    unique_promotion_record,
)

COMPLETION_ATTEMPTS = 3
MAX_COMPLETION_RETRY_SECONDS = 30


class CompletionRequestError(PromotionReadError):
    """Sanitized response metadata for the narrowly scoped completion writes."""

    def __init__(self, status: int | None, retry_after: float | None = None) -> None:
        super().__init__("GitHub promotion completion request failed")
        self.status = status
        self.retry_after = retry_after
        self.retryable = (
            status is None
            or (status is not None and 200 <= status <= 299)
            or status in {404, 408, 429}
            or (status == 403 and retry_after is not None)
            or (status is not None and 500 <= status <= 599)
        ) and (retry_after is None or retry_after <= MAX_COMPLETION_RETRY_SECONDS)


class PublicationAction(StrEnum):
    PROMOTE = "promote"
    UPDATE_PARENT = "update-parent"


@dataclass(frozen=True, slots=True)
class PromotionCompletion:
    """Claimed publication identity; every write checks the live objects again."""

    source: PromotionScope
    source_base: BranchRevision
    source_head: BranchRevision
    action: PublicationAction
    approval_id: int
    approval_kind: PromotionApprovalKind
    replay_approval_id: int | None
    upstream: PromotionScope
    upstream_base: str
    promoted_head: CommitSha

    def __post_init__(self) -> None:
        require_promotion_source(self.source.repository)
        as_positive_integer(self.approval_id)
        check_promotion_branch(self.upstream_base)
        if self.replay_approval_id is not None:
            as_positive_integer(self.replay_approval_id)
        if (
            self.source_base.repository != self.source.repository
            or self.source_head.repository != self.source.repository
            or self.upstream.repository != UV_REPOSITORY
            or self.replay_approval_id not in (None, self.approval_id)
            or (
                self.action == PublicationAction.PROMOTE
                and self.promoted_head != self.source_head.sha
            )
        ):
            raise ValueError("Inconsistent promotion completion identity")

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "source": self.source.to_json(),
            "source_base": self.source_base.to_json(),
            "source_head": self.source_head.to_json(),
            "action": self.action.value,
            "approval_id": self.approval_id,
            "approval_kind": self.approval_kind.value,
            "replay_approval_id": self.replay_approval_id,
            "upstream": self.upstream.to_json(),
            "upstream_base": self.upstream_base,
            "promoted_head": str(self.promoted_head),
        }

    @classmethod
    def from_json(cls, value: object) -> PromotionCompletion:
        data = as_object(value)
        require_keys(
            data,
            {
                "version",
                "source",
                "source_base",
                "source_head",
                "action",
                "approval_id",
                "approval_kind",
                "replay_approval_id",
                "upstream",
                "upstream_base",
                "promoted_head",
            },
        )
        if as_positive_integer(data["version"]) != 1:
            raise ValueError("Unsupported promotion completion version")
        return cls(
            PromotionScope.from_json(data["source"]),
            BranchRevision.from_json(data["source_base"]),
            BranchRevision.from_json(data["source_head"]),
            PublicationAction(as_string(data["action"])),
            as_positive_integer(data["approval_id"]),
            PromotionApprovalKind(as_string(data["approval_kind"])),
            as_positive_integer(data["replay_approval_id"])
            if data["replay_approval_id"] is not None
            else None,
            PromotionScope.from_json(data["upstream"]),
            as_string(data["upstream_base"]),
            CommitSha(as_string(data["promoted_head"])),
        )

    @property
    def comment(self) -> str:
        number = self.upstream.number
        return (
            f"Promoted to [#{number}](https://github.com/astral-sh/uv/pull/{number})."
        )


class CompletionWriter(Protocol):
    def add_promotion_labels(
        self, upstream: PromotionScope, labels: tuple[str, ...]
    ) -> None: ...

    def assign_promotion(self, upstream: PromotionScope, login: str) -> None: ...

    def record_promotion(self, completion: PromotionCompletion) -> None: ...

    def close_promotion_source(self, source: PromotionScope) -> None: ...


@dataclass(frozen=True, slots=True)
class SkippedCompletion:
    reason: str
    changed_head: CommitSha | None = None


@dataclass(frozen=True, slots=True)
class _CurrentCompletion:
    source: PromotionPullRequest
    approval: PromotionApproval


def _current(
    reader: PromotionReader,
    completion: PromotionCompletion,
    *,
    allow_closed: bool = False,
) -> _CurrentCompletion | SkippedCompletion:
    source = reader.get_promotion_pull_request(completion.source)
    if source.details.head.sha != completion.source_head.sha:
        return SkippedCompletion(
            "The source head changed after publication.", source.details.head.sha
        )
    if (
        source.scope != completion.source
        or not source.same_repository
        or source.details.base.ref != completion.source_base.ref
        or source.details.head.ref != completion.source_head.ref
        or source.merge is not None
        or (not source.is_open and not allow_closed)
        or (
            (
                completion.action == PublicationAction.UPDATE_PARENT
                or source.scope.repository == UV_SECURITY_REPOSITORY
            )
            and source.details.base.sha != completion.source_base.sha
        )
        or (source.draft and completion.approval_kind != PromotionApprovalKind.LABELED)
        or (
            completion.approval_kind == PromotionApprovalKind.LABELED
            and PROMOTION_LABEL not in source.details.labels
        )
    ):
        return SkippedCompletion("The source changed after publication.")
    approval = current_promotion_approval(
        source,
        completion.source_head.sha,
        reader.list_promotion_events(completion.source),
        exact_readiness=completion.replay_approval_id is not None,
    )
    if (
        approval is None
        or approval.event_id != completion.approval_id
        or approval.kind != completion.approval_kind
    ):
        return SkippedCompletion("The promotion approval changed after publication.")
    if source.scope.repository == UV_SECURITY_REPOSITORY and (
        PROMOTION_LABEL not in source.details.labels
        or not reader.get_repository_permission(
            source.scope.repository, approval.actor
        ).can_write
    ):
        return SkippedCompletion("The private promotion approval changed.")
    upstream = reader.get_promotion_pull_request(completion.upstream)
    if (
        upstream.scope != completion.upstream
        or not upstream.is_open
        or not upstream.same_repository
        or upstream.details.base.ref != completion.upstream_base
        or upstream.details.head.ref != completion.source_head.ref
        or upstream.details.head.sha != completion.promoted_head
    ):
        return SkippedCompletion("The upstream promotion changed after publication.")
    return _CurrentCompletion(source, approval)


def _retry_delay(
    attempt: int,
    error: PromotionReadError | None = None,
    *,
    before_reconciliation: bool = False,
) -> None:
    delay = float(2**attempt)
    if isinstance(error, CompletionRequestError):
        if not error.retryable:
            raise error
        if error.retry_after is not None:
            delay = max(delay, error.retry_after)
    if before_reconciliation or attempt + 1 < COMPLETION_ATTEMPTS:
        sleep(delay)


def complete_metadata(
    reader: PromotionReader, writer: CompletionWriter, completion: PromotionCompletion
) -> SkippedCompletion | None:
    for attempt in range(COMPLETION_ATTEMPTS):
        try:
            current = _current(reader, completion)
            if isinstance(current, SkippedCompletion):
                return current
            labels = tuple(
                label
                for label in current.source.details.labels
                if re.match(r"^(bot|priority|size|risk|severity):", label) is None
            )
            if labels:
                writer.add_promotion_labels(completion.upstream, labels)
            current = _current(reader, completion)
            if isinstance(current, SkippedCompletion):
                return current
            writer.assign_promotion(completion.upstream, current.approval.actor.login)
            return None
        except PromotionReadError as error:
            # Both endpoints add values to a set, including after a lost response.
            if attempt + 1 == COMPLETION_ATTEMPTS:
                raise
            _retry_delay(attempt, error)
    raise AssertionError("The completion attempt budget is empty")


def _recorded(
    reader: PromotionReader, completion: PromotionCompletion
) -> bool | SkippedCompletion:
    record = unique_promotion_record(
        completion.source, reader.list_promotion_comments(completion.source)
    )
    if record is not None and record.upstream != completion.upstream:
        return SkippedCompletion("The source records a different promotion.")
    return record is not None


def close_source(
    reader: PromotionReader, writer: CompletionWriter, completion: PromotionCompletion
) -> SkippedCompletion | None:
    # A comment is not an idempotent write. Once attempted, reconcile its stable
    # canonical receipt; never repeat an ambiguous POST in this invocation.
    comment_attempted = False
    for attempt in range(COMPLETION_ATTEMPTS):
        waited = False
        try:
            current = _current(reader, completion)
            if isinstance(current, SkippedCompletion):
                return current
            recorded = _recorded(reader, completion)
            if isinstance(recorded, SkippedCompletion):
                return recorded
            if recorded:
                break
            if not comment_attempted:
                comment_attempted = True
                try:
                    writer.record_promotion(completion)
                except PromotionReadError as error:
                    # A server-requested delay applies to reconciliation reads
                    # too, including a final attempt with an ambiguous result.
                    _retry_delay(attempt, error, before_reconciliation=True)
                    waited = True
                current = _current(reader, completion)
                if isinstance(current, SkippedCompletion):
                    return current
                recorded = _recorded(reader, completion)
                if isinstance(recorded, SkippedCompletion):
                    return recorded
                if recorded:
                    break
        except PromotionReadError as error:
            if attempt + 1 == COMPLETION_ATTEMPTS:
                raise
            _retry_delay(attempt, error)
            continue
        if not waited:
            _retry_delay(attempt)
    else:
        raise PromotionReadError("Could not confirm the promotion comment")

    close_attempted = False
    for attempt in range(COMPLETION_ATTEMPTS):
        waited = False
        try:
            current = _current(reader, completion, allow_closed=close_attempted)
            if isinstance(current, SkippedCompletion):
                return current
            recorded = _recorded(reader, completion)
            if isinstance(recorded, SkippedCompletion):
                return recorded
            if not recorded:
                return SkippedCompletion("The promotion comment is no longer present.")
            if current.source.details.state == PullRequestState.CLOSED:
                return None
            close_attempted = True
            try:
                writer.close_promotion_source(completion.source)
            except PromotionReadError as error:
                _retry_delay(attempt, error, before_reconciliation=True)
                waited = True
            current = _current(reader, completion, allow_closed=True)
            if isinstance(current, SkippedCompletion):
                return current
            if current.source.details.state == PullRequestState.CLOSED:
                recorded = _recorded(reader, completion)
                if isinstance(recorded, SkippedCompletion):
                    return recorded
                if recorded:
                    return None
        except PromotionReadError as error:
            if attempt + 1 == COMPLETION_ATTEMPTS:
                raise
            _retry_delay(attempt, error)
            continue
        if not waited:
            _retry_delay(attempt)
    raise PromotionReadError("Could not confirm the source pull request closed")
