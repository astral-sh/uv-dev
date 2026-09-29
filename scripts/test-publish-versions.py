"""Run the versions-publishing shell step against an offline GitHub fixture."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/publish-versions.yml"
HEAD = "a" * 40
VERSION = "0.12.20"
BRANCH = "update-versions-0.12.20-200"
BOT = "app/astral-releases-bot"
REPOSITORY = "astral-sh/versions"

# This is an executable-level GitHub/Git fixture. It implements only the
# commands issued by the publishing step and rejects every other command.
COMMAND_FIXTURE = r"""
import json
import os
import pathlib
import socket
import subprocess
import sys

def deny_network(event, arguments):
    if event in {"socket.connect", "socket.getaddrinfo"}:
        raise RuntimeError("Network disabled in the publishing fixture")

sys.addaudithook(deny_network)

path = pathlib.Path(os.environ["PUBLISH_TEST_STATE"])
state = json.loads(path.read_text())
name, arguments = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
state["commands"].append([name, *arguments])

def save():
    path.write_text(json.dumps(state))

def fail(message):
    save()
    print(message, file=sys.stderr)
    raise SystemExit(1)

def option(name, default=None):
    if name not in arguments:
        return default
    return arguments[arguments.index(name) + 1]

def emit(value):
    fields = option("--json")
    if fields is not None:
        names = fields.split(",")
        if isinstance(value, list):
            value = [{key: row[key] for key in names if key in row} for row in value]
        else:
            value = {key: value[key] for key in names if key in value}
    query = option("--jq")
    if query is None:
        print(json.dumps(value))
    else:
        result = subprocess.run(
            [os.environ["PUBLISH_TEST_JQ"], "-c", query],
            input=json.dumps(value), text=True, capture_output=True, check=True,
        )
        for line in result.stdout.splitlines():
            item = json.loads(line)
            if item is not None:
                print(item if isinstance(item, str) else json.dumps(item))
    save()

if name == "git":
    if arguments[:1] == ["rev-parse"]:
        print(state["head"])
        save()
    elif arguments[:2] == ["push", "origin"]:
        if state.get("fail") == "push":
            fail("injected push failure")
        state["pushed"] = True
        save()
    else:
        fail("unexpected Git command")
elif name == "gh":
    if arguments == ["auth", "setup-git"]:
        save()
    elif arguments[:2] == ["pr", "create"]:
        if not state.get("pushed"):
            fail("create before push")
        if state.get("fail") == "create":
            fail("injected create failure")
        replacement = dict(state["replacement"])
        replacement["title"] = option("--title")
        replacement["headRefName"] = option("--head")
        replacement["baseRefName"] = option("--base")
        state["pulls"].append(replacement)
        save()
        print("https://github.com/astral-sh/versions/pull/" + str(replacement["number"]))
    elif arguments[:2] == ["pr", "view"]:
        target = arguments[2]
        matches = [
            row for row in state["pulls"]
            if str(row["number"]) == target or row["headRefName"] == target
        ]
        if len(matches) != 1 or state.get("fail") == "view":
            fail("injected view failure")
        emit(matches[0])
    elif arguments[:2] == ["pr", "list"]:
        if state.get("fail") == "list":
            fail("injected list failure")
        state["pulls"].extend(state.pop("concurrent_pulls", []))
        rows = state["pulls"]
        if option("--state"):
            rows = [row for row in rows if row["state"].lower() == option("--state")]
        if option("--base"):
            rows = [row for row in rows if row["baseRefName"] == option("--base")]
        if option("--label"):
            rows = [row for row in rows if option("--label") in row["labels"]]
        emit(rows)
    elif arguments[:2] == ["pr", "close"]:
        candidates = [
            item for index, item in enumerate(arguments[2:], 2)
            if not item.startswith("--") and arguments[index - 1] != "--repo"
        ]
        if len(candidates) != 1 or not candidates[0].isdigit():
            fail("invalid pull request number")
        matches = [row for row in state["pulls"] if row["number"] == int(candidates[0])]
        if len(matches) != 1:
            fail("unknown pull request number")
        matches[0]["state"] = "CLOSED"
        save()
    else:
        fail("unexpected GitHub CLI command")
else:
    fail("unexpected executable")
"""


def publishing_step() -> str:
    lines = WORKFLOW.read_text().splitlines()
    start = lines.index('      - name: "Create Pull Request"')
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index].startswith("      - ")
        ),
        len(lines),
    )
    run = next(index for index in range(start, end) if lines[index] == "        run: |")
    return textwrap.dedent("\n".join(lines[run + 1 : end])) + "\n"


def pull(number: int, **changes: object) -> dict[str, object]:
    return {
        "number": number,
        "title": f"Add uv {VERSION}",
        "headRefName": f"update-versions-{VERSION}-{number}",
        "headRefOid": HEAD,
        "baseRefName": "main",
        "isCrossRepository": False,
        "author": {"login": BOT},
        "labels": ["automation"],
        "state": "OPEN",
    } | changes


class PublishVersions(unittest.TestCase):
    def run_publisher(
        self,
        pulls: list[dict[str, object]] | None = None,
        *,
        failure: str | None = None,
        replacement: dict[str, object] | None = None,
        concurrent_pulls: list[dict[str, object]] | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
        jq = shutil.which("jq")
        xargs = shutil.which("xargs")
        bash = shutil.which("bash")
        self.assertIsNotNone(jq)
        self.assertIsNotNone(xargs)
        self.assertIsNotNone(bash)
        with tempfile.TemporaryDirectory(prefix="publish-versions-") as temporary:
            directory = pathlib.Path(temporary)
            binaries = directory / "bin"
            binaries.mkdir()
            fixture = f"#!{sys.executable} -I\n" + COMMAND_FIXTURE
            for name in ("git", "gh"):
                executable = binaries / name
                executable.write_text(fixture)
                executable.chmod(0o700)
            (binaries / "jq").symlink_to(jq)
            (binaries / "xargs").symlink_to(xargs)
            state_file = directory / "state.json"
            state_file.write_text(
                json.dumps(
                    {
                        "head": HEAD,
                        "commands": [],
                        "pulls": [pull(11)] if pulls is None else pulls,
                        "replacement": pull(91) | (replacement or {}),
                        "concurrent_pulls": concurrent_pulls or [],
                        "fail": failure,
                    }
                )
            )
            environment = {
                "PATH": str(binaries),
                "HOME": str(directory),
                "TMPDIR": str(directory),
                "VERSION": VERSION,
                "BRANCH_NAME": BRANCH,
                "GH_TOKEN": "offline-fixture-token",
                "GH_REPO": REPOSITORY,
                "GH_NO_UPDATE_NOTIFIER": "1",
                "PUBLISH_TEST_STATE": str(state_file),
                "PUBLISH_TEST_JQ": str(jq),
            }
            result = subprocess.run(
                [
                    str(bash),
                    "--noprofile",
                    "--norc",
                    "-e",
                    "-o",
                    "pipefail",
                    "-c",
                    publishing_step(),
                ],
                cwd=directory,
                env=environment,
                text=True,
                capture_output=True,
                timeout=30,
                check=False,
            )
            return result, json.loads(state_file.read_text())

    def test_selects_predecessor_number(self) -> None:
        result, state = self.run_publisher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]],
            [(11, "CLOSED"), (91, "OPEN")],
        )

    def test_push_failure_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(failure="push")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]], [(11, "OPEN")]
        )
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_create_failure_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(failure="create")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]], [(11, "OPEN")]
        )
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_proves_replacement_before_closing(self) -> None:
        result, state = self.run_publisher()
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = state["commands"]
        push = next(
            index
            for index, command in enumerate(commands)
            if command[:2] == ["git", "push"]
        )
        create = next(
            index
            for index, command in enumerate(commands)
            if command[1:3] == ["pr", "create"]
        )
        view = next(
            index
            for index, command in enumerate(commands)
            if command[1:3] == ["pr", "view"]
        )
        close = next(
            index
            for index, command in enumerate(commands)
            if command[1:3] == ["pr", "close"]
        )
        self.assertLess(push, create)
        self.assertLess(create, view)
        self.assertLess(view, close)
        self.assertEqual(
            commands[push], ["git", "push", "origin", f"{HEAD}:refs/heads/{BRANCH}"]
        )

    def test_failed_replacement_readback_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(failure="view")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["pulls"][0]["state"], "OPEN")
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_wrong_replacement_head_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(replacement={"headRefOid": "b" * 40})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["pulls"][0]["state"], "OPEN")
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_cross_repository_replacement_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(replacement={"isCrossRepository": True})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["pulls"][0]["state"], "OPEN")
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_list_failure_keeps_predecessor(self) -> None:
        result, state = self.run_publisher(failure="list")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]],
            [(11, "OPEN"), (91, "OPEN")],
        )
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_only_matching_automation_predecessors_are_closed(self) -> None:
        original = [
            pull(11),
            pull(12, author={"login": "astral-versions-bot"}),
            pull(13, author={"login": "another-user"}),
            pull(14, title="Add uv 0.12.19"),
            pull(15, headRefName="unrelated-branch"),
            pull(16, baseRefName="other-base"),
            pull(17, isCrossRepository=True),
            pull(18, labels=[]),
            pull(19, state="CLOSED"),
        ]
        result, state = self.run_publisher(original)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            [row["number"] for row in state["pulls"] if row["state"] == "CLOSED"],
            [11, 12, 19],
        )
        self.assertEqual(state["pulls"][-1]["state"], "OPEN")

    def test_no_predecessor(self) -> None:
        result, state = self.run_publisher([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]], [(91, "OPEN")]
        )
        self.assertFalse(
            any(command[1:3] == ["pr", "close"] for command in state["commands"])
        )

    def test_concurrent_newer_replacement_stays_open(self) -> None:
        result, state = self.run_publisher(concurrent_pulls=[pull(92)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            [(row["number"], row["state"]) for row in state["pulls"]],
            [(11, "CLOSED"), (91, "OPEN"), (92, "OPEN")],
        )

    def test_replacement_number_must_be_positive_integer(self) -> None:
        for number in (None, "91", 0, -1, 91.5):
            with self.subTest(number=number):
                result, state = self.run_publisher(replacement={"number": number})
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(state["pulls"][0]["state"], "OPEN")
                self.assertFalse(
                    any(
                        command[1:3] == ["pr", "close"] for command in state["commands"]
                    )
                )


if __name__ == "__main__":
    unittest.main()
