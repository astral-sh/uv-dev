"""Load and publish the versioned triage report attached to an upstream issue."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ARTIFACT_NAME = "Triage results"
ARTIFACT_CREATOR = "github-actions[bot]"
DATA_HEADER = (
    "\n\n<!-- uv-issue-context-v1 -->\n"
    "<details>\n<summary>Structured results</summary>\n\n```json\n"
)
DATA_FOOTER = "\n```\n</details>\n"
RESULT_NAMES = ("triage", "reproduction", "fix")
LEGACY_REPOSITORY = "https://github.com/astral-sh/uv-dev.git"


def api(endpoint, *, method="GET", payload=None):
    command = [
        "gh",
        "api",
        "--method",
        method,
        "--header",
        "Accept: application/vnd.github+json",
        "--header",
        "X-GitHub-Api-Version: 2026-03-10",
        endpoint,
    ]
    if payload is not None:
        command.extend(["--input", "-"])
    result = subprocess.run(
        command,
        input=json.dumps(payload) if payload is not None else None,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"GitHub API request failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def git(*arguments, directory=None, check=True):
    return subprocess.run(
        [
            "git",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "credential.helper=",
            *arguments,
        ],
        cwd=directory,
        env=os.environ | {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"},
        text=True,
        stdout=subprocess.PIPE,
        check=check,
    )


@dataclass
class Context:
    readme: str | None
    data: dict
    revision: str
    artifact_number: int | None = None


def validate_data(data, repository, issue_number):
    if not isinstance(data, dict) or data.get("repository") != repository:
        raise ValueError("The issue context belongs to a different repository.")
    issue = data.get("issue")
    if (
        not isinstance(issue, dict)
        or type(issue.get("number")) is not int
        or issue["number"] != issue_number
        or issue.get("url") != f"https://github.com/{repository}/issues/{issue_number}"
    ):
        raise ValueError("The issue context does not match the requested issue.")
    if data.keys() - {"repository", "issue", *RESULT_NAMES}:
        raise ValueError("The issue context contains unknown structured results.")
    for name in RESULT_NAMES:
        if name in data and not isinstance(data[name], dict):
            raise ValueError(f"The {name} result must be a JSON object.")


def encode(readme, data):
    return readme.rstrip() + DATA_HEADER + json.dumps(data, indent=2) + DATA_FOOTER


def decode(body, repository, issue_number):
    readme, separator, structured = body.rpartition(DATA_HEADER)
    if not separator or not structured.endswith(DATA_FOOTER):
        raise ValueError("The triage artifact has no supported structured results.")
    data = json.loads(structured.removesuffix(DATA_FOOTER))
    validate_data(data, repository, issue_number)
    return readme + "\n", data


def read_artifact(repository, issue_number):
    endpoint = f"repos/{repository}/issues/{issue_number}/artifacts"
    matches = []
    page = 1
    while True:
        artifacts = api(f"{endpoint}?per_page=100&page={page}")
        if not isinstance(artifacts, list):
            raise TypeError("GitHub returned an invalid issue artifact list.")
        matches.extend(
            artifact for artifact in artifacts if artifact.get("name") == ARTIFACT_NAME
        )
        # This endpoint does not return pagination links.
        if len(artifacts) < 100:
            break
        page += 1
    if not matches:
        return None
    if len(matches) != 1:
        raise ValueError("The issue has more than one triage artifact.")
    number = matches[0]["number"]
    if type(number) is not int or number < 1:
        raise ValueError("The triage artifact number is invalid.")
    response = api(f"{endpoint}/{number}")
    artifact = response["artifact"]
    if (
        artifact["number"] != number
        or artifact["name"] != ARTIFACT_NAME
        or artifact["type"] != "generic"
        or artifact.get("creator", {}).get("login") != ARTIFACT_CREATOR
    ):
        raise ValueError("The triage artifact is not owned by this automation.")
    versions = response["versions"]
    if (
        not versions
        or type(versions[0]["version"]) is not int
        or versions[0]["version"] < 1
    ):
        raise ValueError("The triage artifact has no valid version history.")
    readme, data = decode(artifact["body"], repository, issue_number)
    revision = digest(
        ["artifact", artifact["id"], number, versions[0]["version"], artifact["body"]]
    )
    return Context(readme, data, revision, number)


def read_legacy(repository, issue_number):
    # Reports written before issue artifacts remain available until their next update.
    # Public Git reads do not require a cross-repository contents token.
    reference = git(
        "ls-remote",
        "--exit-code",
        LEGACY_REPOSITORY,
        f"refs/heads/issue/{issue_number}",
        check=False,
    )
    if reference.returncode == 2:
        return Context(None, {}, digest(["missing"]))
    reference.check_returncode()
    revision = reference.stdout.split()[0]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("The legacy issue-context revision is invalid.")
    files = {}
    with tempfile.TemporaryDirectory(
        prefix="issue-context-", dir=os.environ["RUNNER_TEMP"]
    ) as directory:
        git("init", "--quiet", "--bare", directory)
        git(
            "fetch",
            "--quiet",
            "--no-tags",
            "--depth=1",
            LEGACY_REPOSITORY,
            revision,
            directory=directory,
        )
        entries = git(
            "ls-tree",
            "--format=%(objectmode) %(objecttype) %(path)",
            revision,
            "--",
            "README.md",
            "issue.json",
            *(f"{name}.json" for name in RESULT_NAMES),
            directory=directory,
        )
        for entry in entries.stdout.splitlines():
            mode, kind, name = entry.split(" ", 2)
            if mode not in {"100644", "100755"} or kind != "blob":
                raise ValueError(f"The legacy {name} is not a regular file.")
            files[name] = git("show", f"{revision}:{name}", directory=directory).stdout
    if "issue.json" not in files:
        raise ValueError("The legacy issue-context branch has no issue metadata.")
    data = {"repository": repository}
    data.update(
        (name.removesuffix(".json"), json.loads(content))
        for name, content in files.items()
        if name.endswith(".json")
    )
    validate_data(data, repository, issue_number)
    return Context(files.get("README.md"), data, digest(["legacy", revision]))


def read_context(repository, issue_number):
    return read_artifact(repository, issue_number) or read_legacy(
        repository, issue_number
    )


def output(**values):
    if path := os.environ.get("GITHUB_OUTPUT"):
        with Path(path).open("a") as file:
            file.writelines(
                f"{name.replace('_', '-')}={value}\n" for name, value in values.items()
            )


def read_file(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")
    return path.read_text()


def load(args):
    if not re.fullmatch(r"[A-Za-z0-9_./-]+", args.path):
        raise ValueError("The issue-context destination is invalid.")
    directory = Path(args.path).resolve()
    roots = [
        Path(os.environ[name]).resolve() for name in ("RUNNER_TEMP", "GITHUB_WORKSPACE")
    ]
    if not any(directory != root and directory.is_relative_to(root) for root in roots):
        raise ValueError(
            "The context destination must be inside the runner or workspace."
        )
    if directory.exists():
        raise ValueError("The issue-context destination already exists.")
    context = read_context(args.repository, args.issue_number)
    if context.readme is None:
        if args.template and args.issue_file:
            context.readme = read_file(args.template)
            context.data.update(
                repository=args.repository,
                issue=json.loads(read_file(args.issue_file)),
            )
            validate_data(context.data, args.repository, args.issue_number)
        elif args.allow_missing:
            output(exists="false")
            return
        else:
            raise ValueError("No triage report exists for this issue.")
    directory.mkdir(parents=True)
    (directory / "README.md").write_text(context.readme)
    for name in ("issue", *RESULT_NAMES):
        if name in context.data:
            (directory / f"{name}.json").write_text(
                json.dumps(context.data[name], indent=2) + "\n"
            )
    # A local snapshot lets the comment workflow reject changes outside README.md.
    git("init", "--quiet", directory=directory)
    git("add", "README.md", "*.json", directory=directory)
    git(
        "-c",
        "user.name=github-actions[bot]",
        "-c",
        "user.email=41898282+github-actions[bot]@users.noreply.github.com",
        "-c",
        "commit.gpgSign=false",
        "commit",
        "--quiet",
        "--message",
        "Load issue context",
        directory=directory,
    )
    output(exists="true", revision=context.revision, path=directory)


def save(args):
    if not re.fullmatch(r"[0-9a-f]{64}", args.expected_revision):
        raise ValueError("The expected issue-context revision is invalid.")
    # Callers serialize publishers per issue; the REST update has no version argument.
    context = read_context(args.repository, args.issue_number)
    if context.revision != args.expected_revision:
        print("The issue context changed during analysis; skipping the stale update.")
        output(current="false", changed="false")
        return
    data = context.data.copy()
    if args.issue_file:
        data.update(
            repository=args.repository, issue=json.loads(read_file(args.issue_file))
        )
    if args.result_file:
        if args.result_name not in RESULT_NAMES:
            raise ValueError("A structured result must have a supported result name.")
        data[args.result_name] = json.loads(read_file(args.result_file))
    validate_data(data, args.repository, args.issue_number)
    body = encode(read_file(args.readme), data)
    endpoint = f"repos/{args.repository}/issues/{args.issue_number}/artifacts"
    changed = True
    if context.artifact_number is None:
        artifact = api(
            endpoint,
            method="POST",
            payload={"name": ARTIFACT_NAME, "type": "generic", "body": body},
        )
        number = artifact["number"]
    else:
        number = context.artifact_number
        if body == encode(context.readme, context.data):
            changed = False
        else:
            api(f"{endpoint}/{number}", method="PATCH", payload={"body": body})
    url = f"https://github.com/{args.repository}/issues/{args.issue_number}/artifacts/{number}"
    print(f"Issue context: {url}")
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(path).open("a") as file:
            file.write(f"Issue context: {url}\n")
    output(current="true", changed=str(changed).lower(), url=url)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("load", "save"))
    parser.add_argument("--repository", required=True)
    parser.add_argument("--issue-number", required=True, type=int)
    parser.add_argument("--path")
    parser.add_argument("--allow-missing", action="store_true")
    parser.add_argument("--template")
    parser.add_argument("--issue-file")
    parser.add_argument("--readme")
    parser.add_argument("--expected-revision")
    parser.add_argument("--result-name", choices=RESULT_NAMES)
    parser.add_argument("--result-file")
    args = parser.parse_args()
    if (
        not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository)
        or args.issue_number < 1
    ):
        parser.error("The issue repository or number is invalid.")
    if args.operation == "load":
        if not args.path:
            parser.error("Loading issue context requires a destination path.")
        if bool(args.template) != bool(args.issue_file):
            parser.error(
                "Initializing issue context requires both a template and issue file."
            )
        load(args)
    else:
        if not args.readme or not args.expected_revision:
            parser.error(
                "Saving issue context requires a README and expected revision."
            )
        if bool(args.result_name) != bool(args.result_file):
            parser.error("A structured result requires both a name and file.")
        save(args)


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        RuntimeError,
        subprocess.CalledProcessError,
    ) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
