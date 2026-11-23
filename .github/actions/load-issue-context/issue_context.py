"""Load and publish issue triage reports for GitHub Actions.

An issue artifact contains the Markdown report followed by its structured results.
Workflows edit local files and pass the source revision back to the publisher.
"""

from __future__ import annotations

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


@dataclass
class Context:
    readme: str | None
    data: dict
    revision: str
    artifact_number: int | None = None


def api(endpoint: str, *, method: str = "GET", payload: dict | None = None):
    command = ["gh", "api", endpoint, "--method", method]
    command.extend(["--header", "X-GitHub-Api-Version: 2026-03-10"])
    body = None
    if payload is not None:
        command.extend(["--input", "-"])
        body = json.dumps(payload)
    return json.loads(subprocess.check_output(command, input=body, text=True))


def digest(value: list) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def git(
    *arguments: str, directory: str | None = None, check: bool = True
) -> subprocess.CompletedProcess[str]:
    command = ["git", "-c", "core.hooksPath=/dev/null", "-c", "credential.helper="]
    return subprocess.run(
        [*command, *arguments],
        cwd=directory,
        env=os.environ | {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"},
        text=True,
        stdout=subprocess.PIPE,
        check=check,
    )


def validate_data(data: dict, repository: str, issue_number: int) -> None:
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


def encode(readme: str, data: dict) -> str:
    return readme.rstrip() + DATA_HEADER + json.dumps(data, indent=2) + DATA_FOOTER


def decode(body: str, repository: str, issue_number: int) -> tuple[str, dict]:
    readme, separator, structured = body.rpartition(DATA_HEADER)
    if not separator or not structured.endswith(DATA_FOOTER):
        raise ValueError("The triage artifact has no supported structured results.")
    data = json.loads(structured.removesuffix(DATA_FOOTER))
    validate_data(data, repository, issue_number)
    return readme + "\n", data


def is_triage_artifact(artifact: dict) -> bool:
    return (
        artifact["name"] == ARTIFACT_NAME
        and artifact["type"] == "generic"
        and (artifact.get("creator") or {}).get("login") == ARTIFACT_CREATOR
    )


def find_artifact(endpoint: str) -> int | None:
    number = None
    page = 1
    while True:
        artifacts = api(f"{endpoint}?per_page=100&page={page}")
        if not isinstance(artifacts, list):
            raise TypeError("GitHub returned an invalid issue artifact list.")
        for artifact in artifacts:
            if not is_triage_artifact(artifact):
                continue
            if number is not None:
                raise ValueError("The issue has more than one triage artifact.")
            number = artifact["number"]
            if type(number) is not int or number < 1:
                raise ValueError("The triage artifact number is invalid.")
        # This endpoint does not return pagination links.
        if len(artifacts) < 100:
            return number
        page += 1


def read_context(repository: str, issue_number: int) -> Context:
    endpoint = f"repos/{repository}/issues/{issue_number}/artifacts"
    number = find_artifact(endpoint)
    if number is None:
        return read_legacy(repository, issue_number)

    response = api(f"{endpoint}/{number}?per_page=1")
    artifact = response["artifact"]
    if artifact["number"] != number or not is_triage_artifact(artifact):
        raise ValueError("The triage artifact is not owned by this automation.")
    versions = response["versions"]
    latest = versions[0] if versions else {}
    version = latest.get("version")
    if type(version) is not int or version < 1:
        raise ValueError("The triage artifact has no valid version history.")
    if (latest.get("actor") or {}).get("login") != ARTIFACT_CREATOR:
        raise ValueError(
            "The latest triage results were not written by this automation."
        )
    readme, data = decode(artifact["body"], repository, issue_number)
    # Restoring an older body still invalidates analyses of the previous version.
    revision = digest(["artifact", artifact["id"], number, version, artifact["body"]])
    return Context(readme, data, revision, number)


def read_legacy(repository: str, issue_number: int) -> Context:
    # Reports written before issue artifacts remain available until their next update.
    # Public Git reads do not require a cross-repository contents token.
    reference = git(
        "ls-remote",
        "--exit-code",
        LEGACY_REPOSITORY,
        f"refs/heads/issue/{issue_number}",
        check=False,
    )
    if reference.returncode == 2:  # No matching branch.
        return Context(None, {}, digest(["missing"]))
    reference.check_returncode()
    revision = reference.stdout.split()[0]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("The legacy issue-context revision is invalid.")
    # Read only regular blobs, without checking out untrusted branch contents.
    readme = None
    data = {"repository": repository}
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
        # An issue branch can still point to the empty issue/base commit.
        if not entries.stdout:
            return Context(None, {}, digest(["legacy", revision]))
        for entry in entries.stdout.splitlines():
            mode, kind, name = entry.split(" ", 2)
            if mode not in {"100644", "100755"} or kind != "blob":
                raise ValueError(f"The legacy {name} is not a regular file.")
            content = git("show", f"{revision}:{name}", directory=directory).stdout
            if name == "README.md":
                readme = content
            else:
                data[name.removesuffix(".json")] = json.loads(content)
    validate_data(data, repository, issue_number)
    return Context(readme, data, digest(["legacy", revision]))


def output(**values: str) -> None:
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as file:
        for name, value in values.items():
            print(f"{name}={value}", file=file)


def read_file(filename: str) -> str:
    path = Path(filename)
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")
    return path.read_text()


def destination(path: str) -> Path:
    if not re.fullmatch(r"[A-Za-z0-9_./-]+", path):
        raise ValueError("The issue-context destination is invalid.")
    directory = Path(path).resolve()
    roots = [
        Path(os.environ[name]).resolve() for name in ("RUNNER_TEMP", "GITHUB_WORKSPACE")
    ]
    if not any(directory != root and directory.is_relative_to(root) for root in roots):
        raise ValueError(
            "The context destination must be inside the runner or workspace."
        )
    if directory.exists():
        raise ValueError("The issue-context destination already exists.")
    return directory


def load(repository: str, issue_number: int) -> None:
    directory = destination(os.environ["DIRECTORY"])
    template = os.environ["TEMPLATE"]
    issue_file = os.environ["ISSUE_FILE"]
    if bool(template) != bool(issue_file):
        raise ValueError(
            "Initializing context requires both a template and issue file."
        )

    context = read_context(repository, issue_number)
    # Early issue branches contain JSON results but no Markdown report.
    if context.readme is None:
        if template:
            context.readme = read_file(template)
            context.data.update(
                repository=repository,
                issue=json.loads(read_file(issue_file)),
            )
            validate_data(context.data, repository, issue_number)
            output(issue=json.dumps(context.data["issue"], ensure_ascii=False))
        elif os.environ["ALLOW_MISSING"] == "true":
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
    output(exists="true", revision=context.revision, path=str(directory))


def save(repository: str, issue_number: int) -> None:
    expected_revision = os.environ["EXPECTED_REVISION"]
    issue_file = os.environ["ISSUE_FILE"]
    result_name = os.environ["RESULT_NAME"]
    result_file = os.environ["RESULT_FILE"]
    if bool(result_name) != bool(result_file):
        raise ValueError("A structured result requires both a name and file.")
    if not re.fullmatch(r"[0-9a-f]{64}", expected_revision):
        raise ValueError("The expected issue-context revision is invalid.")

    # Callers serialize publishers per issue because the API has no conditional update.
    context = read_context(repository, issue_number)
    if context.revision != expected_revision:
        print("Skipping the update because the report changed during analysis.")
        output(current="false", changed="false")
        return

    data = context.data.copy()
    if issue_file:
        data.update(repository=repository, issue=json.loads(read_file(issue_file)))
    if result_file:
        if result_name not in RESULT_NAMES:
            raise ValueError("A structured result must have a supported result name.")
        data[result_name] = json.loads(read_file(result_file))
    validate_data(data, repository, issue_number)

    body = encode(read_file(os.environ["README"]), data)
    endpoint = f"repos/{repository}/issues/{issue_number}/artifacts"
    number = context.artifact_number
    changed = True
    if number is None:
        artifact = api(
            endpoint,
            method="POST",
            payload={"name": ARTIFACT_NAME, "type": "generic", "body": body},
        )
        number = artifact["number"]
    elif context.readme is not None and body == encode(context.readme, context.data):
        changed = False
    else:
        api(f"{endpoint}/{number}", method="PATCH", payload={"body": body})

    url = f"https://github.com/{repository}/issues/{issue_number}/artifacts/{number}"
    print(f"Issue context: {url}")
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(path).open("a") as file:
            file.write(f"Issue context: {url}\n")
    output(current="true", changed=str(changed).lower(), url=url)


def main() -> None:
    repository = os.environ["REPOSITORY"]
    number = os.environ["ISSUE_NUMBER"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Invalid issue repository.")
    if not re.fullmatch(r"[1-9][0-9]*", number):
        raise ValueError("Invalid issue number.")

    if sys.argv[1:] == ["load"]:
        load(repository, int(number))
    elif sys.argv[1:] == ["save"]:
        save(repository, int(number))
    else:
        raise ValueError("Expected load or save.")


if __name__ == "__main__":
    main()
