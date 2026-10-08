"""Publish a documentation PR before retiring an older publication."""

import argparse
import json
import re
import subprocess

REPOSITORY = "astral-sh/docs"
BOT_LOGINS = {"app/astral-releases-bot", "astral-docs-bot"}


def run(*arguments: str) -> str:
    return subprocess.check_output(arguments, text=True).strip()


def publication_source(body: str) -> str | None:
    markers = [line for line in body.splitlines() if line.startswith("<!-- uv-source:")]
    if len(markers) != 1:
        return None
    match = re.fullmatch(r"<!-- uv-source: ([0-9a-f]{40}) -->", markers[0])
    return match[1] if match else None


def publish(branch: str, display_name: str, source_commit: str) -> None:
    if re.fullmatch(r"[0-9a-f]{40}", source_commit) is None:
        raise ValueError("Expected the full uv source commit")
    body = (
        f"Automated documentation update for {display_name}\n\n"
        f"<!-- uv-source: {source_commit} -->"
    )
    title = f"Update uv documentation for {display_name}"
    head = run("git", "rev-parse", f"refs/heads/{branch}")
    run("gh", "auth", "setup-git")
    run("git", "push", "origin", f"{head}:refs/heads/{branch}")
    run(
        "gh",
        "pr",
        "create",
        "--repo",
        REPOSITORY,
        "--base",
        "main",
        "--head",
        branch,
        "--title",
        title,
        "--body",
        body,
        "--label",
        "documentation",
    )
    replacement = json.loads(
        run(
            "gh",
            "pr",
            "view",
            branch,
            "--repo",
            REPOSITORY,
            "--json",
            "number,title,body,baseRefName,headRefName,headRefOid,isCrossRepository,state",
        )
    )
    number = replacement.get("number")
    if (
        type(number) is not int
        or number <= 0
        or replacement.get("title") != title
        or replacement.get("body") != body
        or replacement.get("baseRefName") != "main"
        or replacement.get("headRefName") != branch
        or replacement.get("headRefOid") != head
        or replacement.get("isCrossRepository") is not False
        or replacement.get("state") != "OPEN"
    ):
        raise RuntimeError(
            "The replacement documentation PR does not match the publication"
        )

    # Keep predecessors until the replacement exists. Only the same uv source
    # and version can be superseded; PR numbers do not order source revisions.
    predecessors = json.loads(
        run(
            "gh",
            "pr",
            "list",
            "--repo",
            REPOSITORY,
            "--state",
            "open",
            "--base",
            "main",
            "--label",
            "documentation",
            "--limit",
            "100",
            "--json",
            "number,title,body,headRefName,isCrossRepository,author",
        )
    )
    prefix = branch.rsplit("-", 1)[0] + "-"
    for predecessor in predecessors:
        if (
            type(predecessor.get("number")) is int
            and 0 < predecessor["number"] < number
            and predecessor.get("title") == title
            and publication_source(predecessor.get("body") or "") == source_commit
            and predecessor.get("headRefName", "").startswith(prefix)
            and predecessor.get("headRefName") != branch
            and predecessor.get("isCrossRepository") is False
            and (predecessor.get("author") or {}).get("login") in BOT_LOGINS
        ):
            run("gh", "pr", "close", str(predecessor["number"]), "--repo", REPOSITORY)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("branch")
    parser.add_argument("display_name")
    parser.add_argument("source_commit")
    args = parser.parse_args()
    publish(args.branch, args.display_name, args.source_commit)


if __name__ == "__main__":
    main()
