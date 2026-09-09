"""Command-line stages for pull-request rebasing."""

import argparse
import logging
import re
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import append_summary, write_json_output, write_output
from uv_automations.artifacts import CommitRange
from uv_automations.git import Git
from uv_automations.github import GitHub
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.workflows.rebase import (
    CloseOutcome,
    EmptyRebase,
    LoadedRebase,
    PersistedRebase,
    PreparedRebase,
    PushOutcome,
    RebaseSource,
    SkippedRebase,
    VerifiedEmptyRebase,
    VerifiedRebaseSource,
    close_empty_rebase,
    load_rebase,
    persist_rebase,
    prepare_rebase,
    push_rebase,
    verify_empty_rebase,
    verify_rebase_source,
)


class CommandKind(StrEnum):
    PREPARE = "prepare"
    PERSIST = "persist"
    LOAD = "load"
    VERIFY_SOURCE = "verify-source"
    PUSH = "push"
    VERIFY_EMPTY = "verify-empty"
    CLOSE_EMPTY = "close-empty"


@dataclass(frozen=True, slots=True, kw_only=True)
class Prepare:
    checkout: Path
    source: RebaseSource
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class Persist:
    checkout: Path
    base_sha: CommitSha
    bundle: Path
    github_output: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class Load:
    checkout: Path
    base_ref: str
    commits: CommitRange
    bundle: Path
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class Push:
    checkout: Path
    verified: VerifiedRebaseSource
    rebased_head: CommitSha
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class VerifySource:
    rebase: PreparedRebase
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class VerifyEmpty:
    checkout: Path
    rebase: PreparedRebase
    github_output: Path
    summary: Path


@dataclass(frozen=True, slots=True, kw_only=True)
class CloseEmpty:
    verified: VerifiedEmptyRebase
    run_id: int
    summary: Path


type Command = Prepare | Persist | Load | VerifySource | Push | VerifyEmpty | CloseEmpty


def _positive_integer(value: str) -> int:
    if re.fullmatch(r"[1-9][0-9]*", value) is None:
        raise ValueError("Expected a positive integer")
    return int(value)


def _optional_commit(value: str) -> CommitSha | None:
    return CommitSha(value) if value else None


def _add_source(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", type=RepositoryName, required=True)
    parser.add_argument("--repository-id", type=_positive_integer, required=True)
    parser.add_argument("--pull-request", type=_positive_integer, required=True)
    parser.add_argument("--base-ref", required=True)
    parser.add_argument("--head-repository", type=RepositoryName, required=True)
    parser.add_argument("--head-ref", required=True)
    parser.add_argument("--head-sha", type=CommitSha, required=True)
    parser.add_argument("--previous-base-sha", type=_optional_commit)


def _source(parsed: argparse.Namespace) -> RebaseSource:
    return RebaseSource(
        repository=RepositoryIdentity(parsed.repo, parsed.repository_id),
        number=parsed.pull_request,
        base_ref=parsed.base_ref,
        head_repository=parsed.head_repository,
        head_ref=parsed.head_ref,
        head_sha=parsed.head_sha,
        previous_base=parsed.previous_base_sha,
    )


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)

    prepare = commands.add_parser(CommandKind.PREPARE)
    prepare.set_defaults(command=CommandKind.PREPARE)
    _add_source(prepare)
    prepare.add_argument("--checkout", type=Path, required=True)
    prepare.add_argument("--github-output", type=Path, required=True)

    persist = commands.add_parser(CommandKind.PERSIST)
    persist.set_defaults(command=CommandKind.PERSIST)
    persist.add_argument("--checkout", type=Path, required=True)
    persist.add_argument("--base-sha", type=CommitSha, required=True)
    persist.add_argument("--bundle", type=Path, required=True)
    persist.add_argument("--github-output", type=Path, required=True)

    load = commands.add_parser(CommandKind.LOAD)
    load.set_defaults(command=CommandKind.LOAD)
    load.add_argument("--checkout", type=Path, required=True)
    load.add_argument("--base-ref", required=True)
    load.add_argument("--base-sha", type=CommitSha, required=True)
    load.add_argument("--head-sha", type=CommitSha, required=True)
    load.add_argument("--bundle", type=Path, required=True)
    load.add_argument("--github-output", type=Path, required=True)
    load.add_argument("--summary", type=Path, required=True)

    verify_source = commands.add_parser(CommandKind.VERIFY_SOURCE)
    verify_source.set_defaults(command=CommandKind.VERIFY_SOURCE)
    _add_source(verify_source)
    verify_source.add_argument("--base-sha", type=CommitSha, required=True)
    verify_source.add_argument("--github-output", type=Path, required=True)
    verify_source.add_argument("--summary", type=Path, required=True)

    push = commands.add_parser(CommandKind.PUSH)
    push.set_defaults(command=CommandKind.PUSH)
    _add_source(push)
    push.add_argument("--checkout", type=Path, required=True)
    push.add_argument("--base-sha", type=CommitSha, required=True)
    push.add_argument("--head-repository-id", type=_positive_integer, required=True)
    push.add_argument("--rebased-head", type=CommitSha, required=True)
    push.add_argument("--summary", type=Path, required=True)

    verify = commands.add_parser(CommandKind.VERIFY_EMPTY)
    verify.set_defaults(command=CommandKind.VERIFY_EMPTY)
    _add_source(verify)
    verify.add_argument("--checkout", type=Path, required=True)
    verify.add_argument("--base-sha", type=CommitSha, required=True)
    verify.add_argument("--github-output", type=Path, required=True)
    verify.add_argument("--summary", type=Path, required=True)

    close = commands.add_parser(CommandKind.CLOSE_EMPTY)
    close.set_defaults(command=CommandKind.CLOSE_EMPTY)
    _add_source(close)
    close.add_argument("--base-sha", type=CommitSha, required=True)
    close.add_argument("--head-repository-id", type=_positive_integer, required=True)
    close.add_argument("--run-id", type=_positive_integer, required=True)
    close.add_argument("--summary", type=Path, required=True)


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="uv-automations rebase", description=__doc__)
    add_commands(parser)
    return parser


def parse_command(parsed: argparse.Namespace) -> Command:
    kind = CommandKind(parsed.command)
    match kind:
        case CommandKind.PREPARE:
            return Prepare(
                checkout=parsed.checkout,
                source=_source(parsed),
                github_output=parsed.github_output,
            )
        case CommandKind.PERSIST:
            return Persist(
                checkout=parsed.checkout,
                base_sha=parsed.base_sha,
                bundle=parsed.bundle,
                github_output=parsed.github_output,
            )
        case CommandKind.LOAD:
            return Load(
                checkout=parsed.checkout,
                base_ref=parsed.base_ref,
                commits=CommitRange(parsed.base_sha, parsed.head_sha),
                bundle=parsed.bundle,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommandKind.VERIFY_SOURCE:
            return VerifySource(
                rebase=PreparedRebase(_source(parsed), parsed.base_sha),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommandKind.PUSH:
            return Push(
                checkout=parsed.checkout,
                verified=VerifiedRebaseSource(
                    PreparedRebase(_source(parsed), parsed.base_sha),
                    RepositoryIdentity(
                        parsed.head_repository, parsed.head_repository_id
                    ),
                ),
                rebased_head=parsed.rebased_head,
                summary=parsed.summary,
            )
        case CommandKind.VERIFY_EMPTY:
            return VerifyEmpty(
                checkout=parsed.checkout,
                rebase=PreparedRebase(_source(parsed), parsed.base_sha),
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case CommandKind.CLOSE_EMPTY:
            return CloseEmpty(
                verified=VerifiedEmptyRebase(
                    PreparedRebase(_source(parsed), parsed.base_sha),
                    RepositoryIdentity(
                        parsed.head_repository, parsed.head_repository_id
                    ),
                ),
                run_id=parsed.run_id,
                summary=parsed.summary,
            )
    assert_never(kind)


def _push_message(outcome: PushOutcome) -> str:
    match outcome:
        case PushOutcome.PUSHED:
            return "Pushed the rebased pull request."
        case PushOutcome.STALE:
            return "The pull request changed; skipped the stale rebase."
    assert_never(outcome)


def _close_message(outcome: CloseOutcome) -> str:
    match outcome:
        case CloseOutcome.CLOSED:
            return "Closed the empty pull request."
        case CloseOutcome.STALE:
            return "The pull request changed; leaving it open."
    assert_never(outcome)


def run(command: Command) -> None:
    github = GitHub()
    match command:
        case Prepare():
            prepared = prepare_rebase(github, Git(command.checkout), command.source)
            write_output(command.github_output, "base_sha", str(prepared.base_sha))
            return
        case Persist():
            result = persist_rebase(
                Git(command.checkout), command.base_sha, command.bundle
            )
            match result:
                case EmptyRebase():
                    write_json_output(command.github_output, "empty", True)
                    write_output(
                        command.github_output, "head_sha", str(result.base_sha)
                    )
                    return
                case PersistedRebase():
                    write_json_output(command.github_output, "empty", False)
                    write_output(
                        command.github_output,
                        "head_sha",
                        str(result.bundle.commits.head),
                    )
                    return
            assert_never(result)
        case Load():
            result = load_rebase(
                Git(command.checkout), command.base_ref, command.commits, command.bundle
            )
            match result:
                case LoadedRebase():
                    write_output(
                        command.github_output, "head_sha", str(result.head_sha)
                    )
                    return
                case SkippedRebase():
                    append_summary(command.summary, result.reason)
                    return
            assert_never(result)
        case Push():
            outcome = push_rebase(
                GitHub(token_variable="GH_READ_TOKEN"),
                Git(command.checkout),
                command.verified,
                command.rebased_head,
            )
            append_summary(command.summary, _push_message(outcome))
            return
        case VerifySource():
            result = verify_rebase_source(github, command.rebase)
            match result:
                case VerifiedRebaseSource():
                    write_json_output(command.github_output, "ready", True)
                    write_json_output(
                        command.github_output,
                        "head_repository_id",
                        result.head_repository.database_id,
                    )
                    return
                case SkippedRebase():
                    write_json_output(command.github_output, "ready", False)
                    append_summary(command.summary, result.reason)
                    return
            assert_never(result)
        case VerifyEmpty():
            result = verify_empty_rebase(github, Git(command.checkout), command.rebase)
            match result:
                case VerifiedEmptyRebase():
                    write_json_output(command.github_output, "close", True)
                    write_json_output(
                        command.github_output,
                        "head_repository_id",
                        result.head_repository.database_id,
                    )
                    return
                case SkippedRebase():
                    write_json_output(command.github_output, "close", False)
                    append_summary(command.summary, result.reason)
                    return
            assert_never(result)
        case CloseEmpty():
            outcome = close_empty_rebase(
                github, command.verified, run_id=command.run_id
            )
            append_summary(command.summary, _close_message(outcome))
            return
    assert_never(command)


def main(arguments: Sequence[str] | None = None) -> None:
    parser = create_parser()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    try:
        run(parse_command(parser.parse_args(arguments)))
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(2, f"{parser.prog}: {error}\n")
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
