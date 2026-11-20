"""Prepare a PR diff for Codex and maintain the bot's behavior summary."""

import json
import os
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import quote

MARKER = "<!-- uv-pull-request-behavior-summary -->"
AUTOMATIONS_BOT_ID = 305554984
REPOSITORY = os.environ["GITHUB_REPOSITORY"]
NUMBER = int(os.environ["PULL_REQUEST_NUMBER"])
DIRECTORY = Path(os.environ["RUNNER_TEMP"]) / "behavior"


def api(endpoint: str, *args: str):
    return json.loads(
        subprocess.check_output(["gh", "api", f"repos/{REPOSITORY}/{endpoint}", *args])
    )


def revisions(pr) -> dict[str, str] | None:
    if pr["state"] != "open":
        return None
    head = pr["head"]["sha"]
    base = pr["base"]["sha"]
    comparison = api(f"compare/{base}...{head}?per_page=1")
    return {"base": comparison["merge_base_commit"]["sha"], "head": head}


def prepare() -> None:
    pr = api(f"pulls/{NUMBER}")
    commits = revisions(pr)
    if commits is None:
        return

    # Only fetch objects. In particular, PR-controlled Codex configuration and
    # instructions must never become files in the agent's working directory.
    DIRECTORY.mkdir()
    subprocess.run(["git", "init", "--quiet", DIRECTORY], check=True)
    subprocess.run(
        [
            "git",
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--no-recurse-submodules",
            f"https://github.com/{REPOSITORY}.git",
            *(f"{sha}:refs/behavior/{side}" for side, sha in commits.items()),
        ],
        cwd=DIRECTORY,
        check=True,
    )
    (DIRECTORY / "pull-request.json").write_text(
        json.dumps({"title": pr["title"], "body": pr["body"]})
    )
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        print(f"revisions={json.dumps(commits)}", file=output)


def verify_sources(changes, commits: dict[str, str]) -> None:
    for change in changes:
        for source in change["sources"]:
            sha = commits[source["revision"]]
            blob = subprocess.check_output(
                ["git", "cat-file", "blob", f"{sha}:{source['path']}"],
                cwd=DIRECTORY,
                stderr=subprocess.PIPE,
            )
            lines = blob.count(b"\n") + bool(blob and not blob.endswith(b"\n"))
            if not 1 <= source["line"] <= lines:
                raise ValueError(f"Source line outside {source['path']!r}")


def prose(value: str) -> str:
    """Allow inline code, keeping links and other markup under publisher control."""
    parts = value.strip().split("`")
    if len(parts) % 2 == 0 or "``" in value or not value.isprintable():
        raise ValueError("Expected a single line with balanced code spans")
    for index in range(0, len(parts), 2):
        if re.search(r"@|#\d|://|www\.", parts[index], re.IGNORECASE):
            raise ValueError("Unexpected mention, issue reference or URL in prose")
        parts[index] = re.sub(r"([\\*_~<>\[\]&])", r"\\\1", parts[index])
    return "`".join(parts)


def source_link(source, commits: dict[str, str]) -> str:
    sha = commits[source["revision"]]
    path = source["path"]
    line = source["line"]
    if (
        not path.isprintable()
        or any(part in {"", ".", ".."} for part in path.split("/"))
        or type(line) is not int
        or line < 1
    ):
        raise ValueError("Invalid source location")
    url = f"https://github.com/{REPOSITORY}/blob/{sha}/{quote(path)}#L{line}"
    return f"[{quote(Path(path).name)}:{line}]({url})"


def render(changes, commits: dict[str, str]) -> str | None:
    if not isinstance(changes, list):
        raise TypeError("Expected a list of behavior changes")
    if not changes:
        return None

    paragraphs = [f"{MARKER}\n### Behavior changes"]
    for index, change in enumerate(changes, 1):
        lead = prose(change["lead"])
        if not lead or not 1 <= len(change["sources"]) <= 2:
            raise ValueError("Each change needs a lead and supporting evidence")
        detail = prose(change["explanation"])
        links = " · ".join(source_link(source, commits) for source in change["sources"])
        prefix = f"{index}. " if len(changes) > 1 else ""
        paragraphs.append(
            " ".join(filter(None, (f"{prefix}**{lead}**", detail, links)))
        )
    return "\n\n".join(paragraphs)


def publish(changes, commits: dict[str, str]) -> None:
    endpoint = f"issues/{NUMBER}/comments"
    pages = api(f"{endpoint}?per_page=100", "--paginate", "--slurp")
    existing = next(
        (
            comment
            for page in pages
            for comment in page
            if comment["user"]["id"] == AUTOMATIONS_BOT_ID
            and comment["body"].startswith(MARKER + "\n")
        ),
        None,
    )
    current = revisions(api(f"pulls/{NUMBER}"))
    if current is None or current != commits:
        print("Skipping a closed or superseded pull request.")
        return

    body = render(changes, current)
    if body == (existing["body"] if existing else None):
        return
    method = "POST"
    if existing:
        endpoint = f"issues/comments/{existing['id']}"
        method = "PATCH" if body else "DELETE"
    command = [
        "gh",
        "api",
        f"repos/{REPOSITORY}/{endpoint}",
        "--silent",
        "--method",
        method,
    ]
    if body:
        command.extend(["--input", "-"])
    subprocess.run(
        command,
        input=json.dumps({"body": body}).encode() if body else None,
        env={**os.environ, "GH_TOKEN": os.environ["BOT_TOKEN"]},
        check=True,
    )


if __name__ == "__main__":
    if sys.argv[1] == "prepare":
        prepare()
    else:
        operation = {"verify": verify_sources, "publish": publish}[sys.argv[1]]
        changes = json.loads(os.environ["RESULT"])["changes"]
        if changes is None:
            sys.exit("Codex could not inspect the revisions.")
        operation(changes, json.loads(os.environ["REVISIONS"]))
