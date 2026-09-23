"""Thin Actions adapters for pull request comment handling."""

import argparse
import json
import logging
import os
import re
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import append_summary, write_json_output, write_output
from uv_automations.comment_models import (
    MAX_CONTINUATIONS,
    CommentRecommendation,
    CommentScope,
    ContinuationKey,
    as_continuation_budget,
)
from uv_automations.git import Git
from uv_automations.github_actions import MANIFEST_FILENAME, ActionsRun
from uv_automations.github_comments import CommentGitHub
from uv_automations.json import loads
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.sessions import (
    CodexSessionSnapshot,
    snapshot_sessions,
    write_sessions,
)
from uv_automations.workflows.comments import (
    CheckpointLocator,
    FeedbackArtifactKind,
    FeedbackCheckpoint,
    FeedbackPublication,
    IneligiblePullRequest,
    PreparedFeedback,
    RetainedFeedback,
    apply_publication,
    artifact_name,
    persist_feedback_result,
    prepare_agent,
    prepare_feedback,
    prepare_publication,
    read_json_file,
    write_json_file,
)
from uv_automations.workflows.comments_index import (
    BootstrapFeedback,
    CheckpointRequest,
    CheckpointSelection,
    SupersededFeedback,
    continue_feedback,
    discover_checkpoint,
    discovery_name,
    index_name,
    marker_name,
    prepare_index,
    prepare_index_aliases,
    validate_continuation,
    validate_selection,
)

logger = logging.getLogger(__name__)


class CommentsCommandKind(StrEnum):
    SOURCE = "comments.source"
    INSPECT_SOURCE = "comments.inspect-source"
    PREPARE = "comments.prepare"
    PREPARE_AGENT = "comments.prepare-agent"
    VERIFY = "comments.verify"
    VALIDATE = "comments.validate"
    APPLY = "comments.apply"
    INDEX = "comments.index"
    INDEX_ALIASES = "comments.index-aliases"
    CONTINUE = "comments.continue"
    SCHEMA = "comments.schema"


@dataclass(frozen=True, slots=True)
class RunContext:
    scope: CommentScope
    source: ActionsRun
    expected_head: CommitSha
    continuations_remaining: int
    continuation_key: ContinuationKey | None
    checkpoint: CheckpointRequest | None


@dataclass(frozen=True, slots=True, kw_only=True)
class FindSource:
    context: RunContext
    destination: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class InspectSource:
    context: RunContext
    source_file: Path
    checkpoint: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class PrepareComments:
    context: RunContext
    repository: Path
    trusted_files: Path
    trusted_root: Path
    destination: Path
    source_file: Path
    checkpoint: Path
    sessions: Path
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class PrepareAgent:
    run_context: RunContext
    context: Path
    repository: Path
    codex_home: Path
    trusted_root: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class VerifyComments:
    run_context: RunContext
    context: Path
    repository: Path
    codex_home: Path
    trusted_root: Path
    destination: Path
    session_destination: Path
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class PublicationInputs:
    context: RunContext
    repository: Path
    prepared: Path
    result: Path
    session: Path
    trusted_root: Path
    preparation_artifact: int
    result_artifact: int
    session_artifact: int


@dataclass(frozen=True, slots=True, kw_only=True)
class ValidateComments:
    inputs: PublicationInputs
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ApplyComments:
    inputs: PublicationInputs
    destination: Path
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class WriteIndex:
    context: RunContext
    state: Path
    state_artifact: int
    destination: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class WriteIndexAliases:
    context: RunContext
    state: Path
    state_artifact: int
    index_artifact: int
    destination: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class ContinueComments:
    context: RunContext
    state: Path
    state_artifact: int
    index_artifact: int
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class WriteSchema:
    destination: Path | None


type CommentsCommand = (
    FindSource
    | InspectSource
    | PrepareComments
    | PrepareAgent
    | VerifyComments
    | ValidateComments
    | ApplyComments
    | WriteIndex
    | WriteIndexAliases
    | ContinueComments
    | WriteSchema
)


def _positive_integer(value: str) -> int:
    if re.fullmatch(r"[1-9][0-9]*", value) is None:
        raise ValueError("Expected a positive integer")
    return int(value)


def _optional_integer(value: str) -> int | None:
    return _positive_integer(value) if value else None


def _continuation_budget(value: str) -> int:
    if re.fullmatch(r"0|[1-9][0-9]*", value) is None:
        raise ValueError("Expected a nonnegative continuation budget")
    return as_continuation_budget(int(value))


def _optional_continuation_key(value: str) -> ContinuationKey | None:
    return ContinuationKey(value) if value else None


def _add_run_context(parser: argparse.ArgumentParser) -> None:
    for argument, variable, value_type in (
        ("--repo", "GITHUB_REPOSITORY", RepositoryName),
        ("--repository-id", "GITHUB_REPOSITORY_ID", _positive_integer),
        ("--pull-request", "PULL_REQUEST_NUMBER", _positive_integer),
        ("--expected-head", "EXPECTED_HEAD_SHA", CommitSha),
        ("--run-id", "GITHUB_RUN_ID", _positive_integer),
        ("--run-attempt", "GITHUB_RUN_ATTEMPT", _positive_integer),
        ("--workflow-sha", "GITHUB_WORKFLOW_SHA", CommitSha),
    ):
        default = os.environ.get(variable)
        parser.add_argument(
            argument, type=value_type, default=default, required=default is None
        )
    parser.add_argument(
        "--continuations-remaining",
        type=_continuation_budget,
        default=os.environ.get("CONTINUATIONS_REMAINING", str(MAX_CONTINUATIONS)),
    )
    parser.add_argument(
        "--continuation-key",
        type=_optional_continuation_key,
        default=os.environ.get("CONTINUATION_KEY", ""),
    )
    for argument, variable in (
        ("--checkpoint-run", "CHECKPOINT_RUN"),
        ("--checkpoint-attempt", "CHECKPOINT_ATTEMPT"),
        ("--checkpoint-artifact", "CHECKPOINT_ARTIFACT"),
        ("--checkpoint-index", "CHECKPOINT_INDEX"),
    ):
        parser.add_argument(
            argument, type=_optional_integer, default=os.environ.get(variable, "")
        )


def _add_publication_inputs(parser: argparse.ArgumentParser) -> None:
    _add_run_context(parser)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--prepared", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--session", type=Path, required=True)
    parser.add_argument("--trusted-root", type=Path, required=True)
    parser.add_argument("--preparation-artifact", type=_positive_integer, required=True)
    parser.add_argument("--result-artifact", type=_positive_integer, required=True)
    parser.add_argument("--session-artifact", type=_positive_integer, required=True)


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)

    source = commands.add_parser("source")
    source.set_defaults(command=CommentsCommandKind.SOURCE)
    _add_run_context(source)
    source.add_argument("--destination", type=Path, required=True)
    source.add_argument("--github-output", type=Path, required=True)

    inspect = commands.add_parser("inspect-source")
    inspect.set_defaults(command=CommentsCommandKind.INSPECT_SOURCE)
    _add_run_context(inspect)
    inspect.add_argument("--source-file", type=Path, required=True)
    inspect.add_argument("--checkpoint", type=Path, required=True)
    inspect.add_argument("--github-output", type=Path, required=True)

    prepare = commands.add_parser("prepare")
    prepare.set_defaults(command=CommentsCommandKind.PREPARE)
    _add_run_context(prepare)
    prepare.add_argument("--repository", type=Path, required=True)
    prepare.add_argument("--trusted-files", type=Path, required=True)
    prepare.add_argument("--trusted-root", type=Path, required=True)
    prepare.add_argument("--destination", type=Path, required=True)
    prepare.add_argument("--source-file", type=Path, required=True)
    prepare.add_argument("--checkpoint", type=Path, required=True)
    prepare.add_argument("--sessions", type=Path, required=True)
    prepare.add_argument("--github-output", type=Path, required=True)
    prepare.add_argument("--summary", type=Path, required=True)

    agent = commands.add_parser("prepare-agent")
    agent.set_defaults(command=CommentsCommandKind.PREPARE_AGENT)
    _add_run_context(agent)
    agent.add_argument("--context", type=Path, required=True)
    agent.add_argument("--repository", type=Path, required=True)
    agent.add_argument("--codex-home", type=Path, required=True)
    agent.add_argument("--trusted-root", type=Path, required=True)
    agent.add_argument("--github-output", type=Path, required=True)

    verify = commands.add_parser("verify")
    verify.set_defaults(command=CommentsCommandKind.VERIFY)
    _add_run_context(verify)
    verify.add_argument("--context", type=Path, required=True)
    verify.add_argument("--repository", type=Path, required=True)
    verify.add_argument("--codex-home", type=Path, required=True)
    verify.add_argument("--trusted-root", type=Path, required=True)
    verify.add_argument("--destination", type=Path, required=True)
    verify.add_argument("--session-destination", type=Path, required=True)
    verify.add_argument("--github-output", type=Path, required=True)
    verify.add_argument("--summary", type=Path, required=True)

    validate = commands.add_parser("validate")
    validate.set_defaults(command=CommentsCommandKind.VALIDATE)
    _add_publication_inputs(validate)
    validate.add_argument("--github-output", type=Path, required=True)

    apply = commands.add_parser("apply")
    apply.set_defaults(command=CommentsCommandKind.APPLY)
    _add_publication_inputs(apply)
    apply.add_argument("--destination", type=Path, required=True)
    apply.add_argument("--github-output", type=Path, required=True)
    apply.add_argument("--summary", type=Path, required=True)

    index = commands.add_parser("index")
    index.set_defaults(command=CommentsCommandKind.INDEX)
    _add_run_context(index)
    index.add_argument("--state", type=Path, required=True)
    index.add_argument("--state-artifact", type=_positive_integer, required=True)
    index.add_argument("--destination", type=Path, required=True)
    index.add_argument("--github-output", type=Path, required=True)

    aliases = commands.add_parser("index-aliases")
    aliases.set_defaults(command=CommentsCommandKind.INDEX_ALIASES)
    _add_run_context(aliases)
    aliases.add_argument("--state", type=Path, required=True)
    aliases.add_argument("--state-artifact", type=_positive_integer, required=True)
    aliases.add_argument("--index-artifact", type=_positive_integer, required=True)
    aliases.add_argument("--destination", type=Path, required=True)
    aliases.add_argument("--github-output", type=Path, required=True)

    continuation = commands.add_parser("continue")
    continuation.set_defaults(command=CommentsCommandKind.CONTINUE)
    _add_run_context(continuation)
    continuation.add_argument("--state", type=Path, required=True)
    continuation.add_argument("--state-artifact", type=_positive_integer, required=True)
    continuation.add_argument("--index-artifact", type=_positive_integer, required=True)
    continuation.add_argument("--github-output", type=Path, required=True)
    continuation.add_argument("--summary", type=Path, required=True)

    schema = commands.add_parser("schema")
    schema.set_defaults(command=CommentsCommandKind.SCHEMA)
    schema.add_argument("--destination", type=Path)


def _checkpoint_request(parsed: argparse.Namespace) -> CheckpointRequest | None:
    values = (
        parsed.checkpoint_run,
        parsed.checkpoint_attempt,
        parsed.checkpoint_artifact,
    )
    if parsed.checkpoint_index is None and all(value is None for value in values):
        return None
    if any(value is None for value in values):
        raise ValueError(
            "An explicit checkpoint requires run, attempt, and artifact IDs"
        )
    return CheckpointRequest(
        checkpoint=CheckpointLocator(int(values[0]), int(values[1]), int(values[2])),
        index_id=parsed.checkpoint_index,
    )


def _run_context(parsed: argparse.Namespace) -> RunContext:
    repository = RepositoryIdentity(parsed.repo, parsed.repository_id)
    return RunContext(
        CommentScope(repository, parsed.pull_request),
        ActionsRun(repository, parsed.run_id, parsed.run_attempt, parsed.workflow_sha),
        parsed.expected_head,
        parsed.continuations_remaining,
        parsed.continuation_key,
        _checkpoint_request(parsed),
    )


def _publication_inputs(parsed: argparse.Namespace) -> PublicationInputs:
    return PublicationInputs(
        context=_run_context(parsed),
        repository=parsed.repository,
        prepared=parsed.prepared,
        result=parsed.result,
        session=parsed.session,
        trusted_root=parsed.trusted_root,
        preparation_artifact=parsed.preparation_artifact,
        result_artifact=parsed.result_artifact,
        session_artifact=parsed.session_artifact,
    )


def parse_command(parsed: argparse.Namespace) -> CommentsCommand:
    kind = CommentsCommandKind(parsed.command)
    match kind:
        case CommentsCommandKind.SOURCE:
            return FindSource(
                context=_run_context(parsed),
                destination=parsed.destination,
                github_output=parsed.github_output,
            )
        case CommentsCommandKind.INSPECT_SOURCE:
            return InspectSource(
                context=_run_context(parsed),
                source_file=parsed.source_file,
                checkpoint=parsed.checkpoint,
                github_output=parsed.github_output,
            )
        case CommentsCommandKind.PREPARE:
            return PrepareComments(
                context=_run_context(parsed),
                repository=parsed.repository,
                trusted_files=parsed.trusted_files,
                trusted_root=parsed.trusted_root,
                destination=parsed.destination,
                source_file=parsed.source_file,
                checkpoint=parsed.checkpoint,
                sessions=parsed.sessions,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommentsCommandKind.PREPARE_AGENT:
            return PrepareAgent(
                run_context=_run_context(parsed),
                context=parsed.context,
                repository=parsed.repository,
                codex_home=parsed.codex_home,
                trusted_root=parsed.trusted_root,
                github_output=parsed.github_output,
            )
        case CommentsCommandKind.VERIFY:
            return VerifyComments(
                run_context=_run_context(parsed),
                context=parsed.context,
                repository=parsed.repository,
                codex_home=parsed.codex_home,
                trusted_root=parsed.trusted_root,
                destination=parsed.destination,
                session_destination=parsed.session_destination,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommentsCommandKind.VALIDATE:
            return ValidateComments(
                inputs=_publication_inputs(parsed), github_output=parsed.github_output
            )
        case CommentsCommandKind.APPLY:
            return ApplyComments(
                inputs=_publication_inputs(parsed),
                destination=parsed.destination,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommentsCommandKind.INDEX:
            return WriteIndex(
                context=_run_context(parsed),
                state=parsed.state,
                state_artifact=parsed.state_artifact,
                destination=parsed.destination,
                github_output=parsed.github_output,
            )
        case CommentsCommandKind.INDEX_ALIASES:
            return WriteIndexAliases(
                context=_run_context(parsed),
                state=parsed.state,
                state_artifact=parsed.state_artifact,
                index_artifact=parsed.index_artifact,
                destination=parsed.destination,
                github_output=parsed.github_output,
            )
        case CommentsCommandKind.CONTINUE:
            return ContinueComments(
                context=_run_context(parsed),
                state=parsed.state,
                state_artifact=parsed.state_artifact,
                index_artifact=parsed.index_artifact,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommentsCommandKind.SCHEMA:
            return WriteSchema(destination=parsed.destination)
    assert_never(kind)


def _checkpoint(
    github: CommentGitHub, context: RunContext, source_file: Path, checkpoint: Path
) -> RetainedFeedback | None:
    try:
        value = read_json_file(source_file)
        if value is None:
            if context.continuation_key is not None:
                raise ValueError("An automatic continuation is missing its checkpoint")
            return None
        selection = CheckpointSelection.from_json(value)
        if (
            context.continuation_key is not None
            and selection.index
            != validate_continuation(
                github,
                context.scope,
                context.source,
                context.expected_head,
                explicit=context.checkpoint,
                continuations_remaining=context.continuations_remaining,
                continuation_key=context.continuation_key,
            ).artifact
        ):
            raise ValueError("The source file does not contain the exact continuation")
        return validate_selection(
            github,
            context.scope,
            selection,
            read_json_file(checkpoint / "state.json"),
        )
    except (
        KeyError,
        TypeError,
        ValueError,
        OSError,
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
    ) as error:
        if context.continuation_key is not None:
            raise ValueError(
                "Cannot verify the automatic continuation checkpoint"
            ) from error
        logger.info(
            "Cannot reuse the feedback checkpoint; starting a bounded bootstrap: %s",
            error,
        )
        return None


def _publication(
    github: CommentGitHub, inputs: PublicationInputs
) -> FeedbackPublication:
    if inputs.context.continuation_key is not None:
        validate_continuation(
            github,
            inputs.context.scope,
            inputs.context.source,
            inputs.context.expected_head,
            explicit=inputs.context.checkpoint,
            continuations_remaining=inputs.context.continuations_remaining,
            continuation_key=inputs.context.continuation_key,
        )
    return prepare_publication(
        github,
        Git(inputs.repository).with_token(github.token_variable),
        inputs.context.scope,
        inputs.context.source,
        inputs.context.expected_head,
        inputs.prepared,
        inputs.result,
        inputs.session,
        trusted_root=inputs.trusted_root,
        preparation_artifact_id=inputs.preparation_artifact,
        result_artifact_id=inputs.result_artifact,
        session_artifact_id=inputs.session_artifact,
        continuations_remaining=inputs.context.continuations_remaining,
        continuation_key=inputs.context.continuation_key,
    )


def _load_prepared(path: Path, context: RunContext) -> PreparedFeedback:
    prepared = PreparedFeedback.from_json(read_json_file(path / "prepared.json"))
    if (
        prepared.scope != context.scope
        or prepared.dispatch_head != context.expected_head
        or not context.source.same_run(prepared.source)
        or context.source.attempt < prepared.source.attempt
        or prepared.continuations_remaining != context.continuations_remaining
        or prepared.continuation_key != context.continuation_key
    ):
        raise ValueError("The preparation does not match the trusted dispatch")
    return prepared


def _load_checkpoint(path: Path, context: RunContext) -> FeedbackCheckpoint:
    state = FeedbackCheckpoint.from_json(read_json_file(path / "state.json"))
    if (
        state.scope != context.scope
        or state.source != context.source
        or state.continuations_remaining != context.continuations_remaining
        or state.continuation_key != context.continuation_key
    ):
        raise ValueError("The checkpoint does not match the current publisher")
    return state


def run(command: CommentsCommand) -> None:
    github = CommentGitHub()
    match command:
        case FindSource():
            try:
                discovery = discover_checkpoint(
                    github,
                    command.context.scope,
                    command.context.source,
                    command.context.expected_head,
                    explicit=command.context.checkpoint,
                    continuations_remaining=command.context.continuations_remaining,
                    continuation_key=command.context.continuation_key,
                )
            except subprocess.CalledProcessError:
                if (
                    command.context.checkpoint is not None
                    or command.context.continuation_key is not None
                ):
                    raise
                logger.info(
                    "No usable workflow history is available; starting a bounded bootstrap"
                )
                discovery = BootstrapFeedback()
            source: CheckpointSelection | None
            proceed = True
            match discovery:
                case CheckpointSelection():
                    source = discovery
                case BootstrapFeedback():
                    source = None
                case SupersededFeedback(checkpoint=checkpoint):
                    source = None
                    proceed = False
                    logger.info(
                        "This continuation was superseded by checkpoint artifact %s",
                        checkpoint.identifier,
                    )
                case _:
                    assert_never(discovery)
            write_json_file(
                command.destination, source.to_json() if source is not None else None
            )
            write_json_output(command.github_output, "proceed", proceed)
            write_json_output(command.github_output, "found", source is not None)
            if source is not None:
                write_output(
                    command.github_output, "run-id", str(source.state.source.identifier)
                )
                write_output(
                    command.github_output, "artifact-id", str(source.state.identifier)
                )
            return
        case InspectSource():
            previous = _checkpoint(
                github, command.context, command.source_file, command.checkpoint
            )
            write_json_output(command.github_output, "usable", previous is not None)
            if previous is not None:
                write_output(
                    command.github_output,
                    "run-id",
                    str(previous.state.session.source.identifier),
                )
                write_output(
                    command.github_output,
                    "artifact-id",
                    str(previous.state.session.identifier),
                )
            return
        case PrepareComments():
            previous = _checkpoint(
                github, command.context, command.source_file, command.checkpoint
            )
            sessions: CodexSessionSnapshot | None = None
            if previous is not None:
                try:
                    sessions = snapshot_sessions(
                        command.sessions,
                        command.repository,
                        trusted_root=command.trusted_root,
                    )
                except (KeyError, TypeError, ValueError, OSError) as error:
                    if command.context.continuation_key is not None:
                        raise ValueError(
                            "Cannot restore the automatic continuation session"
                        ) from error
                    logger.info(
                        "Cannot reuse the Codex session; collecting a fresh history: %s",
                        error,
                    )
            try:
                prepared = prepare_feedback(
                    github,
                    Git(command.repository),
                    command.context.scope,
                    command.context.source,
                    command.context.expected_head,
                    command.destination,
                    trusted_files=command.trusted_files,
                    previous=previous,
                    sessions=sessions,
                    continuations_remaining=command.context.continuations_remaining,
                    continuation_key=command.context.continuation_key,
                )
            except IneligiblePullRequest as error:
                logger.info("%s", error)
                write_json_output(command.github_output, "eligible", False)
                return
            write_json_output(command.github_output, "eligible", True)
            write_output(command.github_output, "head-sha", str(prepared.head))
            write_output(
                command.github_output,
                "artifact-name",
                artifact_name(
                    FeedbackArtifactKind.CONTEXT, prepared.scope, prepared.source
                ),
            )
            append_summary(
                command.summary,
                f"Collected {len(prepared.targets)} actionable feedback targets for "
                f"pull request #{prepared.scope.number} at `{prepared.head}`.\n",
            )
            return
        case PrepareAgent():
            prepared = _load_prepared(command.context, command.run_context)
            arguments = prepare_agent(
                Git(command.repository),
                prepared,
                command.context,
                command.codex_home,
                source=command.run_context.source,
                trusted_root=command.trusted_root,
            )
            write_json_output(command.github_output, "codex-args", arguments)
            return
        case VerifyComments():
            prepared = _load_prepared(command.context, command.run_context)
            result = persist_feedback_result(
                Git(command.repository),
                prepared,
                CommentRecommendation.from_json(loads(sys.stdin.read())),
                command.destination,
                source=command.run_context.source,
                scratch=command.trusted_root,
            )
            sessions = snapshot_sessions(
                command.codex_home / "sessions",
                command.repository,
                trusted_root=command.trusted_root,
            )
            if (
                prepared.session_id is not None
                and sessions.identifier != prepared.session_id
            ):
                raise ValueError("Codex did not continue the verified feedback session")
            write_sessions(sessions, command.session_destination)
            write_json_output(
                command.github_output, "has-changes", result.commits is not None
            )
            write_output(
                command.github_output,
                "result-name",
                artifact_name(FeedbackArtifactKind.RESULT, result.scope, result.source),
            )
            write_output(
                command.github_output,
                "session-name",
                artifact_name(
                    FeedbackArtifactKind.SESSION, result.scope, result.source
                ),
            )
            append_summary(command.summary, result.recommendation.summary)
            return
        case ValidateComments():
            publication = _publication(github, command.inputs)
            write_json_output(
                command.github_output, "needs-writes", publication.needs_writes
            )
            write_json_output(
                command.github_output,
                "needs-continuation",
                publication.needs_continuation,
            )
            return
        case ApplyComments():
            reader = CommentGitHub(token_variable="GH_READ_TOKEN")
            publication = _publication(reader, command.inputs)
            checkpoint = apply_publication(
                reader, github, Git(command.inputs.repository), publication
            )
            command.destination.mkdir(parents=True, exist_ok=False)
            write_json_file(command.destination / "state.json", checkpoint.to_json())
            write_json_output(
                command.github_output,
                "has-pending",
                bool(checkpoint.collection.pending),
            )
            write_output(
                command.github_output,
                "artifact-name",
                artifact_name(
                    FeedbackArtifactKind.STATE, checkpoint.scope, checkpoint.source
                ),
            )
            append_summary(
                command.summary,
                f"Published feedback for pull request #{checkpoint.scope.number} "
                f"at `{checkpoint.head}` and retained its complete checkpoint.\n",
            )
            return
        case WriteIndex():
            state = _load_checkpoint(command.state, command.context)
            index = prepare_index(
                github, command.context.source, state, command.state_artifact
            )
            command.destination.mkdir(parents=True, exist_ok=False)
            write_json_file(command.destination / MANIFEST_FILENAME, index.to_json())
            write_output(
                command.github_output,
                "artifact-name",
                index_name(state.scope, state.source),
            )
            return
        case WriteIndexAliases():
            state = _load_checkpoint(command.state, command.context)
            aliases = prepare_index_aliases(
                github,
                command.context.source,
                state,
                command.state_artifact,
                command.index_artifact,
            )
            command.destination.mkdir(parents=True, exist_ok=False)
            discovery = command.destination / "discovery"
            discovery.mkdir()
            write_json_file(discovery / MANIFEST_FILENAME, aliases.discovery.to_json())
            write_output(
                command.github_output, "discovery-name", discovery_name(state.scope)
            )
            if aliases.consumption is not None:
                if state.continuation_key is None:
                    raise ValueError("A consumption alias is missing its key")
                marker = command.destination / "consumption"
                marker.mkdir()
                write_json_file(
                    marker / MANIFEST_FILENAME, aliases.consumption.to_json()
                )
                write_output(
                    command.github_output,
                    "marker-name",
                    marker_name(state.scope, state.continuation_key),
                )
            return
        case ContinueComments():
            reader = CommentGitHub(token_variable="GH_READ_TOKEN")
            continuation = continue_feedback(
                reader,
                github,
                command.context.source,
                _load_checkpoint(command.state, command.context),
                command.state_artifact,
                command.index_artifact,
                expected_head=command.context.expected_head,
                checkpoint=command.context.checkpoint,
            )
            write_json_output(
                command.github_output, "dispatched", continuation is not None
            )
            if continuation is not None:
                append_summary(
                    command.summary,
                    f"Queued the next bounded feedback batch for pull request "
                    f"#{continuation.scope.number} at `{continuation.head}`.\n",
                )
            return
        case WriteSchema():
            if command.destination is None:
                print(json.dumps(CommentRecommendation.schema(), indent=2))
            else:
                write_json_file(command.destination, CommentRecommendation.schema())
            return
    assert_never(command)


def main(arguments: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="uv-automations comments", description=__doc__
    )
    add_commands(parser)
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    try:
        run(parse_command(parser.parse_args(arguments)))
    except (KeyError, TypeError, ValueError) as error:
        parser.exit(2, f"{parser.prog}: {error}\n")
    except OSError as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    except subprocess.CalledProcessError as error:
        parser.exit(
            1, f"{parser.prog}: command failed with status {error.returncode}\n"
        )
    except subprocess.TimeoutExpired as error:
        parser.exit(
            1, f"{parser.prog}: command timed out after {error.timeout} seconds\n"
        )


if __name__ == "__main__":
    main()
