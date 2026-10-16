# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Reserve and find an automation comment near the start of uv-dev pull requests."""

import argparse
import json
import os
import subprocess
import sys

REPOSITORY = "astral-sh/uv-dev"
AUTOMATIONS_BOT_ID = 305554984
COMMENT_LIMIT = 3
COMMENT_MARKER = "<!-- uv-automation-comment -->"
COMMENT_BODY = f"{COMMENT_MARKER}\nReserved for automation updates."


def api(endpoint: str, *, body: str | None = None):
    command = ["gh", "api", "--method", "GET" if body is None else "POST", endpoint]
    if body is not None:
        command.extend(["--input", "-"])
    result = subprocess.run(
        command,
        input=json.dumps({"body": body}) if body is not None else None,
        stdout=subprocess.PIPE,
        text=True,
        check=True,
    )
    return json.loads(result.stdout)


def is_reserved(comment: dict) -> bool:
    author = comment.get("user") or {}
    return (
        author.get("id") == AUTOMATIONS_BOT_ID
        and author.get("type") == "Bot"
        and (comment.get("body") or "").startswith(f"{COMMENT_MARKER}\n")
    )


def first_comments(pull_request: int) -> list[dict]:
    return api(
        f"repos/{REPOSITORY}/issues/{pull_request}/comments?per_page={COMMENT_LIMIT}"
    )


def find_comment(comments: list[dict]) -> dict | None:
    return next((comment for comment in comments if is_reserved(comment)), None)


def reserve_comment(pull_request: int) -> dict | None:
    pull = api(f"repos/{REPOSITORY}/pulls/{pull_request}")
    if (
        pull["state"] != "open"
        or pull["base"]["repo"]["full_name"] != REPOSITORY
        or (pull["head"]["repo"] or {}).get("full_name") != REPOSITORY
    ):
        print("The pull request is not eligible for a reservation.", file=sys.stderr)
        return None

    comments = first_comments(pull_request)
    comment = find_comment(comments)
    if comment is not None:
        return comment
    if len(comments) >= COMMENT_LIMIT:
        print(
            "The first three comment positions are already occupied.", file=sys.stderr
        )
        return None

    api(f"repos/{REPOSITORY}/issues/{pull_request}/comments", body=COMMENT_BODY)

    # Other commenters do not share the workflow's concurrency group.
    comment = find_comment(first_comments(pull_request))
    if comment is None:
        print("No recognized reservation in the first three comments.", file=sys.stderr)
    return comment


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["reserve", "find"])
    parser.add_argument("--pull-request", required=True, type=int)
    args = parser.parse_args()
    if args.pull_request <= 0:
        parser.error("the pull request number must be positive")
    if os.environ.get("GITHUB_REPOSITORY", REPOSITORY) != REPOSITORY:
        parser.error(f"automation comments are only enabled for {REPOSITORY}")

    if args.command == "reserve":
        comment = reserve_comment(args.pull_request)
    else:
        comment = find_comment(first_comments(args.pull_request))
    if comment is not None:
        print(comment["id"])


if __name__ == "__main__":
    main()
