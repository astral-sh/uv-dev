"""Count-only command-line stages for post-sync promotion retargeting."""

import argparse
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import append_summary, write_output
from uv_automations.github_promotion import PromotionGitHub, PromotionReadError
from uv_automations.github_promotion_retarget import PromotionRetargetGitHub
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.promotion_models import (
    UV_SECURITY_REPOSITORY,
    require_promotion_source,
)
from uv_automations.workflows.promotion_retarget import (
    RetargetBatch,
    Retargeted,
    RetargetOutcome,
    RetargetPreparation,
    RetargetSkipReason,
    SkippedRetarget,
    StaleRetargetSync,
    apply_retargets,
    plan_retargets,
)


class PromotionRetargetCommandKind(StrEnum):
    IDENTIFY = "promotions.retarget.identify"
    APPLY = "promotions.retarget.apply"


@dataclass(frozen=True, slots=True, kw_only=True)
class IdentifyRetargets:
    repository: RepositoryIdentity
    main_sha: CommitSha
    github_output: Path | None
    summary: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class ApplyRetargets:
    repository: RepositoryIdentity
    main_sha: CommitSha
    summary: Path | None


type PromotionRetargetCommand = IdentifyRetargets | ApplyRetargets


def _public_commit(value: str) -> CommitSha:
    try:
        return CommitSha(value)
    except ValueError:
        raise argparse.ArgumentTypeError("Expected a full public commit SHA") from None


def _add_source(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", type=RepositoryName, required=True)
    parser.add_argument("--repository-id", type=int, required=True)
    parser.add_argument("--main-sha", type=_public_commit, required=True)
    parser.add_argument("--summary", type=Path)


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    identify = commands.add_parser("identify")
    identify.set_defaults(command=PromotionRetargetCommandKind.IDENTIFY)
    _add_source(identify)
    identify.add_argument("--github-output", type=Path)

    apply = commands.add_parser("apply")
    apply.set_defaults(command=PromotionRetargetCommandKind.APPLY)
    _add_source(apply)


def parse_command(parsed: argparse.Namespace) -> PromotionRetargetCommand:
    repository = RepositoryIdentity(parsed.repo, parsed.repository_id)
    require_promotion_source(repository)
    kind = PromotionRetargetCommandKind(parsed.command)
    match kind:
        case PromotionRetargetCommandKind.IDENTIFY:
            return IdentifyRetargets(
                repository=repository,
                main_sha=parsed.main_sha,
                github_output=parsed.github_output,
                summary=parsed.summary,
            )
        case PromotionRetargetCommandKind.APPLY:
            return ApplyRetargets(
                repository=repository,
                main_sha=parsed.main_sha,
                summary=parsed.summary,
            )
    assert_never(kind)


def _identify_summary(repository: RepositoryIdentity, batch: RetargetBatch) -> str:
    return (
        f"Found {len(batch.plans)} draft pull requests to retarget in "
        f"`{repository.name}`. Left {batch.ready_children} ready children with "
        f"promotion and skipped {batch.unverified_children} unverifiable children."
    )


def _apply_summary(
    repository: RepositoryIdentity,
    batch: RetargetBatch,
    outcomes: tuple[RetargetOutcome, ...],
) -> str:
    retargeted = 0
    already_targeted = 0
    ready = batch.ready_children
    changed = 0
    for outcome in outcomes:
        match outcome:
            case Retargeted():
                retargeted += 1
                continue
            case SkippedRetarget(reason=reason):
                match reason:
                    case RetargetSkipReason.ALREADY_TARGETED:
                        already_targeted += 1
                        continue
                    case RetargetSkipReason.READY_FOR_PROMOTION:
                        ready += 1
                        continue
                    case (
                        RetargetSkipReason.MAIN_CHANGED
                        | RetargetSkipReason.PARENT_CHANGED
                        | RetargetSkipReason.SOURCE_CHANGED
                    ):
                        changed += 1
                        continue
                assert_never(reason)
        assert_never(outcome)
    return (
        f"Retargeted {retargeted} draft pull requests in `{repository.name}`. "
        f"{already_targeted} were already on main. Left {ready} ready children "
        f"with promotion and skipped {batch.unverified_children + changed} "
        "unverifiable or changed children."
    )


def _report(summary: Path | None, message: str) -> None:
    if summary is None:
        print(message)
    else:
        append_summary(summary, message)


def _stale_summary(repository: RepositoryIdentity) -> str:
    return (
        f"The synchronized main of `{repository.name}` changed; "
        "leaving its stacks unchanged."
    )


def _identification(
    repository: RepositoryIdentity, preparation: RetargetPreparation
) -> tuple[int, str]:
    match preparation:
        case RetargetBatch():
            return len(preparation.plans), _identify_summary(repository, preparation)
        case StaleRetargetSync():
            return 0, _stale_summary(repository)
    assert_never(preparation)


def _run(command: PromotionRetargetCommand) -> None:
    require_promotion_source(command.repository)
    reader = PromotionGitHub(token_variable="GH_READ_TOKEN")
    upstream_reader = PromotionGitHub(token_variable="GH_UPSTREAM_TOKEN")
    preparation = plan_retargets(
        reader, upstream_reader, command.repository, command.main_sha
    )
    match command:
        case IdentifyRetargets():
            count, message = _identification(command.repository, preparation)
            if command.github_output is not None:
                write_output(command.github_output, "count", str(count))
            _report(command.summary, message)
            return
        case ApplyRetargets():
            match preparation:
                case StaleRetargetSync():
                    _report(command.summary, _stale_summary(command.repository))
                    return
                case RetargetBatch():
                    outcomes = apply_retargets(
                        reader,
                        upstream_reader,
                        PromotionRetargetGitHub(token_variable="GH_TOKEN"),
                        preparation,
                    )
                    _report(
                        command.summary,
                        _apply_summary(command.repository, preparation, outcomes),
                    )
                    return
            assert_never(preparation)
    assert_never(command)


def run(command: PromotionRetargetCommand) -> None:
    try:
        _run(command)
    except PromotionReadError as error:
        if command.repository == UV_SECURITY_REPOSITORY:
            raise ValueError("Private promotion retargeting failed") from None
        raise ValueError(str(error)) from None
    except Exception:
        if command.repository == UV_SECURITY_REPOSITORY:
            # These stages run under a public uv workflow. A programming or
            # decoder error must not print a private PR snapshot or ref.
            raise ValueError("Private promotion retargeting failed") from None
        raise
