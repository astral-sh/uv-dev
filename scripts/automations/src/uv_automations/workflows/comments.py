"""Collect feedback, transport agent results, and publish under exact preconditions."""

import json
import logging
import os
import re
import stat
import subprocess
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Protocol, assert_never
from uuid import UUID

from uv_automations.artifacts import CommitRange, load_commit, persist_commit
from uv_automations.checkouts import inspect_candidate
from uv_automations.comment_models import (
    MAX_ACTIONS,
    MAX_COLLECTION_PAGES,
    MAX_KNOWN_THREADS,
    MAX_SELECTED_THREADS,
    CollectedComments,
    CollectionCheckpoint,
    CommentAction,
    CommentOutcome,
    CommentRecommendation,
    CommentScope,
    CommentTarget,
    CommentTargetKind,
    ConversationComment,
    InlineComment,
    ReviewThread,
    SubmittedReview,
    TargetRevision,
    ThreadPage,
    fingerprint,
    sanitize_recommendation,
)
from uv_automations.git import Git, check_branch
from uv_automations.github_actions import ActionsRun, ArtifactIdentity, WorkflowRun
from uv_automations.github_comments import CommentPullRequest
from uv_automations.json import (
    as_array,
    as_object,
    as_positive_integer,
    as_string,
    loads,
    require_keys,
)
from uv_automations.models import (
    CommitSha,
    ManagedRepository,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)
from uv_automations.sessions import (
    CodexSessionSnapshot,
    snapshot_sessions,
    write_sessions,
)

logger = logging.getLogger(__name__)

WORKFLOW = ".github/workflows/pull-request-comments.yml"
CHECKPOINT_DISCOVERY_RUNS = 20
MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_COMMITS = 25
BOT_NAME = "astral-automations-bot[bot]"
BOT_EMAIL = "305554984+astral-automations-bot[bot]@users.noreply.github.com"


class CommentReader(Protocol):
    def get_comment_pull_request(self, scope: CommentScope) -> CommentPullRequest: ...
    def list_conversation_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[ConversationComment, ...]: ...
    def list_inline_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[InlineComment, ...]: ...
    def list_reviews(self, scope: CommentScope) -> tuple[SubmittedReview, ...]: ...
    def list_thread_roots(
        self, scope: CommentScope, cursor: str | None
    ) -> ThreadPage: ...
    def get_review_threads(
        self, scope: CommentScope, identifiers: tuple[str, ...]
    ) -> tuple[ReviewThread, ...]: ...
    def get_conversation_comment(
        self, scope: CommentScope, identifier: int
    ) -> ConversationComment: ...
    def get_review(self, scope: CommentScope, identifier: int) -> SubmittedReview: ...


class CheckpointReader(Protocol):
    def list_successful_workflow_runs(
        self, repository: RepositoryName, workflow: str, *, limit: int
    ) -> tuple[WorkflowRun, ...]: ...
    def read_workflow_run(
        self, repository: RepositoryIdentity, identifier: int, attempt: int
    ) -> WorkflowRun: ...
    def get_workflow_run(self, source: ActionsRun) -> WorkflowRun: ...
    def find_artifact(
        self, source: ActionsRun, name: str
    ) -> ArtifactIdentity | None: ...
    def get_artifact(
        self, source: ActionsRun, identifier: int, name: str
    ) -> ArtifactIdentity: ...


class FeedbackReader(CommentReader, CheckpointReader, Protocol):
    pass


class CommentWriter(Protocol):
    def post_conversation_comment(self, scope: CommentScope, body: str) -> None: ...
    def reply_to_review_thread(self, identifier: str, body: str) -> None: ...
    def resolve_review_thread(self, identifier: str) -> None: ...


class IneligiblePullRequest(ValueError):
    """An otherwise valid dispatch no longer describes an eligible pull request."""


@dataclass(frozen=True, slots=True)
class CheckpointLocator:
    run_id: int
    attempt: int
    artifact_id: int

    def __post_init__(self) -> None:
        as_positive_integer(self.run_id)
        as_positive_integer(self.attempt)
        as_positive_integer(self.artifact_id)


class FeedbackArtifactKind(StrEnum):
    CONTEXT = "context"
    RESULT = "result"
    SESSION = "session"
    STATE = "state"


def artifact_name(
    kind: FeedbackArtifactKind, scope: CommentScope, source: ActionsRun
) -> str:
    return f"pull-request-comments-{kind}-{scope.number}-{source.identifier}-{source.attempt}"


def read_json_file(path: Path) -> object:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
            raise ValueError("The automation payload must be a regular file")
        content = source.read(MAX_JSON_BYTES + 1)
    if len(content) > MAX_JSON_BYTES:
        raise ValueError("The automation payload exceeds the size limit")
    return loads(content.decode("utf-8"))


def write_json_file(path: Path, value: object) -> None:
    with path.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, indent=2, allow_nan=False)
        output.write("\n")


def _write_text(path: Path, value: str) -> None:
    with path.open("x", encoding="utf-8", newline="\n") as output:
        output.write(value)
        if not value.endswith("\n"):
            output.write("\n")


def _copy_trusted_file(source: Path, destination: Path) -> None:
    if not source.is_file(follow_symlinks=False):
        raise ValueError("The trusted automation configuration must be a regular file")
    _write_text(destination, source.read_text(encoding="utf-8"))


@dataclass(frozen=True, slots=True, kw_only=True)
class FeedbackCheckpoint:
    scope: CommentScope
    source: ActionsRun
    head: CommitSha
    collection: CollectionCheckpoint
    preparation: ArtifactIdentity
    result: ArtifactIdentity
    session: ArtifactIdentity
    session_id: UUID

    def __post_init__(self) -> None:
        if (
            self.scope.repository != self.source.repository
            or not self.source.same_run(self.preparation.source)
            or not self.source.same_run(self.result.source)
            or self.result.source != self.session.source
            or not (
                self.preparation.source.attempt
                <= self.result.source.attempt
                <= self.source.attempt
            )
            or self.preparation.name
            != artifact_name(
                FeedbackArtifactKind.CONTEXT, self.scope, self.preparation.source
            )
            or self.result.name
            != artifact_name(
                FeedbackArtifactKind.RESULT, self.scope, self.result.source
            )
            or self.session.name
            != artifact_name(
                FeedbackArtifactKind.SESSION, self.scope, self.session.source
            )
        ):
            raise ValueError("The feedback checkpoint has inconsistent provenance")

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "scope": self.scope.to_json(),
            "source": self.source.to_json(),
            "head": str(self.head),
            "collection": self.collection.to_json(),
            "preparation": self.preparation.to_json(),
            "result": self.result.to_json(),
            "session": self.session.to_json(),
            "session_id": str(self.session_id),
        }

    @classmethod
    def from_json(cls, value: object) -> FeedbackCheckpoint:
        data = as_object(value)
        require_keys(
            data,
            {
                "version",
                "scope",
                "source",
                "head",
                "collection",
                "preparation",
                "result",
                "session",
                "session_id",
            },
        )
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported feedback checkpoint version")
        session_id = as_string(data["session_id"])
        identifier = UUID(session_id)
        if str(identifier) != session_id:
            raise ValueError("Invalid Codex session identifier")
        return cls(
            scope=CommentScope.from_json(data["scope"]),
            source=ActionsRun.from_json(data["source"]),
            head=CommitSha(as_string(data["head"])),
            collection=CollectionCheckpoint.from_json(data["collection"]),
            preparation=ArtifactIdentity.from_json(data["preparation"]),
            result=ArtifactIdentity.from_json(data["result"]),
            session=ArtifactIdentity.from_json(data["session"]),
            session_id=identifier,
        )


@dataclass(frozen=True, slots=True)
class RetainedFeedback:
    artifact: ArtifactIdentity
    state: FeedbackCheckpoint

    def __post_init__(self) -> None:
        if (
            self.artifact.source != self.state.source
            or self.artifact.name
            != artifact_name(
                FeedbackArtifactKind.STATE, self.state.scope, self.state.source
            )
        ):
            raise ValueError("The retained feedback has inconsistent provenance")


@dataclass(frozen=True, slots=True, kw_only=True)
class PreparedFeedback:
    scope: CommentScope
    source: ActionsRun
    dispatch_head: CommitSha
    head: CommitSha
    head_ref: str
    base: CommitSha
    base_ref: str
    after: Timestamp | None
    collection: CollectionCheckpoint
    targets: tuple[TargetRevision, ...]
    session_id: UUID | None
    continuation: ArtifactIdentity | None

    def __post_init__(self) -> None:
        if self.scope.repository != self.source.repository:
            raise ValueError("The feedback source and repository differ")
        check_branch(self.head_ref)
        check_branch(self.base_ref)
        if len({target.target for target in self.targets}) != len(self.targets):
            raise ValueError("Duplicate actionable feedback target")
        if self.after is not None and self.after > self.collection.through:
            raise ValueError("The feedback collection watermark moved backwards")
        if (self.dispatch_head != self.head) != (self.continuation is not None):
            raise ValueError(
                "An advanced dispatch must identify its completed checkpoint"
            )

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "scope": self.scope.to_json(),
            "source": self.source.to_json(),
            "dispatch_head": str(self.dispatch_head),
            "head": str(self.head),
            "head_ref": self.head_ref,
            "base": str(self.base),
            "base_ref": self.base_ref,
            "after": str(self.after) if self.after is not None else None,
            "collection": self.collection.to_json(),
            "targets": [target.to_json() for target in self.targets],
            "session_id": str(self.session_id) if self.session_id is not None else None,
            "continuation": (
                self.continuation.to_json() if self.continuation is not None else None
            ),
        }

    @classmethod
    def from_json(cls, value: object) -> PreparedFeedback:
        data = as_object(value)
        require_keys(
            data,
            {
                "version",
                "scope",
                "source",
                "dispatch_head",
                "head",
                "head_ref",
                "base",
                "base_ref",
                "after",
                "collection",
                "targets",
                "session_id",
                "continuation",
            },
        )
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported prepared feedback version")
        session_id = data["session_id"]
        continuation = data["continuation"]
        return cls(
            scope=CommentScope.from_json(data["scope"]),
            source=ActionsRun.from_json(data["source"]),
            dispatch_head=CommitSha(as_string(data["dispatch_head"])),
            head=CommitSha(as_string(data["head"])),
            head_ref=as_string(data["head_ref"]),
            base=CommitSha(as_string(data["base"])),
            base_ref=as_string(data["base_ref"]),
            after=Timestamp.parse(as_string(data["after"]))
            if data["after"] is not None
            else None,
            collection=CollectionCheckpoint.from_json(data["collection"]),
            targets=tuple(
                TargetRevision.from_json(value) for value in as_array(data["targets"])
            ),
            session_id=UUID(as_string(session_id)) if session_id is not None else None,
            continuation=ArtifactIdentity.from_json(continuation)
            if continuation is not None
            else None,
        )


@dataclass(frozen=True, slots=True)
class FeedbackResult:
    scope: CommentScope
    source: ActionsRun
    base: CommitSha
    head: CommitSha
    recommendation: CommentRecommendation

    @property
    def commits(self) -> CommitRange | None:
        return CommitRange(self.base, self.head) if self.base != self.head else None

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "scope": self.scope.to_json(),
            "source": self.source.to_json(),
            "base": str(self.base),
            "head": str(self.head),
            "recommendation": self.recommendation.to_json(),
        }

    @classmethod
    def from_json(cls, value: object) -> FeedbackResult:
        data = as_object(value)
        require_keys(
            data, {"version", "scope", "source", "base", "head", "recommendation"}
        )
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported feedback result version")
        return cls(
            CommentScope.from_json(data["scope"]),
            ActionsRun.from_json(data["source"]),
            CommitSha(as_string(data["base"])),
            CommitSha(as_string(data["head"])),
            sanitize_recommendation(
                CommentRecommendation.from_json(data["recommendation"])
            ),
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class FeedbackPublication:
    source: ActionsRun
    prepared: PreparedFeedback
    result: FeedbackResult
    commits: tuple[CommitSha, ...]
    preparation_artifact: ArtifactIdentity
    result_artifact: ArtifactIdentity
    session: ArtifactIdentity
    session_id: UUID

    @property
    def needs_writes(self) -> bool:
        return bool(
            self.commits
            or any(
                action.outcome != CommentOutcome.NO_ACTION
                for action in self.result.recommendation.actions
            )
        )


def require_eligible(
    pull_request: CommentPullRequest,
    scope: CommentScope,
    expected_head: CommitSha,
    *,
    head_ref: str | None = None,
    base_ref: str | None = None,
) -> None:
    details = pull_request.details
    if (
        scope.repository.name.full_name != ManagedRepository.UV_DEV.value
        or details.reference != scope.reference
        or not details.is_open
        or pull_request.draft
        or details.base.repository != scope.repository
        or details.head.repository != scope.repository
        or details.head.sha != expected_head
        or (head_ref is not None and details.head.ref != head_ref)
        or (base_ref is not None and details.base.ref != base_ref)
        or details.head.ref == "main"
    ):
        raise IneligiblePullRequest(
            "The pull request is no longer eligible at the expected revision"
        )
    check_branch(details.head.ref)
    check_branch(details.base.ref)


def _trusted_workflow(run: WorkflowRun) -> bool:
    return (
        run.head_repository == run.source.repository
        and run.path in {WORKFLOW, f"{WORKFLOW}@refs/heads/main", f"{WORKFLOW}@main"}
        and run.event == "workflow_dispatch"
        and run.branch == "main"
    )


def find_checkpoint(
    github: CheckpointReader,
    scope: CommentScope,
    current: ActionsRun,
    *,
    explicit: CheckpointLocator | None = None,
) -> ArtifactIdentity | None:
    """Discover by a bounded query, then use only exact immutable identities."""
    if explicit is not None:
        run = github.read_workflow_run(
            scope.repository, explicit.run_id, explicit.attempt
        )
        if not run.is_successful_dispatch(WORKFLOW):
            raise ValueError(
                "The explicit checkpoint is not from a successful trusted run"
            )
        return github.get_artifact(
            run.source,
            explicit.artifact_id,
            artifact_name(FeedbackArtifactKind.STATE, scope, run.source),
        )
    runs = github.list_successful_workflow_runs(
        scope.repository.name,
        "pull-request-comments.yml",
        limit=CHECKPOINT_DISCOVERY_RUNS,
    )
    for run in runs:
        if (
            run.source.repository != scope.repository
            or run.source.identifier >= current.identifier
            or not run.is_successful_dispatch(WORKFLOW)
        ):
            continue
        try:
            artifact = github.find_artifact(
                run.source, artifact_name(FeedbackArtifactKind.STATE, scope, run.source)
            )
        except (KeyError, TypeError, ValueError) as error:
            logger.info("Ignoring an unusable feedback checkpoint: %s", error)
            continue
        if artifact is not None:
            return artifact
    return None


def validate_checkpoint(
    github: CheckpointReader,
    scope: CommentScope,
    artifact: ArtifactIdentity,
    value: object,
) -> RetainedFeedback:
    state = FeedbackCheckpoint.from_json(value)
    if (
        state.scope != scope
        or state.source != artifact.source
        or artifact.name
        != artifact_name(FeedbackArtifactKind.STATE, scope, state.source)
        or github.get_artifact(artifact.source, artifact.identifier, artifact.name)
        != artifact
    ):
        raise ValueError("The checkpoint does not match its immutable artifact")
    runs = {
        source: github.get_workflow_run(source)
        for source in dict.fromkeys(
            (state.source, state.preparation.source, state.result.source)
        )
    }
    if (
        not runs[state.source].is_successful_dispatch(WORKFLOW)
        or not all(_trusted_workflow(run) for run in runs.values())
        or state.collection.through < runs[state.preparation.source].started_at
        or state.collection.through > Timestamp.now()
        or github.get_artifact(
            state.session.source, state.session.identifier, state.session.name
        )
        != state.session
    ):
        raise ValueError("The checkpoint's workflow or session provenance is invalid")
    return RetainedFeedback(artifact, state)


type FeedbackRecord = ConversationComment | SubmittedReview | ReviewThread


def _read_feedback(
    github: CommentReader, scope: CommentScope, target: CommentTarget
) -> FeedbackRecord:
    match target.kind:
        case CommentTargetKind.CONVERSATION_COMMENT:
            return github.get_conversation_comment(scope, int(target.identifier))
        case CommentTargetKind.PULL_REQUEST_REVIEW:
            return github.get_review(scope, int(target.identifier))
        case CommentTargetKind.REVIEW_THREAD:
            return github.get_review_threads(scope, (target.identifier,))[0]
    assert_never(target.kind)


def _actionable_revision(record: FeedbackRecord) -> TargetRevision | None:
    match record:
        case ConversationComment():
            if record.author.can_trigger and not record.author.is_automation:
                return record.revision
            return None
        case SubmittedReview():
            if (
                record.author.can_trigger
                and not record.author.is_automation
                and record.is_submitted
                and record.body.strip()
            ):
                return record.revision
            return None
        case ReviewThread():
            if any(
                comment.author.can_trigger and not comment.author.is_automation
                for comment in record.comments
            ):
                return record.revision
            return None
    assert_never(record)


def _within_watermark(record: FeedbackRecord, through: Timestamp) -> bool:
    match record:
        case ConversationComment() | SubmittedReview():
            return record.updated_at <= through
        case ReviewThread():
            return all(
                comment.updated_at <= through
                for comment in record.comments
                if not comment.author.is_automation
            )
    assert_never(record)


def collect_comments(
    github: CommentReader,
    scope: CommentScope,
    *,
    previous: CollectionCheckpoint | None,
    through: Timestamp,
) -> CollectedComments:
    if previous is not None and previous.through > through:
        raise ValueError("The feedback collection watermark moved backwards")
    since = previous.through.overlap() if previous is not None else None
    all_conversation = tuple(
        comment
        for comment in github.list_conversation_comments(scope, since)
        if not comment.author.is_automation and comment.updated_at <= through
    )
    all_inline = tuple(
        comment
        for comment in github.list_inline_comments(scope, since)
        if not comment.author.is_automation and comment.updated_at <= through
    )
    seen_comments = set(previous.comments) if previous is not None else set()
    conversation = tuple(
        comment for comment in all_conversation if comment.version not in seen_comments
    )
    inline = tuple(
        comment for comment in all_inline if comment.version not in seen_comments
    )

    # Reviews have no updated-since API. Scan their small, bounded summaries so
    # edits to older reviews are not lost when Actions coalesces pending runs.
    all_reviews = tuple(
        review
        for review in github.list_reviews(scope)
        if review.is_submitted
        and not review.author.is_automation
        and review.updated_at <= through
    )
    prior_reviews = (
        {review.target: review.digest for review in previous.reviews}
        if previous
        else {}
    )
    reviews = tuple(
        review
        for review in all_reviews
        if prior_reviews.get(review.revision.target) != review.revision.digest
    )

    roots = {root.comment_id: root for root in previous.threads} if previous else {}
    cursor = previous.thread_cursor if previous is not None else None
    for _ in range(MAX_COLLECTION_PAGES):
        page = github.list_thread_roots(scope, cursor)
        for root in page.roots:
            existing = roots.get(root.comment_id)
            if existing is not None and existing != root:
                raise ValueError("A review-thread root changed identity")
            roots[root.comment_id] = root
        if len(roots) > MAX_KNOWN_THREADS:
            raise ValueError("Review-thread history exceeds the checkpoint limit")
        if page.cursor is not None:
            if page.has_next_page and page.cursor == cursor:
                raise ValueError("GitHub returned a non-advancing thread cursor")
            cursor = page.cursor
        if not page.has_next_page:
            break
        if page.cursor is None:
            raise ValueError("GitHub omitted the next review-thread cursor")
    else:
        raise ValueError("Review-thread history exceeds the bounded collection budget")

    changed_roots = {comment.root_identifier for comment in inline}
    if missing := changed_roots.difference(roots):
        raise ValueError(
            f"Collected comments have no review thread: {sorted(missing)!r}"
        )
    selected_threads = tuple(
        sorted({roots[identifier].thread_id for identifier in changed_roots})
    )
    if len(selected_threads) > MAX_SELECTED_THREADS:
        raise ValueError("Too many changed review threads for one feedback run")
    threads = github.get_review_threads(scope, selected_threads)
    if {thread.identifier for thread in threads} != set(selected_threads):
        raise ValueError("GitHub did not return all selected review threads")
    trusted_threads = {
        roots[comment.root_identifier].thread_id
        for comment in inline
        if comment.author.can_trigger
    }
    fresh_records = (
        *(comment for comment in conversation if comment.author.can_trigger),
        *(
            review
            for review in reviews
            if review.author.can_trigger and review.body.strip()
        ),
        *(thread for thread in threads if thread.identifier in trusted_threads),
    )
    records = {record.revision.target: record for record in fresh_records}
    fresh = {target: record.revision for target, record in records.items()}
    pending = previous.pending if previous is not None else ()
    prior_targets = {revision.target for revision in pending}
    ordered = (
        *(fresh.get(revision.target, revision) for revision in pending),
        *(
            revision
            for revision in sorted(
                fresh.values(),
                key=lambda revision: (
                    revision.target.kind.value,
                    revision.target.identifier,
                ),
            )
            if revision.target not in prior_targets
        ),
    )
    selected_targets = ordered[:MAX_ACTIONS]
    remaining = ordered[MAX_ACTIONS:]
    selected: list[TargetRevision] = []
    deferred: list[TargetRevision] = []
    conversation_by_id = {comment.identifier: comment for comment in conversation}
    reviews_by_id = {review.identifier: review for review in reviews}
    threads_by_id = {
        thread.identifier: thread
        for thread in threads
        if _within_watermark(thread, through)
    }
    for revision in selected_targets:
        # A complete checkpoint retains undispositioned feedback. Re-read only
        # the bounded batch being offered now, not the entire old discussion.
        record = records.get(revision.target)
        if record is None:
            record = _read_feedback(github, scope, revision.target)
        if record.revision.target != revision.target:
            raise ValueError("GitHub returned a different feedback target")
        if not _within_watermark(record, through):
            deferred.append(record.revision)
            continue
        current = _actionable_revision(record)
        if current is None:
            continue
        selected.append(current)
        match record:
            case ConversationComment():
                conversation_by_id[record.identifier] = record
                continue
            case SubmittedReview():
                reviews_by_id[record.identifier] = record
                continue
            case ReviewThread():
                threads_by_id[record.identifier] = record
                continue
        assert_never(record)
    targets = tuple(selected)
    return CollectedComments(
        tuple(conversation_by_id.values()),
        inline,
        tuple(reviews_by_id.values()),
        tuple(threads_by_id.values()),
        CollectionCheckpoint(
            through,
            cursor,
            tuple(sorted(roots.values(), key=lambda root: root.comment_id)),
            tuple(review.revision for review in all_reviews),
            tuple(
                comment.version
                for comment in (*all_conversation, *all_inline)
                if comment.updated_at == through
            ),
            (*deferred, *remaining),
        ),
        targets,
    )


def fetch_commit(repository: Git, scope: CommentScope, commit: CommitSha) -> None:
    repository.command(
        (
            "-c",
            "credential.helper=",
            "-c",
            "credential.helper=!gh auth git-credential",
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            f"https://github.com/{scope.repository.name}.git",
            str(commit),
        )
    )
    if repository.resolve_commit(str(commit)) != commit:
        raise ValueError("Git fetched a different pull request commit")


def prepare_feedback(
    github: CommentReader,
    repository: Git,
    scope: CommentScope,
    source: ActionsRun,
    expected_head: CommitSha,
    destination: Path,
    *,
    trusted_files: Path,
    previous: RetainedFeedback | None,
    sessions: CodexSessionSnapshot | None,
) -> PreparedFeedback:
    through = Timestamp.now()
    pull_request = github.get_comment_pull_request(scope)
    candidate_head = pull_request.details.head.sha
    require_eligible(pull_request, scope, candidate_head)
    fetch_commit(repository, scope, candidate_head)
    fetch_commit(repository, scope, pull_request.details.base.sha)
    if previous is not None and previous.state.scope != scope:
        raise ValueError("The retained feedback belongs to a different pull request")
    continuation: RetainedFeedback | None = None
    if candidate_head != expected_head:
        if previous is None or previous.state.head != candidate_head:
            raise IneligiblePullRequest(
                "The pull request moved beyond the dispatched or completed feedback head"
            )
        fetch_commit(repository, scope, expected_head)
        if not repository.is_ancestor(expected_head, candidate_head):
            raise IneligiblePullRequest(
                "The completed feedback head is not a descendant of the dispatch"
            )
        continuation = previous
    resume = previous.state if previous is not None else None
    if resume is not None:
        try:
            current = (
                sessions is not None
                and sessions.identifier == resume.session_id
                and repository.is_ancestor(resume.head, candidate_head)
            )
        except subprocess.CalledProcessError:
            current = False
        if not current:
            logger.info(
                "The retained session is not a current ancestor; collecting a fresh bounded history"
            )
            resume = None
            sessions = None
    if resume is None:
        sessions = None
    try:
        collection = collect_comments(
            github,
            scope,
            previous=resume.collection if resume is not None else None,
            through=through,
        )
    except KeyError, TypeError, ValueError, subprocess.CalledProcessError:
        if resume is None:
            raise
        logger.info(
            "The saved collection cursor is no longer usable; collecting a fresh bounded history"
        )
        resume = None
        sessions = None
        collection = collect_comments(github, scope, previous=None, through=through)
    # The three-dot diff and all metadata describe the same immutable head.
    diff = repository.output(
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        f"{pull_request.details.base.sha}...{candidate_head}",
        "--",
    )
    prepared = PreparedFeedback(
        scope=scope,
        source=source,
        dispatch_head=expected_head,
        head=candidate_head,
        head_ref=pull_request.details.head.ref,
        base=pull_request.details.base.sha,
        base_ref=pull_request.details.base.ref,
        after=resume.collection.through if resume is not None else None,
        collection=collection.checkpoint,
        targets=collection.targets,
        session_id=sessions.identifier if sessions is not None else None,
        continuation=continuation.artifact if continuation is not None else None,
    )
    destination.mkdir(parents=True, exist_ok=False)
    write_json_file(destination / "prepared.json", prepared.to_json())
    _write_text(destination / "event.json", pull_request.event_json)
    write_json_file(destination / "comments.json", collection.to_json())
    _write_text(destination / "diff.patch", diff)
    _copy_trusted_file(
        trusted_files / "agents/codex/config.toml", destination / "config.toml"
    )
    _copy_trusted_file(
        trusted_files / "agents/prompts/pull-request-comments.md",
        destination / "prompt.md",
    )
    write_json_file(destination / "schema.json", CommentRecommendation.schema())
    if continuation is not None:
        write_json_file(destination / "continuation.json", continuation.state.to_json())
    if sessions is not None:
        write_sessions(sessions, destination / "sessions")
    return prepared


def prepare_agent(
    repository: Git,
    prepared: PreparedFeedback,
    context: Path,
    codex_home: Path,
    *,
    source: ActionsRun,
    trusted_root: Path,
) -> tuple[str, ...]:
    if not source.same_run(prepared.source) or source.attempt < prepared.source.attempt:
        raise ValueError("The agent does not belong to the preparation's workflow run")
    if repository.resolve_commit("HEAD") != prepared.head or repository.output(
        "status", "--porcelain"
    ):
        raise ValueError(
            "The candidate checkout does not match the prepared clean head"
        )
    codex_home.mkdir(parents=True, exist_ok=False)
    _copy_trusted_file(context / "config.toml", codex_home / "config.toml")
    arguments: tuple[str, ...] = ()
    if prepared.session_id is not None:
        sessions = snapshot_sessions(
            context / "sessions", repository.path, trusted_root=trusted_root
        )
        if sessions.identifier != prepared.session_id:
            raise ValueError("The retained Codex session changed")
        write_sessions(sessions, codex_home / "sessions")
        arguments = ("resume", str(sessions.identifier), "-")
    repository.command(("config", "user.name", BOT_NAME))
    repository.command(("config", "user.email", BOT_EMAIL))
    return arguments


def verify_commits(
    repository: Git, commits: CommitRange | None
) -> tuple[CommitSha, ...]:
    if commits is None:
        return ()
    if not repository.is_ancestor(commits.base, commits.head):
        raise ValueError("The feedback commits rewrite existing history")
    result: list[CommitSha] = []
    parent = commits.base
    for line in repository.output(
        "rev-list", "--reverse", "--parents", f"{commits.base}..{commits.head}"
    ).splitlines():
        identifiers = line.split()
        if len(identifiers) != 2 or identifiers[1] != str(parent):
            raise ValueError("Feedback must be addressed in discrete, linear commits")
        commit = CommitSha(identifiers[0])
        metadata = repository.output(
            "show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce%x00%B", str(commit), "--"
        ).split("\0", 4)
        if (
            len(metadata) != 5
            or metadata[:4] != [BOT_NAME, BOT_EMAIL, BOT_NAME, BOT_EMAIL]
            or not metadata[4].strip()
            or re.search(r"(?im)^co-authored(?:-by)?:", metadata[4])
        ):
            raise ValueError("A feedback commit has unexpected authorship or trailers")
        result.append(commit)
        parent = commit
        if len(result) > MAX_COMMITS:
            raise ValueError("Too many feedback commits")
    if not result or parent != commits.head:
        raise ValueError(
            "The feedback result does not contain the expected new commits"
        )
    repository.command(("diff", "--check", f"{commits.base}..{commits.head}", "--"))
    return tuple(result)


def validate_recommendation(
    prepared: PreparedFeedback,
    recommendation: CommentRecommendation,
    commits: tuple[CommitSha, ...],
) -> None:
    allowed = {target.target for target in prepared.targets}
    if {action.target for action in recommendation.actions} != allowed:
        raise ValueError("Every prepared feedback target needs exactly one disposition")
    addressed: set[CommitSha] = set()
    for action in recommendation.actions:
        if (
            action.addressing_commit is not None
            and action.addressing_commit not in commits
        ):
            raise ValueError("The addressing commit is not a new feedback commit")
        if action.addressing_commit is not None:
            addressed.add(action.addressing_commit)
    if addressed != set(commits):
        raise ValueError("Every new commit must address authorized feedback")


def persist_feedback_result(
    repository: Git,
    prepared: PreparedFeedback,
    recommendation: CommentRecommendation,
    destination: Path,
    *,
    source: ActionsRun,
    scratch: Path,
) -> FeedbackResult:
    if not source.same_run(prepared.source) or source.attempt < prepared.source.attempt:
        raise ValueError("The result does not belong to the preparation's workflow run")
    with inspect_candidate(
        repository, base=prepared.head, scratch=scratch
    ) as candidate:
        candidate.require_clean()
        result = FeedbackResult(
            prepared.scope,
            source,
            prepared.head,
            candidate.head,
            sanitize_recommendation(recommendation),
        )
        commits = verify_commits(candidate.repository, result.commits)
        validate_recommendation(prepared, result.recommendation, commits)
        destination.mkdir(parents=True, exist_ok=False)
        if result.commits is not None:
            persist_commit(
                candidate.repository, result.commits, destination / "commits.bundle"
            )
        write_json_file(destination / "result.json", result.to_json())
    return result


def _target_revision(
    github: CommentReader, scope: CommentScope, target: CommentTarget
) -> TargetRevision:
    revision = _actionable_revision(_read_feedback(github, scope, target))
    if revision is None:
        raise ValueError("The target is no longer trusted feedback")
    return revision


def _require_targets_current(
    github: CommentReader,
    prepared: PreparedFeedback,
    recommendation: CommentRecommendation,
) -> None:
    expected = {revision.target: revision for revision in prepared.targets}
    for action in recommendation.actions:
        if action.outcome == CommentOutcome.NO_ACTION:
            continue
        if (
            _target_revision(github, prepared.scope, action.target)
            != expected[action.target]
        ):
            raise ValueError(
                "Feedback changed after collection; refusing stale publication"
            )


def prepare_publication(
    github: FeedbackReader,
    repository: Git,
    scope: CommentScope,
    source: ActionsRun,
    expected_head: CommitSha,
    context: Path,
    result_directory: Path,
    session_directory: Path,
    *,
    trusted_root: Path,
    preparation_artifact_id: int,
    result_artifact_id: int,
    session_artifact_id: int,
) -> FeedbackPublication:
    prepared = PreparedFeedback.from_json(read_json_file(context / "prepared.json"))
    result = FeedbackResult.from_json(read_json_file(result_directory / "result.json"))
    if (
        prepared.scope != scope
        or result.scope != scope
        or not source.same_run(prepared.source)
        or not source.same_run(result.source)
        or not prepared.source.attempt <= result.source.attempt <= source.attempt
        or prepared.dispatch_head != expected_head
        or result.base != prepared.head
    ):
        raise ValueError("The feedback artifacts do not match the trusted dispatch")
    runs = tuple(
        github.get_workflow_run(producer)
        for producer in dict.fromkeys((prepared.source, result.source, source))
    )
    if not all(_trusted_workflow(run) for run in runs):
        raise ValueError("The publisher is not running the trusted workflow on main")
    preparation_artifact = github.get_artifact(
        prepared.source,
        preparation_artifact_id,
        artifact_name(FeedbackArtifactKind.CONTEXT, scope, prepared.source),
    )
    result_artifact = github.get_artifact(
        result.source,
        result_artifact_id,
        artifact_name(FeedbackArtifactKind.RESULT, scope, result.source),
    )
    pull_request = github.get_comment_pull_request(scope)
    # A failed publisher can be retried after its exact validated commit was pushed.
    # No other changed head is accepted, even when it contains the same diff.
    live_head = pull_request.details.head.sha
    if live_head not in {prepared.head, result.head}:
        raise ValueError("The pull request head changed before publication")
    require_eligible(
        pull_request,
        scope,
        live_head,
        head_ref=prepared.head_ref,
        base_ref=prepared.base_ref,
    )
    fetch_commit(repository, scope, prepared.head)
    if prepared.continuation is not None:
        retained = validate_checkpoint(
            github,
            scope,
            prepared.continuation,
            read_json_file(context / "continuation.json"),
        )
        fetch_commit(repository, scope, expected_head)
        if retained.state.head != prepared.head or not repository.is_ancestor(
            expected_head, prepared.head
        ):
            raise ValueError("The completed checkpoint does not authorize this head")
    elif (context / "continuation.json").exists():
        raise ValueError("An unchanged dispatch must not include a continuation")
    if result.commits is not None:
        load_commit(repository, result_directory / "commits.bundle", result.commits)
    elif (result_directory / "commits.bundle").exists():
        raise ValueError("A no-change result must not include a Git bundle")
    commits = verify_commits(repository, result.commits)
    validate_recommendation(prepared, result.recommendation, commits)
    _require_targets_current(github, prepared, result.recommendation)
    session = github.get_artifact(
        result.source,
        session_artifact_id,
        artifact_name(FeedbackArtifactKind.SESSION, scope, result.source),
    )
    snapshot = snapshot_sessions(
        session_directory, repository.path, trusted_root=trusted_root
    )
    if prepared.session_id is not None and snapshot.identifier != prepared.session_id:
        raise ValueError("Codex did not continue the verified feedback session")
    return FeedbackPublication(
        source=source,
        prepared=prepared,
        result=result,
        commits=commits,
        preparation_artifact=preparation_artifact,
        result_artifact=result_artifact,
        session=session,
        session_id=snapshot.identifier,
    )


def _marker(scope: CommentScope, revision: TargetRevision) -> str:
    key = fingerprint({"scope": scope.to_json(), "revision": revision.to_json()})
    return f"<!-- uv-automations:pull-request-comments:{key} -->"


def _response_body(scope: CommentScope, action: CommentAction, marker: str) -> str:
    body = action.body
    if action.addressing_commit is not None:
        body += (
            f"\n\nAddressed in [`{str(action.addressing_commit)[:12]}`]"
            f"(https://github.com/{scope.repository.name}/commit/{action.addressing_commit})."
        )
    match action.target.kind:
        case CommentTargetKind.CONVERSATION_COMMENT:
            anchor = f"issuecomment-{action.target.identifier}"
            response = (
                f"In response to [this comment](https://github.com/{scope.repository.name}"
                f"/pull/{scope.number}#{anchor}):\n\n{body}"
            )
            return f"{response}\n\n{marker}"
        case CommentTargetKind.PULL_REQUEST_REVIEW:
            anchor = f"pullrequestreview-{action.target.identifier}"
            response = (
                f"In response to [this review](https://github.com/{scope.repository.name}"
                f"/pull/{scope.number}#{anchor}):\n\n{body}"
            )
            return f"{response}\n\n{marker}"
        case CommentTargetKind.REVIEW_THREAD:
            return f"{body}\n\n{marker}"
    assert_never(action.target.kind)


def _apply_thread_action(
    github: CommentReader,
    writer: CommentWriter,
    scope: CommentScope,
    action: CommentAction,
    marker: str,
) -> None:
    thread = github.get_review_threads(scope, (action.target.identifier,))[0]
    match action.outcome:
        case CommentOutcome.COMMIT_AND_RESOLVE:
            if not thread.resolved:
                writer.resolve_review_thread(action.target.identifier)
            return
        case CommentOutcome.RESPOND | CommentOutcome.CLARIFY:
            if not any(
                comment.author.is_publisher and marker in comment.body
                for comment in thread.comments
            ):
                writer.reply_to_review_thread(
                    action.target.identifier, _response_body(scope, action, marker)
                )
            return
        case CommentOutcome.NO_ACTION:
            return
    assert_never(action.outcome)


def apply_publication(
    github: CommentReader,
    writer: CommentWriter,
    repository: Git,
    publication: FeedbackPublication,
) -> FeedbackCheckpoint:
    prepared = publication.prepared
    result = publication.result
    scope = prepared.scope
    expected = {revision.target: revision for revision in prepared.targets}
    pull_request = github.get_comment_pull_request(scope)
    live_head = pull_request.details.head.sha
    if live_head not in {prepared.head, result.head}:
        raise ValueError("The pull request head changed before publication")
    require_eligible(
        pull_request,
        scope,
        live_head,
        head_ref=prepared.head_ref,
        base_ref=prepared.base_ref,
    )
    _require_targets_current(github, prepared, result.recommendation)
    if publication.commits and live_head == prepared.head:
        # Strict ancestry was verified above. The lease is an atomic stale-ref
        # guard, not permission to rewrite history or undo a concurrent rewind.
        repository.command(
            (
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!gh auth git-credential",
                "push",
                "--porcelain",
                f"--force-with-lease=refs/heads/{prepared.head_ref}:{prepared.head}",
                f"https://github.com/{scope.repository.name}.git",
                f"{result.head}:refs/heads/{prepared.head_ref}",
            )
        )

    top_level_markers: set[str] = set()
    if any(
        action.target.kind != CommentTargetKind.REVIEW_THREAD
        and action.outcome != CommentOutcome.NO_ACTION
        for action in result.recommendation.actions
    ):
        since = prepared.after.overlap() if prepared.after is not None else None
        top_level_markers = {
            comment.body
            for comment in github.list_conversation_comments(scope, since)
            if comment.author.is_publisher
        }
    for action in result.recommendation.actions:
        if action.outcome == CommentOutcome.NO_ACTION:
            continue
        require_eligible(
            github.get_comment_pull_request(scope),
            scope,
            result.head,
            head_ref=prepared.head_ref,
            base_ref=prepared.base_ref,
        )
        revision = expected[action.target]
        if _target_revision(github, scope, action.target) != revision:
            raise ValueError("Feedback changed during publication")
        marker = _marker(scope, revision)
        match action.target.kind:
            case CommentTargetKind.REVIEW_THREAD:
                _apply_thread_action(github, writer, scope, action, marker)
                continue
            case (
                CommentTargetKind.CONVERSATION_COMMENT
                | CommentTargetKind.PULL_REQUEST_REVIEW
            ):
                if not any(marker in body for body in top_level_markers):
                    body = _response_body(scope, action, marker)
                    writer.post_conversation_comment(scope, body)
                    top_level_markers.add(body)
                continue
        assert_never(action.target.kind)
    require_eligible(
        github.get_comment_pull_request(scope),
        scope,
        result.head,
        head_ref=prepared.head_ref,
        base_ref=prepared.base_ref,
    )
    return FeedbackCheckpoint(
        scope=scope,
        source=publication.source,
        head=result.head,
        collection=prepared.collection,
        preparation=publication.preparation_artifact,
        result=publication.result_artifact,
        session=publication.session,
        session_id=publication.session_id,
    )
