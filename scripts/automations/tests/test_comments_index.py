import argparse
import hashlib
import io
import json
import os
import unittest
from dataclasses import dataclass, field, replace
from unittest.mock import patch
from zipfile import ZIP_DEFLATED, ZipFile

from test_comments import (
    BASE,
    CURRENT_RUN,
    EARLIER,
    FUTURE,
    HEAD,
    LATER,
    PRIOR_RUN,
    REPOSITORY,
    SCOPE,
    WORKFLOW_SHA,
    FakeGitHub,
    artifact,
    collection_checkpoint,
    conversation,
    feedback_checkpoint,
    pull_request,
    workflow_run,
)

from uv_automations import cli, comments_cli
from uv_automations.comment_models import (
    MAX_CONTINUATIONS,
    ContinuationKey,
    FeedbackContinuation,
)
from uv_automations.comments_cli import ValidateComments, add_commands, parse_command
from uv_automations.github_actions import (
    ActionsRun,
    ManifestArtifact,
    ManifestPage,
    decode_json_manifest,
)
from uv_automations.github_comments import CommentGitHub
from uv_automations.models import CommitSha, RepositoryIdentity, Timestamp
from uv_automations.workflows.comments import (
    CheckpointLocator,
    FeedbackArtifactKind,
    FeedbackCheckpoint,
    PreparedFeedback,
    RetainedFeedback,
)
from uv_automations.workflows.comments_index import (
    CHECKPOINT_DISCOVERY_ARTIFACTS,
    BootstrapFeedback,
    CheckpointRequest,
    CheckpointSelection,
    FeedbackIndex,
    FeedbackIndexAlias,
    FeedbackIndexAliases,
    SupersededFeedback,
    VerifiedFeedbackIndex,
    continue_feedback,
    discover_checkpoint,
    discovery_name,
    index_name,
    marker_name,
    next_continuation,
    prepare_index,
    prepare_index_aliases,
    read_completed_index,
    validate_continuation,
    validate_selection,
)


def artifact_base(source: ActionsRun) -> int:
    return source.identifier * 1_000 + source.attempt * 10


def state_for(
    source: ActionsRun = PRIOR_RUN,
    *,
    head: CommitSha = HEAD,
    through: Timestamp = EARLIER,
    pending: int = 1,
    progress: int = 1,
    budget: int = MAX_CONTINUATIONS,
    key: ContinuationKey | None = None,
) -> FeedbackCheckpoint:
    return replace(
        feedback_checkpoint(source=source, head=head),
        collection=replace(
            collection_checkpoint(),
            through=through,
            pending=tuple(
                conversation(100 + index).revision for index in range(pending)
            ),
        ),
        preparation=artifact(
            FeedbackArtifactKind.CONTEXT, source, artifact_base(source) + 1
        ),
        result=artifact(FeedbackArtifactKind.RESULT, source, artifact_base(source) + 2),
        session=artifact(
            FeedbackArtifactKind.SESSION, source, artifact_base(source) + 3
        ),
        processed_targets=progress,
        continuations_remaining=budget,
        continuation_key=key,
    )


def manifest_bytes(value: object) -> bytes:
    output = io.BytesIO()
    with ZipFile(output, "w", compression=ZIP_DEFLATED) as archive:
        archive.writestr("manifest.json", json.dumps(value, allow_nan=False))
    return output.getvalue()


@dataclass
class IndexGitHub(FakeGitHub):
    manifests: dict[int, ManifestArtifact] = field(default_factory=dict)
    archives: dict[int, bytes] = field(default_factory=dict)
    manifest_queries: list[tuple[RepositoryIdentity, str, int]] = field(
        default_factory=list
    )
    manifest_reads: list[int] = field(default_factory=list)
    run_manifest_queries: list[tuple[ActionsRun, str]] = field(default_factory=list)
    dispatches: list[FeedbackContinuation] = field(default_factory=list)

    def list_manifest_artifacts(
        self, repository: RepositoryIdentity, name: str, *, limit: int
    ) -> ManifestPage:
        self.manifest_queries.append((repository, name, limit))
        matching = sorted(
            (
                manifest
                for manifest in self.manifests.values()
                if manifest.repository == repository and manifest.name == name
            ),
            key=lambda manifest: (manifest.created_at, manifest.identifier),
            reverse=True,
        )
        return ManifestPage(tuple(matching[:limit]), complete=len(matching) <= limit)

    def get_manifest_artifact(
        self, repository: RepositoryIdentity, identifier: int, name: str
    ) -> ManifestArtifact:
        result = self.manifests[identifier]
        if result.repository != repository or result.name != name:
            raise ValueError("Unexpected manifest provenance")
        return result

    def find_manifest_artifact(
        self, source: ActionsRun, name: str
    ) -> ManifestArtifact | None:
        self.run_manifest_queries.append((source, name))
        matching = [
            manifest
            for manifest in self.manifests.values()
            if manifest.repository == source.repository
            and manifest.run_identifier == source.identifier
            and manifest.workflow_sha == source.workflow_sha
            and manifest.name == name
        ]
        if len(matching) > 1:
            raise ValueError("Expected one run-scoped manifest")
        return matching[0] if matching else None

    def read_json_manifest(self, artifact: ManifestArtifact) -> object:
        self.manifest_reads.append(artifact.identifier)
        return decode_json_manifest(artifact, self.archives[artifact.identifier])

    def dispatch_feedback(self, continuation: FeedbackContinuation) -> None:
        self.dispatches.append(continuation)


def github_for(current: ActionsRun = CURRENT_RUN) -> IndexGitHub:
    return IndexGitHub(workflow_runs={current: workflow_run(current, successful=False)})


def retain_manifest(
    github: IndexGitHub,
    source: ActionsRun,
    name: str,
    value: object,
    *,
    identifier: int,
    created_at: Timestamp,
    overwrite: bool = False,
) -> ManifestArtifact:
    if overwrite:
        for existing in tuple(github.manifests.values()):
            if existing.run_identifier == source.identifier and existing.name == name:
                del github.manifests[existing.identifier]
                del github.archives[existing.identifier]
    content = manifest_bytes(value)
    manifest = ManifestArtifact(
        REPOSITORY,
        identifier,
        name,
        source.identifier,
        source.workflow_sha,
        "sha256:" + hashlib.sha256(content).hexdigest(),
        created_at,
        len(content),
    )
    github.manifests[manifest.identifier] = manifest
    github.archives[manifest.identifier] = content
    return manifest


def retain_alias(
    github: IndexGitHub,
    alias: FeedbackIndexAlias,
    *,
    name: str | None = None,
    identifier: int | None = None,
    created_at: Timestamp = EARLIER,
    overwrite: bool = False,
) -> ManifestArtifact:
    source = alias.index.source
    return retain_manifest(
        github,
        source,
        discovery_name(alias.scope) if name is None else name,
        alias.to_json(),
        identifier=artifact_base(source) + 6 if identifier is None else identifier,
        created_at=created_at,
        overwrite=overwrite,
    )


def retain_index(
    github: IndexGitHub,
    state: FeedbackCheckpoint,
    *,
    created_at: Timestamp = EARLIER,
    identifier: int | None = None,
    discoverable: bool = True,
) -> VerifiedFeedbackIndex:
    state_identity = artifact(
        FeedbackArtifactKind.STATE, state.source, artifact_base(state.source) + 4
    )
    index = FeedbackIndex.from_checkpoint(RetainedFeedback(state_identity, state))
    manifest = retain_manifest(
        github,
        state.source,
        index_name(SCOPE, state.source),
        index.to_json(),
        identifier=identifier
        if identifier is not None
        else artifact_base(state.source) + 5,
        created_at=created_at,
    )
    for value in (state_identity, state.preparation, state.result, state.session):
        github.artifacts[value.identifier] = value
        github.workflow_runs.setdefault(value.source, workflow_run(value.source))
    verified = VerifiedFeedbackIndex(manifest.bind(state.source), created_at, index)
    if discoverable:
        retain_alias(
            github,
            FeedbackIndexAlias(scope=SCOPE, index=verified.artifact, previous=None),
            created_at=created_at,
            overwrite=True,
        )
    return verified


def request_for(continuation: FeedbackContinuation) -> CheckpointRequest:
    return CheckpointRequest(
        CheckpointLocator(
            continuation.checkpoint_run,
            continuation.checkpoint_attempt,
            continuation.checkpoint_artifact,
        ),
        continuation.checkpoint_index,
    )


def retain_marker(
    github: IndexGitHub,
    index: VerifiedFeedbackIndex,
    *,
    identifier: int | None = None,
    previous: VerifiedFeedbackIndex | None = None,
) -> ManifestArtifact:
    key = index.value.continuation_key
    if key is None:
        raise ValueError("A consumption marker needs an exact continuation key")
    return retain_alias(
        github,
        FeedbackIndexAlias(
            scope=index.value.scope,
            index=index.artifact,
            previous=previous.artifact if previous is not None else None,
        ),
        name=marker_name(index.value.scope, key),
        identifier=identifier
        if identifier is not None
        else artifact_base(index.artifact.source) + 7,
        created_at=index.created_at,
        overwrite=True,
    )


def retain_aliases(
    github: IndexGitHub,
    index: VerifiedFeedbackIndex,
    aliases: FeedbackIndexAliases,
) -> None:
    retain_alias(github, aliases.discovery, created_at=index.created_at, overwrite=True)
    if aliases.consumption is not None:
        key = index.value.continuation_key
        if key is None:
            raise ValueError("A consumption alias needs its exact key")
        retain_alias(
            github,
            aliases.consumption,
            name=marker_name(index.value.scope, key),
            identifier=artifact_base(index.artifact.source) + 7,
            created_at=index.created_at,
            overwrite=True,
        )


def discover(
    github: IndexGitHub,
    *,
    current: ActionsRun = CURRENT_RUN,
    expected_head: CommitSha = HEAD,
    explicit: CheckpointRequest | None = None,
    budget: int = MAX_CONTINUATIONS,
    key: ContinuationKey | None = None,
) -> CheckpointSelection | BootstrapFeedback | SupersededFeedback:
    return discover_checkpoint(
        github,
        SCOPE,
        current,
        expected_head,
        explicit=explicit,
        continuations_remaining=budget,
        continuation_key=key,
    )


class CommentIndexTests(unittest.TestCase):
    def test_legacy_checkpoints_resume_without_granting_a_continuation_budget(
        self,
    ) -> None:
        state = state_for()
        legacy = state.to_json()
        legacy["version"] = 1
        for key in ("processed_targets", "continuations_remaining", "continuation_key"):
            del legacy[key]
        restored = FeedbackCheckpoint.from_json(legacy)
        self.assertEqual(restored.collection, state.collection)
        self.assertEqual(restored.session, state.session)
        self.assertEqual(restored.processed_targets, 0)
        self.assertEqual(restored.continuations_remaining, 0)
        self.assertIsNone(restored.continuation_key)

        prepared = PreparedFeedback(
            scope=SCOPE,
            source=PRIOR_RUN,
            dispatch_head=HEAD,
            head=HEAD,
            head_ref="feature",
            base=BASE,
            base_ref="main",
            after=None,
            collection=state.collection,
            targets=(conversation().revision,),
            session_id=None,
            continuation=None,
            processed_targets=1,
            continuations_remaining=MAX_CONTINUATIONS,
            continuation_key=None,
        ).to_json()
        prepared["version"] = 1
        for key in ("processed_targets", "continuations_remaining", "continuation_key"):
            del prepared[key]
        previous = PreparedFeedback.from_json(prepared)
        self.assertEqual(previous.processed_targets, 1)
        self.assertEqual(previous.continuations_remaining, 0)
        self.assertIsNone(previous.continuation_key)

    def test_index_and_selection_round_trip_without_accepting_extra_fields(
        self,
    ) -> None:
        github = github_for()
        index = retain_index(github, state_for())
        selection = CheckpointSelection(index.value.state, index.artifact)
        self.assertEqual(FeedbackIndex.from_json(index.value.to_json()), index.value)
        self.assertEqual(CheckpointSelection.from_json(selection.to_json()), selection)
        self.assertEqual(selection.to_json()["index"], index.artifact.to_json())
        for changed in (
            {**index.value.to_json(), "extra": True},
            {**index.value.to_json(), "pending_targets": True},
            {**index.value.to_json(), "processed_targets": 21},
            {**index.value.to_json(), "continuations_remaining": 4},
            {**index.value.to_json(), "checkpoint_digest": "not-a-digest"},
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                FeedbackIndex.from_json(changed)

    def test_aliases_record_only_exact_indexes_from_the_same_run(self) -> None:
        github = github_for()
        previous = retain_index(github, state_for())
        current = retain_index(
            github,
            state_for(replace(PRIOR_RUN, attempt=2), through=LATER),
            created_at=LATER,
            discoverable=False,
        )
        alias = FeedbackIndexAlias(
            scope=SCOPE,
            index=current.artifact,
            previous=previous.artifact,
        )
        self.assertEqual(FeedbackIndexAlias.from_json(alias.to_json()), alias)
        for changed in (
            {**alias.to_json(), "extra": True},
            {**alias.to_json(), "previous": current.artifact.to_json()},
            {
                **alias.to_json(),
                "previous": replace(previous.artifact, source=CURRENT_RUN).to_json(),
            },
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                FeedbackIndexAlias.from_json(changed)

    def test_parent_retry_preserves_exact_ids_and_its_last_successful_alias(
        self,
    ) -> None:
        github = github_for()
        original = retain_index(github, state_for())
        continuation = next_continuation(original)
        if continuation is None:
            raise AssertionError("Expected a pending continuation")
        original_alias = artifact_base(PRIOR_RUN) + 6

        retry = replace(PRIOR_RUN, attempt=2)
        github.workflow_runs[retry] = workflow_run(retry, successful=False)
        retry_state = state_for(retry, through=LATER)
        retry_index = retain_index(
            github, retry_state, created_at=LATER, discoverable=False
        )
        aliases = prepare_index_aliases(
            github,
            retry,
            retry_state,
            retry_index.value.state.identifier,
            retry_index.artifact.identifier,
        )
        self.assertEqual(aliases.discovery.previous, original.artifact)
        self.assertIsNone(aliases.consumption)
        retain_aliases(github, retry_index, aliases)
        self.assertNotIn(original_alias, github.manifests)
        self.assertIn(original.artifact.identifier, github.manifests)
        self.assertEqual(
            github.artifacts[original.value.state.identifier], original.value.state
        )
        self.assertEqual(
            github.artifacts[original.value.session.identifier], original.value.session
        )
        github.workflow_runs[retry] = replace(workflow_run(retry), conclusion="failure")
        self.assertEqual(
            discover(github),
            CheckpointSelection(original.value.state, original.artifact),
        )
        self.assertEqual(
            validate_continuation(
                github,
                SCOPE,
                CURRENT_RUN,
                HEAD,
                explicit=request_for(continuation),
                continuations_remaining=continuation.remaining,
                continuation_key=continuation.key,
            ),
            original,
        )

        later_retry = replace(PRIOR_RUN, attempt=3)
        github.workflow_runs[later_retry] = workflow_run(later_retry, successful=False)
        later_state = state_for(later_retry, through=FUTURE)
        later_index = retain_index(
            github, later_state, created_at=FUTURE, discoverable=False
        )
        later_aliases = prepare_index_aliases(
            github,
            later_retry,
            later_state,
            later_index.value.state.identifier,
            later_index.artifact.identifier,
        )
        self.assertEqual(later_aliases.discovery.previous, original.artifact)
        retain_aliases(github, later_index, later_aliases)
        github.workflow_runs[later_retry] = workflow_run(later_retry)
        self.assertEqual(
            discover(github),
            CheckpointSelection(later_index.value.state, later_index.artifact),
        )

    def test_alias_replacement_rejects_an_unfinished_earlier_attempt(self) -> None:
        github = github_for()
        original = retain_index(github, state_for())
        github.workflow_runs[PRIOR_RUN] = workflow_run(PRIOR_RUN, successful=False)
        retry = replace(PRIOR_RUN, attempt=2)
        github.workflow_runs[retry] = workflow_run(retry, successful=False)
        state = state_for(retry, through=LATER)
        current = retain_index(github, state, created_at=LATER, discoverable=False)
        with self.assertRaisesRegex(ValueError, "active index alias"):
            prepare_index_aliases(
                github,
                retry,
                state,
                current.value.state.identifier,
                current.artifact.identifier,
            )
        self.assertIn(original.artifact.identifier, github.manifests)

    def test_discovery_prefers_coverage_over_a_late_publisher_retry(self) -> None:
        github = github_for()
        retain_index(github, state_for())
        newer = retain_index(
            github,
            state_for(ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA), through=LATER),
            created_at=LATER,
        )
        retry = replace(PRIOR_RUN, attempt=2)
        github.workflow_runs[retry] = workflow_run(retry, successful=False)
        old_state = state_for(retry, through=EARLIER)
        republished = retain_index(
            github, old_state, created_at=FUTURE, discoverable=False
        )
        aliases = prepare_index_aliases(
            github,
            retry,
            old_state,
            republished.value.state.identifier,
            republished.artifact.identifier,
        )
        retain_aliases(github, republished, aliases)
        github.workflow_runs[retry] = workflow_run(retry)
        self.assertEqual(
            discover(github), CheckpointSelection(newer.value.state, newer.artifact)
        )

    def test_alias_keeps_better_coverage_from_a_successful_earlier_attempt(
        self,
    ) -> None:
        github = github_for()
        original = retain_index(github, state_for(through=LATER), created_at=LATER)
        retry = replace(PRIOR_RUN, attempt=2)
        github.workflow_runs[retry] = workflow_run(retry, successful=False)
        old_state = state_for(retry, through=EARLIER)
        republished = retain_index(
            github, old_state, created_at=FUTURE, discoverable=False
        )
        aliases = prepare_index_aliases(
            github,
            retry,
            old_state,
            republished.value.state.identifier,
            republished.artifact.identifier,
        )
        self.assertEqual(aliases.discovery.previous, original.artifact)
        retain_aliases(github, republished, aliases)
        github.workflow_runs[retry] = workflow_run(retry)
        self.assertEqual(
            discover(github),
            CheckpointSelection(original.value.state, original.artifact),
        )

    def test_index_cannot_claim_a_watermark_after_its_artifact(self) -> None:
        github = github_for()
        index = retain_index(github, state_for(through=LATER), created_at=EARLIER)
        with self.assertRaisesRegex(ValueError, "provenance"):
            read_completed_index(
                github, SCOPE, github.manifests[index.artifact.identifier]
            )

    def test_per_pull_request_discovery_ignores_unrelated_workflow_traffic(
        self,
    ) -> None:
        github = github_for()
        selected = retain_index(github, state_for())
        older = retain_index(
            github,
            state_for(ActionsRun(REPOSITORY, 90, 1, WORKFLOW_SHA)),
            created_at=EARLIER.overlap(),
        )
        github.runs = tuple(
            workflow_run(ActionsRun(REPOSITORY, number, 1, WORKFLOW_SHA))
            for number in range(150, 180)
        )
        self.assertEqual(
            discover(github),
            CheckpointSelection(selected.value.state, selected.artifact),
        )
        self.assertEqual(
            github.manifest_queries,
            [(REPOSITORY, discovery_name(SCOPE), CHECKPOINT_DISCOVERY_ARTIFACTS)],
        )
        self.assertEqual(
            github.manifest_reads,
            [artifact_base(selected.artifact.source) + 6, selected.artifact.identifier],
        )
        self.assertNotIn(older.artifact.identifier, github.manifest_reads)
        self.assertEqual(github.run_queries, [])

    def test_bad_indexes_fall_back_to_a_complete_bounded_bootstrap(self) -> None:
        github = github_for()
        for number in range(CHECKPOINT_DISCOVERY_ARTIFACTS + 1):
            source = ActionsRun(REPOSITORY, 100 + number, 1, WORKFLOW_SHA)
            index = retain_index(github, state_for(source), identifier=3_000 + number)
            github.archives[index.artifact.identifier] = b"not the verified archive"
        self.assertIsInstance(discover(github), BootstrapFeedback)
        self.assertEqual(len(github.manifest_reads), CHECKPOINT_DISCOVERY_ARTIFACTS * 2)
        self.assertEqual(len(github.run_queries), 1)

    def test_discovery_rejects_wrong_workflow_attempt_and_session_identity(
        self,
    ) -> None:
        for failure in ("workflow", "failed", "attempt", "session"):
            github = github_for()
            index = retain_index(github, state_for())
            manifest = github.manifests[index.artifact.identifier]
            match failure:
                case "workflow":
                    github.workflow_runs[PRIOR_RUN] = replace(
                        workflow_run(PRIOR_RUN), event="pull_request"
                    )
                case "failed":
                    github.workflow_runs[PRIOR_RUN] = replace(
                        workflow_run(PRIOR_RUN), conclusion="failure"
                    )
                case "attempt":
                    github.workflow_runs[PRIOR_RUN] = replace(
                        workflow_run(PRIOR_RUN), started_at=LATER
                    )
                case "session":
                    github.artifacts[index.value.session.identifier] = replace(
                        index.value.session, digest="sha256:" + "0" * 64
                    )
            with self.subTest(failure=failure), self.assertRaises(ValueError):
                read_completed_index(github, SCOPE, manifest)

    def test_full_checkpoint_must_match_the_index_digest(self) -> None:
        github = github_for()
        state = state_for()
        index = retain_index(github, state)
        selection = CheckpointSelection(index.value.state, index.artifact)
        self.assertEqual(
            validate_selection(github, SCOPE, selection, state.to_json()).state,
            state,
        )
        changed = replace(
            state,
            collection=replace(state.collection, pending=(conversation(999).revision,)),
        )
        with self.assertRaisesRegex(ValueError, "does not match"):
            validate_selection(github, SCOPE, selection, changed.to_json())

    def test_current_publisher_can_build_but_not_discover_its_index(self) -> None:
        github = github_for()
        state = state_for(CURRENT_RUN)
        index = retain_index(github, state)
        self.assertEqual(
            prepare_index(github, CURRENT_RUN, state, index.value.state.identifier),
            index.value,
        )
        with self.assertRaisesRegex(ValueError, "publisher attempt"):
            read_completed_index(
                github, SCOPE, github.manifests[index.artifact.identifier]
            )
        with self.assertRaises(ValueError):
            prepare_index(github, PRIOR_RUN, state, index.value.state.identifier)


class CommentContinuationTests(unittest.TestCase):
    def test_new_stages_use_the_central_comments_dispatcher(self) -> None:
        environment = {
            "GITHUB_REPOSITORY": str(REPOSITORY.name),
            "GITHUB_REPOSITORY_ID": str(REPOSITORY.database_id),
            "PULL_REQUEST_NUMBER": str(SCOPE.number),
            "EXPECTED_HEAD_SHA": str(HEAD),
            "GITHUB_RUN_ID": str(CURRENT_RUN.identifier),
            "GITHUB_RUN_ATTEMPT": str(CURRENT_RUN.attempt),
            "GITHUB_WORKFLOW_SHA": str(WORKFLOW_SHA),
        }
        for stage, arguments, kind in (
            (
                "index-aliases",
                [
                    "--index-artifact",
                    "2",
                    "--destination",
                    "aliases",
                ],
                comments_cli.WriteIndexAliases,
            ),
            (
                "continue",
                ["--index-artifact", "2", "--summary", "summary"],
                comments_cli.ContinueComments,
            ),
        ):
            with (
                self.subTest(stage=stage),
                patch.dict(os.environ, environment, clear=True),
            ):
                command = cli.parse_command(
                    cli.create_parser(),
                    [
                        "comments",
                        stage,
                        "--state",
                        "state",
                        "--state-artifact",
                        "1",
                        "--github-output",
                        "output",
                        *arguments,
                    ],
                )
            self.assertIsInstance(command, kind)
            with patch("uv_automations.comments_cli.run") as dispatch:
                cli.run(command)
            dispatch.assert_called_once_with(command)

    def test_continuation_key_covers_exact_immutable_parent_and_budget(self) -> None:
        github = github_for()
        index = retain_index(github, state_for())
        continuation = next_continuation(index)
        self.assertIsNotNone(continuation)
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        self.assertEqual(continuation, next_continuation(index))
        self.assertEqual(continuation.remaining, MAX_CONTINUATIONS - 1)
        for changed in (
            replace(index, artifact=replace(index.artifact, identifier=9_999)),
            replace(
                index, artifact=replace(index.artifact, digest="sha256:" + "0" * 64)
            ),
            replace(index, value=replace(index.value, head=BASE)),
            replace(index, value=replace(index.value, continuations_remaining=2)),
        ):
            with self.subTest(changed=changed):
                self.assertNotEqual(next_continuation(changed), continuation)

    def test_empty_or_nonprogressing_checkpoints_never_continue(self) -> None:
        github = github_for()
        for changed in (
            state_for(pending=0),
            state_for(progress=0),
            state_for(budget=0),
        ):
            with self.subTest(changed=changed):
                self.assertIsNone(next_continuation(retain_index(github, changed)))
        count = 0
        budget = MAX_CONTINUATIONS
        key = None
        while True:
            source = ActionsRun(REPOSITORY, 100 + count, 1, WORKFLOW_SHA)
            index = retain_index(github, state_for(source, budget=budget, key=key))
            continuation = next_continuation(index)
            if continuation is None:
                break
            count += 1
            budget = continuation.remaining
            key = continuation.key
        self.assertEqual(count, MAX_CONTINUATIONS)

    def test_explicit_continuation_cannot_change_its_head_ids_or_budget(self) -> None:
        github = github_for()
        index = retain_index(github, state_for())
        continuation = next_continuation(index)
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        request = request_for(continuation)
        self.assertEqual(
            discover(
                github,
                explicit=request,
                budget=continuation.remaining,
                key=continuation.key,
            ),
            CheckpointSelection(index.value.state, index.artifact),
        )
        for head, selected, budget, key in (
            (BASE, request, continuation.remaining, continuation.key),
            (
                HEAD,
                replace(
                    request,
                    checkpoint=replace(
                        request.checkpoint,
                        artifact_id=request.checkpoint.artifact_id + 1,
                    ),
                ),
                continuation.remaining,
                continuation.key,
            ),
            (HEAD, request, continuation.remaining + 1, continuation.key),
            (HEAD, request, continuation.remaining, ContinuationKey("0" * 64)),
            (
                HEAD,
                replace(request, index_id=continuation.checkpoint_index + 1),
                continuation.remaining,
                continuation.key,
            ),
            (
                HEAD,
                replace(request, index_id=None),
                continuation.remaining,
                continuation.key,
            ),
        ):
            with (
                self.subTest(head=head, selected=selected, budget=budget, key=key),
                self.assertRaises(ValueError),
            ):
                discover(
                    github,
                    expected_head=head,
                    explicit=selected,
                    budget=budget,
                    key=key,
                )

    def test_newer_completed_checkpoint_supersedes_a_duplicate_continuation(
        self,
    ) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        continuation = next_continuation(parent)
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        child_source = ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA)
        child = retain_index(
            github,
            state_for(
                child_source,
                through=LATER,
                budget=continuation.remaining,
                key=continuation.key,
            ),
            created_at=LATER,
        )
        request = request_for(continuation)
        self.assertEqual(
            discover(
                github,
                explicit=request,
                budget=continuation.remaining,
                key=continuation.key,
            ),
            SupersededFeedback(child.value.state),
        )
        github.manifest_queries.clear()
        self.assertEqual(
            validate_continuation(
                github,
                SCOPE,
                CURRENT_RUN,
                HEAD,
                explicit=request,
                continuations_remaining=continuation.remaining,
                continuation_key=continuation.key,
            ),
            parent,
        )
        self.assertEqual(github.manifest_queries, [])

    def test_failed_older_publisher_does_not_fork_a_consumed_key(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        incoming = next_continuation(parent)
        if incoming is None:
            raise AssertionError("Expected a pending continuation")
        first_attempt = replace(CURRENT_RUN, attempt=1)
        github.workflow_runs[first_attempt] = replace(
            workflow_run(first_attempt), conclusion="failure"
        )
        failed_state = state_for(
            first_attempt,
            through=LATER,
            budget=incoming.remaining,
            key=incoming.key,
        )
        sibling = retain_index(
            github,
            state_for(
                ActionsRun(REPOSITORY, 300, 1, WORKFLOW_SHA),
                through=LATER,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            created_at=LATER,
        )
        retain_marker(github, sibling)
        retry_state = replace(
            state_for(
                CURRENT_RUN,
                through=LATER,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            preparation=failed_state.preparation,
            result=failed_state.result,
            session=failed_state.session,
        )
        retry_index = retain_index(
            github, retry_state, created_at=LATER, discoverable=False
        )
        aliases = prepare_index_aliases(
            github,
            CURRENT_RUN,
            retry_state,
            retry_index.value.state.identifier,
            retry_index.artifact.identifier,
        )
        retain_aliases(github, retry_index, aliases)
        request = request_for(incoming)

        # A failed-job retry may still finish the exact original publication.
        self.assertEqual(
            validate_continuation(
                github,
                SCOPE,
                CURRENT_RUN,
                HEAD,
                explicit=request,
                continuations_remaining=incoming.remaining,
                continuation_key=incoming.key,
            ),
            parent,
        )
        self.assertNotEqual(next_continuation(retry_index), next_continuation(sibling))
        self.assertIsNone(
            continue_feedback(
                github,
                github,
                CURRENT_RUN,
                retry_state,
                retry_index.value.state.identifier,
                retry_index.artifact.identifier,
                expected_head=HEAD,
                checkpoint=request,
            )
        )
        self.assertEqual(github.dispatches, [])
        self.assertEqual(
            discover(
                github,
                explicit=request,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            SupersededFeedback(sibling.value.state),
        )

    def test_successful_consumption_survives_a_failed_same_run_retry(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        incoming = next_continuation(parent)
        if incoming is None:
            raise AssertionError("Expected a pending continuation")
        original_source = replace(CURRENT_RUN, attempt=1)
        original = retain_index(
            github,
            state_for(
                original_source,
                through=LATER,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            created_at=LATER,
        )
        original_marker = retain_marker(github, original)
        state = state_for(
            CURRENT_RUN,
            through=FUTURE,
            budget=incoming.remaining,
            key=incoming.key,
        )
        index = retain_index(github, state, created_at=FUTURE, discoverable=False)
        aliases = prepare_index_aliases(
            github,
            CURRENT_RUN,
            state,
            index.value.state.identifier,
            index.artifact.identifier,
        )
        self.assertIsNotNone(aliases.consumption)
        if aliases.consumption is None:
            raise AssertionError("Expected a consumption pointer")
        self.assertEqual(aliases.consumption.previous, original.artifact)
        retain_aliases(github, index, aliases)
        self.assertNotIn(original_marker.identifier, github.manifests)
        self.assertIn(original.artifact.identifier, github.manifests)
        self.assertIsNone(
            continue_feedback(
                github,
                github,
                CURRENT_RUN,
                state,
                index.value.state.identifier,
                index.artifact.identifier,
                expected_head=HEAD,
                checkpoint=request_for(incoming),
            )
        )
        github.workflow_runs[CURRENT_RUN] = replace(
            workflow_run(CURRENT_RUN), conclusion="failure"
        )
        duplicate = ActionsRun(REPOSITORY, 300, 1, WORKFLOW_SHA)
        github.workflow_runs[duplicate] = workflow_run(duplicate, successful=False)
        self.assertEqual(
            discover(
                github,
                current=duplicate,
                explicit=request_for(incoming),
                budget=incoming.remaining,
                key=incoming.key,
            ),
            SupersededFeedback(original.value.state),
        )
        github.workflow_runs[original_source] = replace(
            workflow_run(original_source), conclusion="failure"
        )
        with self.assertRaisesRegex(ValueError, "discovery bound"):
            discover(
                github,
                current=duplicate,
                explicit=request_for(incoming),
                budget=incoming.remaining,
                key=incoming.key,
            )

    def test_current_in_progress_marker_does_not_block_its_first_dispatch(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        incoming = next_continuation(parent)
        if incoming is None:
            raise AssertionError("Expected a pending continuation")
        state = state_for(
            CURRENT_RUN,
            through=LATER,
            budget=incoming.remaining,
            key=incoming.key,
        )
        index = retain_index(github, state, created_at=LATER, discoverable=False)
        aliases = prepare_index_aliases(
            github,
            CURRENT_RUN,
            state,
            index.value.state.identifier,
            index.artifact.identifier,
        )
        retain_aliases(github, index, aliases)
        continuation = continue_feedback(
            github,
            github,
            CURRENT_RUN,
            state,
            index.value.state.identifier,
            index.artifact.identifier,
            expected_head=HEAD,
            checkpoint=request_for(incoming),
        )
        self.assertIsNotNone(continuation)
        self.assertEqual(github.dispatches, [continuation])

    def test_exact_key_marker_survives_unrelated_per_pull_request_indexes(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        continuation = next_continuation(parent)
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        child = retain_index(
            github,
            state_for(
                ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA),
                through=LATER,
                budget=continuation.remaining,
                key=continuation.key,
            ),
            created_at=LATER,
        )
        retain_marker(github, child)
        for number in range(160, 185):
            unrelated = retain_index(
                github,
                state_for(ActionsRun(REPOSITORY, number, 1, WORKFLOW_SHA)),
                created_at=LATER,
            )
            github.archives[unrelated.artifact.identifier] = b"unusable newer index"
        self.assertEqual(
            discover(
                github,
                explicit=request_for(continuation),
                budget=continuation.remaining,
                key=continuation.key,
            ),
            SupersededFeedback(child.value.state),
        )
        self.assertEqual(
            github.manifest_queries,
            [
                (
                    REPOSITORY,
                    marker_name(SCOPE, continuation.key),
                    CHECKPOINT_DISCOVERY_ARTIFACTS,
                )
            ],
        )

    def test_incomplete_exact_key_marker_history_fails_closed(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        continuation = next_continuation(parent)
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        for number in range(120, 120 + CHECKPOINT_DISCOVERY_ARTIFACTS + 1):
            child = retain_index(
                github,
                state_for(
                    ActionsRun(REPOSITORY, number, 1, WORKFLOW_SHA),
                    budget=continuation.remaining,
                    key=continuation.key,
                ),
            )
            marker = retain_marker(github, child)
            github.archives[marker.identifier] = b"unusable marker"
        with self.assertRaisesRegex(ValueError, "discovery bound"):
            discover(
                github,
                explicit=request_for(continuation),
                budget=continuation.remaining,
                key=continuation.key,
            )

    def test_publisher_dispatches_only_after_rechecking_exact_checkpoint_head(
        self,
    ) -> None:
        github = github_for()
        state = state_for(CURRENT_RUN)
        index = retain_index(github, state, created_at=LATER)
        continuation = continue_feedback(
            github,
            github,
            CURRENT_RUN,
            state,
            index.value.state.identifier,
            index.artifact.identifier,
            expected_head=HEAD,
        )
        self.assertEqual(github.dispatches, [continuation])
        self.assertIsNotNone(continuation)
        github.dispatches.clear()
        github.pull_request = pull_request(BASE)
        self.assertIsNone(
            continue_feedback(
                github,
                github,
                CURRENT_RUN,
                state,
                index.value.state.identifier,
                index.artifact.identifier,
                expected_head=HEAD,
            )
        )
        self.assertEqual(github.dispatches, [])

    def test_github_writer_has_only_the_fixed_same_workflow_dispatch_operation(
        self,
    ) -> None:
        github = github_for()
        continuation = next_continuation(retain_index(github, state_for()))
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        with patch.object(CommentGitHub, "_api", return_value=None) as api:
            CommentGitHub().dispatch_feedback(continuation)
        api.assert_called_once_with(
            "POST",
            "repos/astral-sh/uv-dev/actions/workflows/pull-request-comments.yml/dispatches",
            payload={"ref": "main", "inputs": continuation.inputs()},
        )

    def test_publisher_cli_reads_exact_continuation_inputs_for_revalidation(
        self,
    ) -> None:
        github = github_for()
        continuation = next_continuation(retain_index(github, state_for()))
        if continuation is None:
            raise AssertionError("Expected one pending feedback continuation")
        environment = {
            "GITHUB_REPOSITORY": str(REPOSITORY.name),
            "GITHUB_REPOSITORY_ID": str(REPOSITORY.database_id),
            "PULL_REQUEST_NUMBER": str(SCOPE.number),
            "EXPECTED_HEAD_SHA": str(HEAD),
            "GITHUB_RUN_ID": str(CURRENT_RUN.identifier),
            "GITHUB_RUN_ATTEMPT": str(CURRENT_RUN.attempt),
            "GITHUB_WORKFLOW_SHA": str(WORKFLOW_SHA),
            "CHECKPOINT_RUN": str(continuation.checkpoint_run),
            "CHECKPOINT_ATTEMPT": str(continuation.checkpoint_attempt),
            "CHECKPOINT_ARTIFACT": str(continuation.checkpoint_artifact),
            "CHECKPOINT_INDEX": str(continuation.checkpoint_index),
            "CONTINUATION_KEY": str(continuation.key),
            "CONTINUATIONS_REMAINING": str(continuation.remaining),
        }
        with patch.dict(os.environ, environment):
            parser = argparse.ArgumentParser()
            add_commands(parser)
            command = parse_command(
                parser.parse_args(
                    [
                        "validate",
                        "--repository",
                        "repository",
                        "--prepared",
                        "context",
                        "--result",
                        "result",
                        "--session",
                        "session",
                        "--trusted-root",
                        "root",
                        "--preparation-artifact",
                        "1",
                        "--result-artifact",
                        "2",
                        "--session-artifact",
                        "3",
                        "--github-output",
                        "output",
                    ]
                )
            )
        self.assertIsInstance(command, ValidateComments)
        if not isinstance(command, ValidateComments):
            raise TypeError("Expected the publication validation command")
        self.assertEqual(command.inputs.context.checkpoint, request_for(continuation))
        self.assertEqual(command.inputs.context.continuation_key, continuation.key)
        self.assertEqual(
            command.inputs.context.continuations_remaining, continuation.remaining
        )
