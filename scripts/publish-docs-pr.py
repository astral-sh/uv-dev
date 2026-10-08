"""Publish a documentation PR before retiring an older publication."""

import argparse
import json
import subprocess

REPOSITORY = "astral-sh/docs"
BOT_LOGINS = {"app/astral-releases-bot", "astral-docs-bot"}


def run(*arguments: str) -> str:
    return subprocess.check_output(arguments, text=True).strip()


def publish(branch: str, display_name: str) -> None:
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
        f"Automated documentation update for {display_name}",
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
            "number,title,baseRefName,headRefName,headRefOid,isCrossRepository,state",
        )
    )
    number = replacement.get("number")
    if (
        type(number) is not int
        or number <= 0
        or replacement.get("title") != title
        or replacement.get("baseRefName") != "main"
        or replacement.get("headRefName") != branch
        or replacement.get("headRefOid") != head
        or replacement.get("isCrossRepository") is not False
        or replacement.get("state") != "OPEN"
    ):
        raise RuntimeError(
            "The replacement documentation PR does not match the publication"
        )

    # Keep older publications usable until this exact replacement exists.
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
            "number,title,headRefName,isCrossRepository,author",
        )
    )
    prefix = branch.rsplit("-", 1)[0] + "-"
    for predecessor in predecessors:
        if (
            type(predecessor.get("number")) is int
            and 0 < predecessor["number"] < number
            and predecessor.get("title") == title
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
    args = parser.parse_args()
    publish(args.branch, args.display_name)


if __name__ == "__main__":
    main()
