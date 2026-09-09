"""Command-line adapters for GitHub Actions."""

import argparse
import json
import logging
import sys
from enum import StrEnum
from pathlib import Path
from typing import assert_never

from uv_automations.actions import write_json_output
from uv_automations.github import GitHub
from uv_automations.json import as_array, as_string, loads
from uv_automations.models import RepositoryName
from uv_automations.workflows.conflicts import (
    conflict_payload,
    find_conflicted_pull_requests,
)
from uv_automations.workflows.labels import LabelRecommendation, plan_labels


class Command(StrEnum):
    LABELS = "labels"
    PULL_REQUESTS = "pull-requests"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    labels = commands.add_parser(Command.LABELS.value)
    label_commands = labels.add_subparsers(required=True)
    validate = label_commands.add_parser("validate")
    validate.add_argument("--allowed", type=Path, required=True)
    validate.add_argument("--github-output", type=Path)

    pull_requests = commands.add_parser(Command.PULL_REQUESTS.value)
    pull_request_commands = pull_requests.add_subparsers(required=True)
    conflicts = pull_request_commands.add_parser("conflicts")
    conflicts.add_argument("--repo", required=True)
    conflicts.add_argument("--author")

    arguments = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    command = Command(arguments.command)
    try:
        match command:
            case Command.LABELS:
                allowed = tuple(
                    as_string(label)
                    for label in as_array(
                        loads(arguments.allowed.read_text(encoding="utf-8"))
                    )
                )
                recommendation = LabelRecommendation.from_json(loads(sys.stdin.read()))
                plan = plan_labels(recommendation, allowed)
                if arguments.github_output is not None:
                    write_json_output(arguments.github_output, "labels", plan.labels)
                else:
                    print(json.dumps(plan.labels, separators=(",", ":")))
                return
            case Command.PULL_REQUESTS:
                found = find_conflicted_pull_requests(
                    GitHub(),
                    RepositoryName(arguments.repo),
                    author=arguments.author,
                )
                print(
                    json.dumps(
                        [conflict_payload(pull_request) for pull_request in found],
                        separators=(",", ":"),
                    )
                )
                return
        assert_never(command)
    except (KeyError, TypeError, ValueError) as error:
        parser.exit(2, f"{parser.prog}: {error}\n")
