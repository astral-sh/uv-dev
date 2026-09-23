"""The persisted and agent-facing contracts for pull request feedback."""

import hashlib
import html
import json
import re
from dataclasses import dataclass, replace
from enum import StrEnum
from typing import assert_never

from uv_automations.json import (
    as_array,
    as_object,
    as_positive_integer,
    as_string,
    require_keys,
)
from uv_automations.models import (
    ActorKind,
    CommitSha,
    PullRequestRef,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

MAX_ACTIONS = 20
MAX_BODY_LENGTH = 12_000
MAX_SUMMARY_LENGTH = 4_000
MAX_PAGE_SIZE = 100
MAX_COLLECTION_PAGES = 10
MAX_THREAD_COMMENTS = 100
MAX_SELECTED_THREADS = 100
MAX_KNOWN_THREADS = MAX_PAGE_SIZE * MAX_COLLECTION_PAGES
MAX_KNOWN_REVIEWS = MAX_PAGE_SIZE * MAX_COLLECTION_PAGES
AUTOMATION_LOGINS = frozenset({"astral-automations-bot", "astral-automations-bot[bot]"})


@dataclass(frozen=True, slots=True)
class CommentScope:
    repository: RepositoryIdentity
    number: int

    def __post_init__(self) -> None:
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
    def from_json(cls, value: object) -> CommentScope:
        data = as_object(value)
        require_keys(data, {"repository", "repository_id", "pull_request"})
        return cls(
            RepositoryIdentity(
                RepositoryName(as_string(data["repository"])),
                as_positive_integer(data["repository_id"]),
            ),
            as_positive_integer(data["pull_request"]),
        )


class AuthorAssociation(StrEnum):
    COLLABORATOR = "COLLABORATOR"
    CONTRIBUTOR = "CONTRIBUTOR"
    FIRST_TIMER = "FIRST_TIMER"
    FIRST_TIME_CONTRIBUTOR = "FIRST_TIME_CONTRIBUTOR"
    MANNEQUIN = "MANNEQUIN"
    MEMBER = "MEMBER"
    NONE = "NONE"
    OWNER = "OWNER"


@dataclass(frozen=True, slots=True)
class CommentAuthor:
    login: str | None
    kind: ActorKind | None
    association: AuthorAssociation

    @property
    def is_automation(self) -> bool:
        return self.login in AUTOMATION_LOGINS

    @property
    def is_publisher(self) -> bool:
        return self.kind == ActorKind.BOT and self.is_automation

    @property
    def can_trigger(self) -> bool:
        if self.kind != ActorKind.USER or self.login is None or self.is_automation:
            return False
        match self.association:
            case (
                AuthorAssociation.OWNER
                | AuthorAssociation.MEMBER
                | AuthorAssociation.COLLABORATOR
            ):
                return True
            case (
                AuthorAssociation.CONTRIBUTOR
                | AuthorAssociation.FIRST_TIMER
                | AuthorAssociation.FIRST_TIME_CONTRIBUTOR
                | AuthorAssociation.MANNEQUIN
                | AuthorAssociation.NONE
            ):
                return False
        assert_never(self.association)

    def to_json(self) -> dict[str, object]:
        return {
            "login": self.login,
            "type": self.kind,
            "association": self.association,
        }

    @classmethod
    def from_json(cls, value: object) -> CommentAuthor:
        data = as_object(value)
        require_keys(data, {"login", "type", "association"})
        return cls(
            as_string(data["login"]) if data["login"] is not None else None,
            ActorKind(as_string(data["type"])) if data["type"] is not None else None,
            AuthorAssociation(as_string(data["association"])),
        )


class CommentTargetKind(StrEnum):
    CONVERSATION_COMMENT = "CONVERSATION_COMMENT"
    REVIEW_THREAD = "REVIEW_THREAD"
    PULL_REQUEST_REVIEW = "PULL_REQUEST_REVIEW"


@dataclass(frozen=True, slots=True)
class CommentTarget:
    kind: CommentTargetKind
    identifier: str

    def __post_init__(self) -> None:
        if not 1 <= len(self.identifier) <= 200:
            raise ValueError("Invalid GitHub feedback target ID length")
        match self.kind:
            case (
                CommentTargetKind.CONVERSATION_COMMENT
                | CommentTargetKind.PULL_REQUEST_REVIEW
            ):
                if re.fullmatch(r"[1-9][0-9]*", self.identifier) is None:
                    raise ValueError("Expected a numeric GitHub comment or review ID")
                return
            case CommentTargetKind.REVIEW_THREAD:
                if re.fullmatch(r"[A-Za-z0-9_+/=-]{1,200}", self.identifier) is None:
                    raise ValueError("Expected a GitHub review thread node ID")
                return
        assert_never(self.kind)

    def to_json(self) -> dict[str, object]:
        return {"target": self.kind, "id": self.identifier}


def fingerprint(value: object) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()
    return hashlib.sha256(encoded).hexdigest()


@dataclass(frozen=True, slots=True)
class TargetRevision:
    target: CommentTarget
    digest: str

    def __post_init__(self) -> None:
        if re.fullmatch(r"[0-9a-f]{64}", self.digest) is None:
            raise ValueError("Expected a SHA-256 feedback digest")

    def to_json(self) -> dict[str, object]:
        return {**self.target.to_json(), "digest": self.digest}

    @classmethod
    def from_json(cls, value: object) -> TargetRevision:
        data = as_object(value)
        require_keys(data, {"target", "id", "digest"})
        return cls(
            CommentTarget(
                CommentTargetKind(as_string(data["target"])), as_string(data["id"])
            ),
            as_string(data["digest"]),
        )


class CommentRecordKind(StrEnum):
    CONVERSATION = "conversation"
    INLINE = "inline"


@dataclass(frozen=True, slots=True)
class CommentVersion:
    kind: CommentRecordKind
    identifier: int
    digest: str

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        if re.fullmatch(r"[0-9a-f]{64}", self.digest) is None:
            raise ValueError("Expected a SHA-256 comment digest")

    def to_json(self) -> dict[str, object]:
        return {"kind": self.kind, "id": self.identifier, "digest": self.digest}

    @classmethod
    def from_json(cls, value: object) -> CommentVersion:
        data = as_object(value)
        require_keys(data, {"kind", "id", "digest"})
        return cls(
            CommentRecordKind(as_string(data["kind"])),
            as_positive_integer(data["id"]),
            as_string(data["digest"]),
        )


@dataclass(frozen=True, slots=True)
class ConversationComment:
    identifier: int
    author: CommentAuthor
    body: str
    updated_at: Timestamp

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)

    def to_json(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "author": self.author.to_json(),
            "body": self.body,
            "updated_at": str(self.updated_at),
        }

    @property
    def revision(self) -> TargetRevision:
        return TargetRevision(
            CommentTarget(CommentTargetKind.CONVERSATION_COMMENT, str(self.identifier)),
            fingerprint(self.to_json()),
        )

    @property
    def version(self) -> CommentVersion:
        return CommentVersion(
            CommentRecordKind.CONVERSATION, self.identifier, self.revision.digest
        )


@dataclass(frozen=True, slots=True)
class InlineComment:
    identifier: int
    root_identifier: int
    author: CommentAuthor
    body: str
    updated_at: Timestamp
    path: str
    diff_hunk: str

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        as_positive_integer(self.root_identifier)

    def to_json(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "root_id": self.root_identifier,
            "author": self.author.to_json(),
            "body": self.body,
            "updated_at": str(self.updated_at),
            "path": self.path,
            "diff_hunk": self.diff_hunk,
        }

    @property
    def version(self) -> CommentVersion:
        return CommentVersion(
            CommentRecordKind.INLINE, self.identifier, fingerprint(self.to_json())
        )


class ReviewState(StrEnum):
    PENDING = "PENDING"
    COMMENTED = "COMMENTED"
    APPROVED = "APPROVED"
    CHANGES_REQUESTED = "CHANGES_REQUESTED"
    DISMISSED = "DISMISSED"


@dataclass(frozen=True, slots=True)
class SubmittedReview:
    identifier: int
    author: CommentAuthor
    body: str
    updated_at: Timestamp
    state: ReviewState

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)

    @property
    def is_submitted(self) -> bool:
        match self.state:
            case ReviewState.PENDING:
                return False
            case (
                ReviewState.COMMENTED
                | ReviewState.APPROVED
                | ReviewState.CHANGES_REQUESTED
                | ReviewState.DISMISSED
            ):
                return True
        assert_never(self.state)

    def to_json(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "author": self.author.to_json(),
            "body": self.body,
            "updated_at": str(self.updated_at),
            "state": self.state,
        }

    @property
    def revision(self) -> TargetRevision:
        return TargetRevision(
            CommentTarget(CommentTargetKind.PULL_REQUEST_REVIEW, str(self.identifier)),
            fingerprint(self.to_json()),
        )


@dataclass(frozen=True, slots=True)
class ThreadComment:
    identifier: int
    author: CommentAuthor
    body: str
    updated_at: Timestamp

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)

    def to_json(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "author": self.author.to_json(),
            "body": self.body,
            "updated_at": str(self.updated_at),
        }


@dataclass(frozen=True, slots=True)
class ReviewThread:
    identifier: str
    resolved: bool
    outdated: bool
    path: str
    comments: tuple[ThreadComment, ...]

    def __post_init__(self) -> None:
        CommentTarget(CommentTargetKind.REVIEW_THREAD, self.identifier)
        if not self.comments or len(self.comments) > MAX_THREAD_COMMENTS:
            raise ValueError("Expected one complete, bounded review thread")
        if len({comment.identifier for comment in self.comments}) != len(self.comments):
            raise ValueError("Duplicate comment in review thread")

    def to_json(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "is_resolved": self.resolved,
            "is_outdated": self.outdated,
            "path": self.path,
            "comments": [comment.to_json() for comment in self.comments],
        }

    @property
    def revision(self) -> TargetRevision:
        # Pushing the addressing commit can update anchors/outdatedness, and our
        # own replies must not invalidate a retry. Human discussion is the precondition.
        return TargetRevision(
            CommentTarget(CommentTargetKind.REVIEW_THREAD, self.identifier),
            fingerprint(
                [
                    comment.to_json()
                    for comment in self.comments
                    if not comment.author.is_automation
                ]
            ),
        )


@dataclass(frozen=True, slots=True)
class ThreadRoot:
    comment_id: int
    thread_id: str

    def __post_init__(self) -> None:
        as_positive_integer(self.comment_id)
        CommentTarget(CommentTargetKind.REVIEW_THREAD, self.thread_id)

    def to_json(self) -> dict[str, object]:
        return {"comment_id": self.comment_id, "thread_id": self.thread_id}

    @classmethod
    def from_json(cls, value: object) -> ThreadRoot:
        data = as_object(value)
        require_keys(data, {"comment_id", "thread_id"})
        return cls(
            as_positive_integer(data["comment_id"]), as_string(data["thread_id"])
        )


@dataclass(frozen=True, slots=True)
class ThreadPage:
    roots: tuple[ThreadRoot, ...]
    cursor: str | None
    has_next_page: bool

    def __post_init__(self) -> None:
        if len(self.roots) > MAX_PAGE_SIZE:
            raise ValueError("GitHub returned an oversized review-thread page")
        if len({root.comment_id for root in self.roots}) != len(self.roots):
            raise ValueError("Duplicate review-thread root in one page")
        if self.has_next_page and not self.cursor:
            raise ValueError("An incomplete review-thread page needs a cursor")


@dataclass(frozen=True, slots=True)
class CollectionCheckpoint:
    through: Timestamp
    thread_cursor: str | None
    threads: tuple[ThreadRoot, ...]
    reviews: tuple[TargetRevision, ...]
    comments: tuple[CommentVersion, ...] = ()

    def __post_init__(self) -> None:
        if len(self.threads) > MAX_KNOWN_THREADS:
            raise ValueError("Too many retained review threads")
        if len(self.reviews) > MAX_KNOWN_REVIEWS:
            raise ValueError("Too many retained pull request reviews")
        if len(self.comments) > 2 * MAX_PAGE_SIZE * MAX_COLLECTION_PAGES:
            raise ValueError("Too many comments at the collection watermark")
        if self.thread_cursor is not None and (
            not self.thread_cursor or len(self.thread_cursor) > 4096
        ):
            raise ValueError("Invalid retained review-thread cursor")
        if len({thread.comment_id for thread in self.threads}) != len(self.threads):
            raise ValueError("Duplicate review-thread root comment")
        if len({thread.thread_id for thread in self.threads}) != len(self.threads):
            raise ValueError("Duplicate review thread")
        if len({review.target for review in self.reviews}) != len(self.reviews):
            raise ValueError("Duplicate pull request review")
        if len(
            {(comment.kind, comment.identifier) for comment in self.comments}
        ) != len(self.comments):
            raise ValueError("Duplicate comment at the collection watermark")
        if any(
            review.target.kind != CommentTargetKind.PULL_REQUEST_REVIEW
            for review in self.reviews
        ):
            raise ValueError("Expected pull request review revisions")

    def to_json(self) -> dict[str, object]:
        return {
            "complete": True,
            "through": str(self.through),
            "thread_cursor": self.thread_cursor,
            "threads": [thread.to_json() for thread in self.threads],
            "reviews": [review.to_json() for review in self.reviews],
            "comments": [comment.to_json() for comment in self.comments],
        }

    @classmethod
    def from_json(cls, value: object) -> CollectionCheckpoint:
        data = as_object(value)
        require_keys(
            data,
            {"complete", "through", "thread_cursor", "threads", "reviews", "comments"},
        )
        if data["complete"] is not True:
            raise ValueError("A truncated collection is not a usable checkpoint")
        return cls(
            through=Timestamp.parse(as_string(data["through"])),
            thread_cursor=(
                as_string(data["thread_cursor"])
                if data["thread_cursor"] is not None
                else None
            ),
            threads=tuple(
                ThreadRoot.from_json(value) for value in as_array(data["threads"])
            ),
            reviews=tuple(
                TargetRevision.from_json(value) for value in as_array(data["reviews"])
            ),
            comments=tuple(
                CommentVersion.from_json(value) for value in as_array(data["comments"])
            ),
        )


@dataclass(frozen=True, slots=True)
class CollectedComments:
    conversation: tuple[ConversationComment, ...]
    inline: tuple[InlineComment, ...]
    reviews: tuple[SubmittedReview, ...]
    threads: tuple[ReviewThread, ...]
    checkpoint: CollectionCheckpoint
    targets: tuple[TargetRevision, ...]

    def to_json(self) -> dict[str, object]:
        return {
            "conversation_comments": [
                comment.to_json() for comment in self.conversation
            ],
            "inline_comments": [comment.to_json() for comment in self.inline],
            "reviews": [review.to_json() for review in self.reviews],
            "review_threads": [thread.to_json() for thread in self.threads],
            "actionable_targets": [target.to_json() for target in self.targets],
        }


class CommentOutcome(StrEnum):
    COMMIT_AND_RESOLVE = "COMMIT_AND_RESOLVE"
    RESPOND = "RESPOND"
    CLARIFY = "CLARIFY"


@dataclass(frozen=True, slots=True)
class CommentAction:
    target: CommentTarget
    outcome: CommentOutcome
    body: str
    addressing_commit: CommitSha | None

    def __post_init__(self) -> None:
        if len(self.body) > MAX_BODY_LENGTH:
            raise ValueError("A generated comment body is too long")
        match self.outcome:
            case CommentOutcome.COMMIT_AND_RESOLVE:
                if self.addressing_commit is None:
                    raise ValueError("Addressed feedback must identify its new commit")
                match self.target.kind:
                    case CommentTargetKind.REVIEW_THREAD:
                        if self.body.strip():
                            raise ValueError(
                                "Resolving a review thread must not add a reply"
                            )
                        return
                    case (
                        CommentTargetKind.CONVERSATION_COMMENT
                        | CommentTargetKind.PULL_REQUEST_REVIEW
                    ):
                        if not self.body.strip():
                            raise ValueError(
                                "A conversation or review response needs a body"
                            )
                        return
                assert_never(self.target.kind)
            case CommentOutcome.RESPOND | CommentOutcome.CLARIFY:
                if self.addressing_commit is not None or not self.body.strip():
                    raise ValueError("A reply needs a nonempty body and no commit")
                return
        assert_never(self.outcome)

    def to_json(self) -> dict[str, object]:
        return {
            **self.target.to_json(),
            "outcome": self.outcome,
            "body": self.body,
            "addressing_commit": (
                str(self.addressing_commit)
                if self.addressing_commit is not None
                else None
            ),
        }

    @classmethod
    def from_json(cls, value: object) -> CommentAction:
        data = as_object(value)
        require_keys(data, {"target", "id", "outcome", "body", "addressing_commit"})
        return cls(
            target=CommentTarget(
                CommentTargetKind(as_string(data["target"])), as_string(data["id"])
            ),
            outcome=CommentOutcome(as_string(data["outcome"])),
            body=as_string(data["body"]),
            addressing_commit=(
                CommitSha(as_string(data["addressing_commit"]))
                if data["addressing_commit"] is not None
                else None
            ),
        )


@dataclass(frozen=True, slots=True)
class CommentRecommendation:
    summary: str
    actions: tuple[CommentAction, ...]

    def __post_init__(self) -> None:
        if not self.summary.strip() or len(self.summary) > MAX_SUMMARY_LENGTH:
            raise ValueError("Expected a nonempty, bounded feedback summary")
        if len(self.actions) > MAX_ACTIONS:
            raise ValueError(f"Expected at most {MAX_ACTIONS} comment actions")
        if len({action.target for action in self.actions}) != len(self.actions):
            raise ValueError("A feedback target may have only one action")

    @classmethod
    def from_json(cls, value: object) -> CommentRecommendation:
        data = as_object(value)
        require_keys(data, {"summary", "actions"})
        return cls(
            as_string(data["summary"]),
            tuple(
                CommentAction.from_json(value) for value in as_array(data["actions"])
            ),
        )

    def to_json(self) -> dict[str, object]:
        return {
            "summary": self.summary,
            "actions": [action.to_json() for action in self.actions],
        }

    @staticmethod
    def schema() -> dict[str, object]:
        """Generate the Codex schema from the same enums and bounds as the decoder."""
        action_properties = {
            "target": {"type": "string", "enum": list(CommentTargetKind)},
            "id": {"type": "string", "minLength": 1, "maxLength": 200},
            "outcome": {"type": "string", "enum": list(CommentOutcome)},
            "body": {"type": "string", "maxLength": MAX_BODY_LENGTH},
            "addressing_commit": {
                "type": ["string", "null"],
                "pattern": "^[0-9a-f]{40}$",
            },
        }
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "summary": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_SUMMARY_LENGTH,
                },
                "actions": {
                    "type": "array",
                    "maxItems": MAX_ACTIONS,
                    "items": {
                        "type": "object",
                        "additionalProperties": False,
                        "properties": action_properties,
                        "required": list(action_properties),
                    },
                },
            },
            "required": ["summary", "actions"],
        }


def sanitize_comment_body(body: str) -> str:
    """Keep generated Markdown readable without notifications or hidden markers."""
    value = body.replace("\r\n", "\n").strip()
    # Decode entities before neutralizing mentions, including doubly encoded input.
    for _ in range(8):
        decoded = html.unescape(value)
        if decoded == value:
            break
        value = decoded
    else:
        raise ValueError("Generated comment contains excessively nested HTML entities")
    if any(
        (ord(character) < 32 and character not in "\n\t")
        or ord(character) == 127
        or character == "\u061c"
        or "\u200e" <= character <= "\u200f"
        or "\u202a" <= character <= "\u202e"
        or "\u2066" <= character <= "\u2069"
        for character in value
    ):
        raise ValueError("Generated comments cannot contain control characters")
    value = value.replace("<!--", "&lt;!--").replace("-->", "--&gt;")
    value = re.sub("@\u200b*", "@\u200b", value)
    if not value or len(value) > MAX_BODY_LENGTH:
        raise ValueError("Expected a nonempty, bounded generated comment")
    return value


def sanitize_recommendation(
    recommendation: CommentRecommendation,
) -> CommentRecommendation:
    return replace(
        recommendation,
        actions=tuple(
            replace(action, body=sanitize_comment_body(action.body))
            if action.body.strip()
            else replace(action, body="")
            for action in recommendation.actions
        ),
    )
