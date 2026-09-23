"""Command-line adapters for immutable commit artifacts."""

import argparse
import json
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import write_output
from uv_automations.artifacts import CommitRange, load_commit, persist_commit
from uv_automations.git import Git
from uv_automations.models import CommitSha


class CommitCommandKind(StrEnum):
    PERSIST = "commits.persist"
    LOAD = "commits.load"


@dataclass(frozen=True, slots=True, kw_only=True)
class PersistCommit:
    repository: Path
    base: CommitSha
    head: CommitSha | None
    destination: Path
    github_output: Path | None


@dataclass(frozen=True, slots=True, kw_only=True)
class LoadCommit:
    repository: Path
    commits: CommitRange
    bundle: Path
    github_output: Path | None


type CommitCommand = PersistCommit | LoadCommit


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    persist = commands.add_parser("persist")
    persist.set_defaults(command=CommitCommandKind.PERSIST)
    persist.add_argument("--repository", type=Path, required=True)
    persist.add_argument("--base", type=CommitSha, required=True)
    persist.add_argument("--head", type=CommitSha)
    persist.add_argument("--destination", type=Path, required=True)
    persist.add_argument("--github-output", type=Path)

    load = commands.add_parser("load")
    load.set_defaults(command=CommitCommandKind.LOAD)
    load.add_argument("--repository", type=Path, required=True)
    load.add_argument("--base", type=CommitSha, required=True)
    load.add_argument("--head", type=CommitSha, required=True)
    load.add_argument("--bundle", type=Path, required=True)
    load.add_argument("--github-output", type=Path)


def parse_command(parsed: argparse.Namespace) -> CommitCommand:
    kind = CommitCommandKind(parsed.command)
    match kind:
        case CommitCommandKind.PERSIST:
            return PersistCommit(
                repository=parsed.repository,
                base=parsed.base,
                head=parsed.head,
                destination=parsed.destination,
                github_output=parsed.github_output,
            )
        case CommitCommandKind.LOAD:
            return LoadCommit(
                repository=parsed.repository,
                commits=CommitRange(parsed.base, parsed.head),
                bundle=parsed.bundle,
                github_output=parsed.github_output,
            )
    assert_never(kind)


def _execute(command: CommitCommand) -> dict[str, str]:
    repository = Git(command.repository)
    match command:
        case PersistCommit():
            bundle = persist_commit(
                repository,
                CommitRange(
                    command.base, command.head or repository.resolve_commit("HEAD")
                ),
                command.destination,
            )
            return {"head-sha": str(bundle.commits.head), "path": str(bundle.path)}
        case LoadCommit():
            head = load_commit(repository, command.bundle, command.commits)
            return {"head-sha": str(head)}
    assert_never(command)


def run(command: CommitCommand) -> None:
    outputs = _execute(command)
    if command.github_output is None:
        print(json.dumps(outputs, separators=(",", ":")))
    else:
        for name, value in outputs.items():
            write_output(command.github_output, name, value)
