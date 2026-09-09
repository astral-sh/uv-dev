"""Discover one pull request's immutable checkpoint and bound automatic follow-ups."""

import logging
import re
import subprocess
from dataclasses import dataclass
from typing import Protocol

from uv_automations.comment_models import (
    MAX_PENDING_TARGETS,
    CommentScope,
    ContinuationKey,
    FeedbackContinuation,
    as_continuation_budget,
    as_feedback_progress,
    fingerprint,
)
from uv_automations.github_actions import (
    ActionsRun,
    ArtifactIdentity,
    ManifestArtifact,
    ManifestPage,
    WorkflowRun,
)
from uv_automations.json import (
    as_object,
    as_positive_integer,
    as_string,
    require_keys,
)
from uv_automations.models import CommitSha, RepositoryIdentity, Timestamp
from uv_automations.workflows.comments import (
    WORKFLOW,
    CheckpointLocator,
    CheckpointReader,
    CommentReader,
    FeedbackArtifactKind,
    FeedbackCheckpoint,
    IneligiblePullRequest,
    RetainedFeedback,
    artifact_name,
    find_checkpoint,
    require_eligible,
    validate_checkpoint,
    validate_current_checkpoint,
)

logger = logging.getLogger(__name__)

CHECKPOINT_DISCOVERY_ARTIFACTS = 20


class IndexReader(CheckpointReader, Protocol):
    def list_manifest_artifacts(
        self, repository: RepositoryIdentity, name: str, *, limit: int
    ) -> ManifestPage: ...
    def get_manifest_artifact(
        self, repository: RepositoryIdentity, identifier: int, name: str
    ) -> ManifestArtifact: ...
    def find_manifest_artifact(
        self, source: ActionsRun, name: str
    ) -> ManifestArtifact | None: ...
    def read_json_manifest(self, artifact: ManifestArtifact) -> object: ...


class ContinuationReader(IndexReader, CommentReader, Protocol):
    pass


class ContinuationWriter(Protocol):
    def dispatch_feedback(self, continuation: FeedbackContinuation) -> None: ...


def index_name(scope: CommentScope, source: ActionsRun) -> str:
    return artifact_name(FeedbackArtifactKind.INDEX, scope, source)


def discovery_name(scope: CommentScope) -> str:
    return f"pull-request-comments-checkpoint-{scope.number}"


def marker_name(scope: CommentScope, key: ContinuationKey) -> str:
    return f"pull-request-comments-consumed-{scope.number}-{key}"


@dataclass(frozen=True, slots=True, kw_only=True)
class FeedbackIndex:
    """An immutable summary whose complete collection remains in state."""

    scope: CommentScope
    state: ArtifactIdentity
    session: ArtifactIdentity
    checkpoint_digest: str
    head: CommitSha
    through: Timestamp
    pending_targets: int
    processed_targets: int
    continuations_remaining: int
    continuation_key: ContinuationKey | None

    def __post_init__(self) -> None:
        as_continuation_budget(self.continuations_remaining)
        as_feedback_progress(self.processed_targets)
        if (
            self.scope.repository != self.state.source.repository
            or not self.state.source.same_run(self.session.source)
            or self.session.source.attempt > self.state.source.attempt
            or self.state.name
            != artifact_name(FeedbackArtifactKind.STATE, self.scope, self.state.source)
            or self.session.name
            != artifact_name(
                FeedbackArtifactKind.SESSION, self.scope, self.session.source
            )
            or re.fullmatch(r"[0-9a-f]{64}", self.checkpoint_digest) is None
            or type(self.pending_targets) is not int
            or not 0 <= self.pending_targets <= MAX_PENDING_TARGETS
        ):
            raise ValueError("The feedback index has inconsistent provenance")

    @property
    def needs_continuation(self) -> bool:
        return bool(
            self.pending_targets
            and self.processed_targets
            and self.continuations_remaining
        )

    @classmethod
    def from_checkpoint(cls, checkpoint: RetainedFeedback) -> FeedbackIndex:
        state = checkpoint.state
        return cls(
            scope=state.scope,
            state=checkpoint.artifact,
            session=state.session,
            checkpoint_digest=fingerprint(state.to_json()),
            head=state.head,
            through=state.collection.through,
            pending_targets=len(state.collection.pending),
            processed_targets=state.processed_targets,
            continuations_remaining=state.continuations_remaining,
            continuation_key=state.continuation_key,
        )

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "scope": self.scope.to_json(),
            "state": self.state.to_json(),
            "session": self.session.to_json(),
            "checkpoint_digest": self.checkpoint_digest,
            "head": str(self.head),
            "through": str(self.through),
            "pending_targets": self.pending_targets,
            "processed_targets": self.processed_targets,
            "continuations_remaining": self.continuations_remaining,
            "continuation_key": (
                str(self.continuation_key)
                if self.continuation_key is not None
                else None
            ),
        }

    @classmethod
    def from_json(cls, value: object) -> FeedbackIndex:
        data = as_object(value)
        require_keys(
            data,
            {
                "version",
                "scope",
                "state",
                "session",
                "checkpoint_digest",
                "head",
                "through",
                "pending_targets",
                "processed_targets",
                "continuations_remaining",
                "continuation_key",
            },
        )
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported feedback index version")
        pending = data["pending_targets"]
        if type(pending) is not int:
            raise ValueError("Expected an integer pending-target count")
        return cls(
            scope=CommentScope.from_json(data["scope"]),
            state=ArtifactIdentity.from_json(data["state"]),
            session=ArtifactIdentity.from_json(data["session"]),
            checkpoint_digest=as_string(data["checkpoint_digest"]),
            head=CommitSha(as_string(data["head"])),
            through=Timestamp.parse(as_string(data["through"])),
            pending_targets=pending,
            processed_targets=as_feedback_progress(data["processed_targets"]),
            continuations_remaining=as_continuation_budget(
                data["continuations_remaining"]
            ),
            continuation_key=ContinuationKey(as_string(data["continuation_key"]))
            if data["continuation_key"] is not None
            else None,
        )


@dataclass(frozen=True, slots=True)
class VerifiedFeedbackIndex:
    """A metadata-verified index; its workflow completion is checked separately."""

    artifact: ArtifactIdentity
    created_at: Timestamp
    value: FeedbackIndex

    def __post_init__(self) -> None:
        if (
            self.artifact.source != self.value.state.source
            or self.artifact.name != index_name(self.value.scope, self.artifact.source)
        ):
            raise ValueError("The feedback index belongs to a different publisher")


@dataclass(frozen=True, slots=True, kw_only=True)
class FeedbackIndexAlias:
    """A replaceable pointer retaining at most one successful prior attempt."""

    scope: CommentScope
    index: ArtifactIdentity
    previous: ArtifactIdentity | None

    def __post_init__(self) -> None:
        if (
            self.index.source.repository != self.scope.repository
            or self.index.name != index_name(self.scope, self.index.source)
        ):
            raise ValueError("The discovery alias has inconsistent provenance")
        if self.previous is not None and (
            not self.previous.source.same_run(self.index.source)
            or self.previous.source.attempt >= self.index.source.attempt
            or self.previous.name != index_name(self.scope, self.previous.source)
        ):
            raise ValueError("The discovery fallback is not an earlier run attempt")

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "scope": self.scope.to_json(),
            "index": self.index.to_json(),
            "previous": self.previous.to_json() if self.previous is not None else None,
        }

    @classmethod
    def from_json(cls, value: object) -> FeedbackIndexAlias:
        data = as_object(value)
        require_keys(data, {"version", "scope", "index", "previous"})
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported feedback index alias version")
        return cls(
            scope=CommentScope.from_json(data["scope"]),
            index=ArtifactIdentity.from_json(data["index"]),
            previous=(
                ArtifactIdentity.from_json(data["previous"])
                if data["previous"] is not None
                else None
            ),
        )


@dataclass(frozen=True, slots=True)
class FeedbackIndexAliases:
    discovery: FeedbackIndexAlias
    consumption: FeedbackIndexAlias | None


@dataclass(frozen=True, slots=True)
class _InspectedAlias:
    current: VerifiedFeedbackIndex
    previous: VerifiedFeedbackIndex | None
    run: WorkflowRun

    @property
    def completed(self) -> VerifiedFeedbackIndex | None:
        return max(
            (
                value
                for value in (
                    self.previous,
                    self.current if self.run.is_successful_dispatch(WORKFLOW) else None,
                )
                if value is not None
            ),
            key=_index_order,
            default=None,
        )


@dataclass(frozen=True, slots=True)
class CheckpointRequest:
    checkpoint: CheckpointLocator
    index_id: int | None

    def __post_init__(self) -> None:
        if self.index_id is not None:
            as_positive_integer(self.index_id)


@dataclass(frozen=True, slots=True)
class CheckpointSelection:
    state: ArtifactIdentity
    index: ArtifactIdentity | None = None

    def __post_init__(self) -> None:
        if self.index is not None and self.index.source != self.state.source:
            raise ValueError("The selected checkpoint and index publishers differ")

    def to_json(self) -> dict[str, object]:
        return {
            "version": 1,
            "state": self.state.to_json(),
            "index": self.index.to_json() if self.index is not None else None,
        }

    @classmethod
    def from_json(cls, value: object) -> CheckpointSelection:
        data = as_object(value)
        require_keys(data, {"version", "state", "index"})
        if type(data["version"]) is not int or data["version"] != 1:
            raise ValueError("Unsupported checkpoint selection version")
        return cls(
            ArtifactIdentity.from_json(data["state"]),
            ArtifactIdentity.from_json(data["index"])
            if data["index"] is not None
            else None,
        )


@dataclass(frozen=True, slots=True)
class BootstrapFeedback:
    pass


@dataclass(frozen=True, slots=True)
class SupersededFeedback:
    checkpoint: ArtifactIdentity


type CheckpointDiscovery = CheckpointSelection | BootstrapFeedback | SupersededFeedback


def _inspect_index(
    github: IndexReader,
    scope: CommentScope,
    manifest: ManifestArtifact,
) -> tuple[ArtifactIdentity, FeedbackIndex, WorkflowRun]:
    value = FeedbackIndex.from_json(github.read_json_manifest(manifest))
    source = value.state.source
    identity = manifest.bind(source)
    run = github.get_workflow_run(source)
    if (
        value.scope != scope
        or manifest.name != index_name(scope, source)
        or manifest.repository != scope.repository
        or manifest.created_at < run.started_at
        or value.through > manifest.created_at
        or value.through > Timestamp.now()
        or not run.is_main_dispatch(WORKFLOW)
        or github.get_artifact(source, value.state.identifier, value.state.name)
        != value.state
        or github.get_artifact(
            value.session.source, value.session.identifier, value.session.name
        )
        != value.session
    ):
        raise ValueError(
            "The feedback index's workflow or artifact provenance is invalid"
        )
    return identity, value, run


def _read_index(
    github: IndexReader,
    scope: CommentScope,
    manifest: ManifestArtifact,
    *,
    current: ActionsRun | None = None,
) -> VerifiedFeedbackIndex:
    identity, value, run = _inspect_index(github, scope, manifest)
    if current is None and not run.is_successful_dispatch(WORKFLOW):
        raise ValueError("The feedback index's publisher attempt is not eligible")
    if current is not None and identity.source != current:
        raise ValueError("The feedback index's publisher attempt is not eligible")
    return VerifiedFeedbackIndex(identity, manifest.created_at, value)


def read_completed_index(
    github: IndexReader, scope: CommentScope, manifest: ManifestArtifact
) -> VerifiedFeedbackIndex:
    return _read_index(github, scope, manifest)


def _load_index(
    github: IndexReader,
    scope: CommentScope,
    identity: ArtifactIdentity,
    *,
    current: ActionsRun | None = None,
) -> VerifiedFeedbackIndex:
    index = _read_index(
        github,
        scope,
        github.get_manifest_artifact(
            scope.repository, identity.identifier, identity.name
        ),
        current=current,
    )
    if index.artifact != identity:
        raise ValueError("The immutable feedback index identity changed")
    return index


def _inspect_alias(
    github: IndexReader,
    scope: CommentScope,
    manifest: ManifestArtifact,
    *,
    expected_name: str,
) -> _InspectedAlias:
    alias = FeedbackIndexAlias.from_json(github.read_json_manifest(manifest))
    source = alias.index.source
    manifest.bind(source)
    run = github.get_workflow_run(source)
    if (
        alias.scope != scope
        or manifest.name != expected_name
        or manifest.repository != scope.repository
        or manifest.created_at < run.started_at
        or not run.is_main_dispatch(WORKFLOW)
    ):
        raise ValueError("The feedback index alias has invalid workflow provenance")
    index = _load_index(github, scope, alias.index, current=source)
    previous = (
        _load_index(github, scope, alias.previous)
        if alias.previous is not None
        else None
    )
    if index.created_at > manifest.created_at or (
        previous is not None
        and (
            previous.created_at > index.created_at
            or previous.value.continuation_key != index.value.continuation_key
            or previous.value.continuations_remaining
            != index.value.continuations_remaining
        )
    ):
        raise ValueError("The feedback index alias has inconsistent retry provenance")
    return _InspectedAlias(index, previous, run)


def _can_precede(source: ActionsRun, current: ActionsRun) -> bool:
    """A separately verified completed run may precede an older run's retry."""
    if source.repository != current.repository:
        return False
    if source.identifier == current.identifier:
        return source.same_run(current) and source.attempt < current.attempt
    return True


def _index_order(index: VerifiedFeedbackIndex) -> tuple[Timestamp, Timestamp, int]:
    return index.value.through, index.created_at, index.artifact.identifier


def _discover_index(
    github: IndexReader, scope: CommentScope, current: ActionsRun
) -> VerifiedFeedbackIndex | None:
    page = github.list_manifest_artifacts(
        scope.repository,
        discovery_name(scope),
        limit=CHECKPOINT_DISCOVERY_ARTIFACTS,
    )
    selected: VerifiedFeedbackIndex | None = None
    for manifest in sorted(
        page.artifacts,
        key=lambda value: (value.created_at, value.identifier),
        reverse=True,
    ):
        # Each pointer is created after its index and collection watermark.
        # Once an older pointer predates our best coverage, it cannot improve it.
        if selected is not None and manifest.created_at < selected.value.through:
            break
        try:
            inspected = _inspect_alias(
                github, scope, manifest, expected_name=discovery_name(scope)
            )
            index = inspected.completed
            if (
                index is not None
                and _can_precede(index.artifact.source, current)
                and (selected is None or _index_order(index) > _index_order(selected))
            ):
                selected = index
        except (
            KeyError,
            TypeError,
            ValueError,
            OSError,
            subprocess.CalledProcessError,
            subprocess.TimeoutExpired,
        ) as error:
            logger.info("Ignoring an unusable per-pull-request checkpoint: %s", error)
    return selected


def _consumed_continuation(
    github: IndexReader,
    current: ActionsRun,
    parent: VerifiedFeedbackIndex,
    key: ContinuationKey,
    remaining: int,
) -> ArtifactIdentity | None:
    """Prove absence within a complete exact-key page, or fail closed."""
    scope = parent.value.scope
    name = marker_name(scope, key)
    page = github.list_manifest_artifacts(
        scope.repository, name, limit=CHECKPOINT_DISCOVERY_ARTIFACTS
    )
    uncertain = not page.complete
    for manifest in page.artifacts:
        try:
            inspected = _inspect_alias(github, scope, manifest, expected_name=name)
            index = inspected.current
            value = index.value
            if (
                value.continuation_key != key
                or value.continuations_remaining != remaining
                or value.through < parent.value.through
                or not _can_precede(parent.artifact.source, index.artifact.source)
            ):
                raise ValueError(
                    "The consumption marker does not match its continuation"
                )
            completed = inspected.completed
            if completed is not None:
                if completed.value.through < parent.value.through or not _can_precede(
                    parent.artifact.source, completed.artifact.source
                ):
                    raise ValueError("The consumption fallback predates its parent")
                if _can_precede(completed.artifact.source, current):
                    return completed.value.state
                if completed.artifact.source != current:
                    raise ValueError("The consumption marker is from a later attempt")
            if inspected.run.status != "completed" and index.artifact.source != current:
                uncertain = True
        except (
            KeyError,
            TypeError,
            ValueError,
            OSError,
            subprocess.CalledProcessError,
            subprocess.TimeoutExpired,
        ) as error:
            uncertain = True
            logger.info("Cannot verify a continuation consumption marker: %s", error)
    if uncertain:
        raise ValueError(
            "Cannot prove continuation completion within the discovery bound"
        )
    return None


def next_continuation(index: VerifiedFeedbackIndex) -> FeedbackContinuation | None:
    state = index.value
    if not state.needs_continuation:
        return None
    remaining = state.continuations_remaining - 1
    key = ContinuationKey(
        fingerprint(
            {
                "version": 1,
                "scope": state.scope.to_json(),
                "index": index.artifact.to_json(),
                "state": state.state.to_json(),
                "head": str(state.head),
                "remaining": remaining,
            }
        )
    )
    return FeedbackContinuation(
        scope=state.scope,
        head=state.head,
        checkpoint_run=state.state.source.identifier,
        checkpoint_attempt=state.state.source.attempt,
        checkpoint_artifact=state.state.identifier,
        checkpoint_index=index.artifact.identifier,
        key=key,
        remaining=remaining,
    )


def _require_current_workflow(
    github: IndexReader, scope: CommentScope, current: ActionsRun
) -> None:
    if scope.repository != current.repository or not github.get_workflow_run(
        current
    ).is_main_dispatch(WORKFLOW):
        raise ValueError("Checkpoint discovery requires the trusted workflow on main")


def _select_explicit_checkpoint(
    github: IndexReader,
    scope: CommentScope,
    current: ActionsRun,
    explicit: CheckpointRequest,
) -> CheckpointSelection:
    if explicit.index_id is None:
        artifact = find_checkpoint(
            github,
            scope,
            current,
            explicit=explicit.checkpoint,
        )
        if artifact is None or not _can_precede(artifact.source, current):
            raise ValueError("The explicit checkpoint is not from an earlier run")
        return CheckpointSelection(artifact)
    selected = _read_requested_index(github, scope, current, explicit)
    return CheckpointSelection(selected.value.state, selected.artifact)


def _read_requested_index(
    github: IndexReader,
    scope: CommentScope,
    current: ActionsRun,
    explicit: CheckpointRequest,
) -> VerifiedFeedbackIndex:
    if explicit.index_id is None:
        raise ValueError("The request does not identify an immutable feedback index")
    source = github.read_workflow_run(
        scope.repository, explicit.checkpoint.run_id, explicit.checkpoint.attempt
    ).source
    selected = read_completed_index(
        github,
        scope,
        github.get_manifest_artifact(
            scope.repository, explicit.index_id, index_name(scope, source)
        ),
    )
    if (
        selected.value.state.source != source
        or selected.value.state.identifier != explicit.checkpoint.artifact_id
        or not _can_precede(selected.artifact.source, current)
    ):
        raise ValueError("The explicit checkpoint and index identities do not match")
    return selected


def validate_continuation(
    github: IndexReader,
    scope: CommentScope,
    current: ActionsRun,
    expected_head: CommitSha,
    *,
    explicit: CheckpointRequest | None,
    continuations_remaining: int,
    continuation_key: ContinuationKey,
) -> VerifiedFeedbackIndex:
    """Revalidate the exact parent IDs without rediscovery on a publisher retry."""
    as_continuation_budget(continuations_remaining)
    _require_current_workflow(github, scope, current)
    if explicit is None or explicit.index_id is None:
        raise ValueError(
            "A continuation requires exact checkpoint and index identities"
        )
    index = _read_requested_index(github, scope, current, explicit)
    requested = FeedbackContinuation(
        scope=scope,
        head=expected_head,
        checkpoint_run=explicit.checkpoint.run_id,
        checkpoint_attempt=explicit.checkpoint.attempt,
        checkpoint_artifact=explicit.checkpoint.artifact_id,
        checkpoint_index=explicit.index_id,
        key=continuation_key,
        remaining=continuations_remaining,
    )
    if next_continuation(index) != requested:
        raise ValueError("The continuation does not advance its exact checkpoint")
    return index


def discover_checkpoint(
    github: IndexReader,
    scope: CommentScope,
    current: ActionsRun,
    expected_head: CommitSha,
    *,
    explicit: CheckpointRequest | None,
    continuations_remaining: int,
    continuation_key: ContinuationKey | None,
) -> CheckpointDiscovery:
    as_continuation_budget(continuations_remaining)
    if continuation_key is not None:
        selected = validate_continuation(
            github,
            scope,
            current,
            expected_head,
            explicit=explicit,
            continuations_remaining=continuations_remaining,
            continuation_key=continuation_key,
        )
        consumed = _consumed_continuation(
            github, current, selected, continuation_key, continuations_remaining
        )
        if consumed is not None:
            return SupersededFeedback(consumed)
        newer = _discover_index(github, scope, current)
        if newer is not None and (
            (
                _index_order(newer) > _index_order(selected)
                and newer.value.through >= selected.value.through
            )
            or newer.value.continuation_key == continuation_key
        ):
            return SupersededFeedback(newer.value.state)
        return CheckpointSelection(selected.value.state, selected.artifact)
    _require_current_workflow(github, scope, current)
    if explicit is not None:
        return _select_explicit_checkpoint(github, scope, current, explicit)
    selected = _discover_index(github, scope, current)
    if selected is not None:
        return CheckpointSelection(selected.value.state, selected.artifact)
    # The previous consumer used run-scoped state without an index. This bounded
    # migration fallback is never used to recover an unverifiable continuation.
    artifact = find_checkpoint(github, scope, current)
    return (
        CheckpointSelection(artifact) if artifact is not None else BootstrapFeedback()
    )


def validate_selection(
    github: IndexReader,
    scope: CommentScope,
    selection: CheckpointSelection,
    value: object,
) -> RetainedFeedback:
    retained = validate_checkpoint(github, scope, selection.state, value)
    if selection.index is not None:
        index = _load_index(github, scope, selection.index)
        if index.value != FeedbackIndex.from_checkpoint(retained):
            raise ValueError(
                "The downloaded checkpoint does not match its immutable index"
            )
    return retained


def prepare_index(
    github: IndexReader,
    current: ActionsRun,
    state: FeedbackCheckpoint,
    state_artifact: int,
) -> FeedbackIndex:
    identity = github.get_artifact(
        current,
        state_artifact,
        artifact_name(FeedbackArtifactKind.STATE, state.scope, current),
    )
    retained = validate_current_checkpoint(
        github, state.scope, current, identity, state.to_json()
    )
    return FeedbackIndex.from_checkpoint(retained)


def _current_index(
    github: IndexReader,
    current: ActionsRun,
    state: FeedbackCheckpoint,
    state_artifact: int,
    index_artifact: int,
) -> VerifiedFeedbackIndex:
    expected = prepare_index(github, current, state, state_artifact)
    index = _read_index(
        github,
        state.scope,
        github.get_manifest_artifact(
            state.scope.repository, index_artifact, index_name(state.scope, current)
        ),
        current=current,
    )
    if index.value != expected:
        raise ValueError("The uploaded index does not match the published checkpoint")
    return index


def _previous_success(
    github: IndexReader, index: VerifiedFeedbackIndex, name: str
) -> VerifiedFeedbackIndex | None:
    """Preserve a successful prior attempt before replacing its discovery alias."""
    current = index.artifact.source
    manifest = github.find_manifest_artifact(current, name)
    if manifest is None:
        return None
    inspected = _inspect_alias(github, index.value.scope, manifest, expected_name=name)
    prior = inspected.current.artifact.source
    if (
        not prior.same_run(current)
        or prior.attempt >= current.attempt
        or inspected.run.status != "completed"
    ):
        raise ValueError("Cannot replace another active index alias")
    previous = inspected.completed
    if previous is not None and (
        previous.created_at > index.created_at
        or previous.value.continuation_key != index.value.continuation_key
        or previous.value.continuations_remaining != index.value.continuations_remaining
    ):
        raise ValueError("The publisher retry does not continue its earlier attempt")
    return previous


def prepare_index_aliases(
    github: IndexReader,
    current: ActionsRun,
    state: FeedbackCheckpoint,
    state_artifact: int,
    index_artifact: int,
) -> FeedbackIndexAliases:
    index = _current_index(github, current, state, state_artifact, index_artifact)
    prior_discovery = _previous_success(github, index, discovery_name(state.scope))
    discovery = FeedbackIndexAlias(
        scope=state.scope,
        index=index.artifact,
        previous=prior_discovery.artifact if prior_discovery is not None else None,
    )
    consumption = None
    if state.continuation_key is not None:
        prior_marker = _previous_success(
            github, index, marker_name(state.scope, state.continuation_key)
        )
        previous = max(
            (value for value in (prior_discovery, prior_marker) if value is not None),
            key=_index_order,
            default=None,
        )
        consumption = FeedbackIndexAlias(
            scope=state.scope,
            index=index.artifact,
            previous=previous.artifact if previous is not None else None,
        )
    return FeedbackIndexAliases(discovery, consumption)


def continue_feedback(
    github: ContinuationReader,
    writer: ContinuationWriter,
    current: ActionsRun,
    state: FeedbackCheckpoint,
    state_artifact: int,
    index_artifact: int,
    *,
    expected_head: CommitSha,
    checkpoint: CheckpointRequest | None = None,
) -> FeedbackContinuation | None:
    index = _current_index(github, current, state, state_artifact, index_artifact)
    continuation = next_continuation(index)
    if continuation is None:
        return None
    if state.continuation_key is not None:
        parent = validate_continuation(
            github,
            state.scope,
            current,
            expected_head,
            explicit=checkpoint,
            continuations_remaining=state.continuations_remaining,
            continuation_key=state.continuation_key,
        )
        if (
            _consumed_continuation(
                github,
                current,
                parent,
                state.continuation_key,
                state.continuations_remaining,
            )
            is not None
        ):
            logger.info(
                "Another successful publisher consumed this continuation; not dispatching twice"
            )
            return None
    try:
        require_eligible(
            github.get_comment_pull_request(state.scope), state.scope, state.head
        )
    except IneligiblePullRequest:
        logger.info(
            "The pull request moved after its checkpoint; not dispatching a follow-up"
        )
        return None
    writer.dispatch_feedback(continuation)
    return continuation
