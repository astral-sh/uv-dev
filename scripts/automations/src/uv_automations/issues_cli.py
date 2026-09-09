"""Command-line adapters for verified issue collection."""

import argparse
import json
import logging
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import write_json_output, write_output
from uv_automations.github import GitHub
from uv_automations.models import IssueRef, RepositoryName
from uv_automations.workflows.issues import prepare_issue


class IssueCommandKind(StrEnum):
    PREPARE = "issues.prepare"


@dataclass(frozen=True, slots=True, kw_only=True)
class PrepareIssue:
    reference: IssueRef
    path: Path
    workspace: Path
    runner_temp: Path
    github_output: Path | None


type IssueCommand = PrepareIssue


def add_commands(parser: argparse.ArgumentParser) -> None:
    commands = parser.add_subparsers(required=True)
    prepare = commands.add_parser("prepare")
    prepare.set_defaults(command=IssueCommandKind.PREPARE)
    prepare.add_argument(
        "--repo", dest="repository", type=RepositoryName, required=True
    )
    prepare.add_argument("--issue", required=True)
    prepare.add_argument("--path", type=Path, required=True)
    prepare.add_argument("--workspace", type=Path, required=True)
    prepare.add_argument("--runner-temp", type=Path, required=True)
    prepare.add_argument("--github-output", type=Path)


def parse_command(parsed: argparse.Namespace) -> IssueCommand:
    kind = IssueCommandKind(parsed.command)
    match kind:
        case IssueCommandKind.PREPARE:
            return PrepareIssue(
                reference=IssueRef.from_input(parsed.repository, parsed.issue),
                path=parsed.path,
                workspace=parsed.workspace,
                runner_temp=parsed.runner_temp,
                github_output=parsed.github_output,
            )
    assert_never(kind)


def run(command: IssueCommand) -> None:
    prepared = prepare_issue(
        GitHub(),
        command.reference,
        command.path,
        workspace=command.workspace,
        runner_temp=command.runner_temp,
    )
    if command.github_output is None:
        print(json.dumps(prepared.issue.to_payload(), separators=(",", ":")))
        return
    write_json_output(
        command.github_output, "issue-number", prepared.issue.reference.number
    )
    write_json_output(command.github_output, "issue-json", prepared.issue.to_payload())
    write_output(command.github_output, "path", str(prepared.path))


def main(arguments: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(prog="uv-automations issues", description=__doc__)
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
