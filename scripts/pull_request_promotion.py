"""Plan and publish pull requests from trusted workflow checkouts."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any
from urllib.parse import quote

UV = "astral-sh/uv"
UV_DEV = "astral-sh/uv-dev"
REPOSITORY_IDS = {UV: 699532645, UV_DEV: 1302176231}
BOT = "astral-automations-bot[bot]"
PROMOTION_PREFIX = "automations/promote/"


class PromotionError(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PromotionError(message)


def run(*arguments: str, input: str | None = None) -> str:
    return subprocess.run(
        arguments, input=input, check=True, text=True, stdout=subprocess.PIPE
    ).stdout.strip()


class GitHub:
    def api(self, path: str, method: str = "GET", **data: Any) -> Any:
        arguments = ["gh", "api", "--method", method, path]
        if data:
            arguments.extend(["--input", "-"])
        result = run(*arguments, input=json.dumps(data) if data else None)
        return json.loads(result) if result else None

    def pulls(self, repository: str, **filters: str) -> list[dict[str, Any]]:
        arguments = [
            "gh",
            "pr",
            "list",
            "--repo",
            repository,
            "--state",
            filters.pop("state", "all"),
            "--limit",
            "1000",
            "--json",
            "number",
        ]
        for key, value in filters.items():
            arguments.extend([f"--{key}", value])
        return [
            self.api(f"repos/{repository}/pulls/{item['number']}")
            for item in json.loads(run(*arguments))
        ]

    def events(self, number: int) -> list[dict[str, Any]]:
        pages = json.loads(
            run(
                "gh",
                "api",
                f"repos/{UV_DEV}/issues/{number}/events",
                "--paginate",
                "--slurp",
            )
        )
        return [event for page in pages for event in page]

    def ref(self, repository: str, branch: str) -> str | None:
        reference = f"refs/heads/{branch}"
        matches = self.api(
            f"repos/{repository}/git/matching-refs/heads/{quote(branch, safe='')}"
        )
        return next(
            (item["object"]["sha"] for item in matches if item["ref"] == reference),
            None,
        )


def repository_is(value: dict[str, Any] | None, repository: str) -> bool:
    return (
        bool(value)
        and value.get("id") == REPOSITORY_IDS[repository]
        and value.get("full_name") == repository
    )


def promotion_ref(number: int) -> str:
    return f"{PROMOTION_PREFIX}{number}"


def marker(source: dict[str, Any]) -> str:
    return f"<!-- uv-dev-promotion:{source['number']}:{source['head']['sha']} -->"


def source_is_valid(source: dict[str, Any]) -> bool:
    return (
        repository_is(source["base"]["repo"], UV_DEV)
        and repository_is(source["head"]["repo"], UV_DEV)
        and not source["head"]["ref"].startswith(PROMOTION_PREFIX)
    )


def promoter(github: GitHub, number: int) -> str:
    promoters = [
        event["actor"]["login"]
        for event in github.events(number)
        if event["event"] == "ready_for_review"
        and (event.get("actor") or {}).get("type") == "User"
    ]
    require(bool(promoters), "The source has no human promoter.")
    return promoters[-1]


def find_promotion(github: GitHub, source: dict[str, Any]) -> dict[str, Any] | None:
    """Recognize dedicated branches and both historical promotion formats."""
    candidates = []
    for branch in (promotion_ref(source["number"]), source["head"]["ref"]):
        for candidate in github.pulls(UV, head=branch):
            if not repository_is(candidate["base"]["repo"], UV):
                continue
            head = candidate["head"]
            if head["ref"] != branch:
                continue
            if branch == promotion_ref(source["number"]):
                if not repository_is(head["repo"], UV_DEV):
                    continue
                require(
                    candidate["user"]["login"] == BOT
                    and (candidate["body"] or "").rstrip().endswith(marker(source)),
                    "The dedicated promotion does not match its source revision.",
                )
            elif not (
                repository_is(head["repo"], UV) or repository_is(head["repo"], UV_DEV)
            ):
                continue
            else:
                require(
                    head["sha"] == source["head"]["sha"],
                    "The historical promotion has a different head revision.",
                )
            candidates.append(candidate)
    require(len(candidates) <= 1, "Found multiple upstream promotions.")
    return candidates[0] if candidates else None


def plan(github: GitHub, number: int, expected_head: str = "") -> dict[str, Any]:
    require(number > 0, "Invalid source pull request number.")
    require(
        not expected_head or re.fullmatch("[0-9a-f]{40}", expected_head) is not None,
        "Invalid expected head revision.",
    )
    source = github.api(f"repos/{UV_DEV}/pulls/{number}")
    require(
        source_is_valid(source) and source["state"] == "open" and not source["draft"],
        "The source pull request is no longer ready.",
    )
    require(
        not expected_head or source["head"]["sha"] == expected_head,
        "The source pull request head changed.",
    )
    result = {
        "number": number,
        "promoter": promoter(github, number),
        "head_ref": source["head"]["ref"],
        "head_sha": source["head"]["sha"],
        "source_base": source["base"]["ref"],
        "base_ref": source["base"]["ref"],
        "base_sha": "",
        "previous_sha": "",
        "merge_sha": "",
    }
    existing = find_promotion(github, source)
    if existing:
        require(
            existing["state"] == "open" or existing["merged_at"] is not None,
            "The existing promotion was closed without merging.",
        )
        return {**result, "base_ref": existing["base"]["ref"], "action": "promote"}

    if result["source_base"] != "main":
        parents = [
            parent
            for parent in github.pulls(UV_DEV, head=result["source_base"])
            if source_is_valid(parent)
            and parent["head"]["ref"] == result["source_base"]
        ]
        require(len(parents) <= 1, "The source branch has multiple possible parents.")
        if parents:
            parent = parents[0]
            require(
                parent["head"]["sha"] == source["base"]["sha"],
                "The source parent no longer matches the stacked base.",
            )
            promoted_parent = find_promotion(github, parent)
            if promoted_parent is None or promoted_parent["state"] == "open":
                return {
                    **result,
                    "action": "wait",
                    "reason": f"Waiting for astral-sh/uv-dev#{parent['number']} to merge upstream.",
                }
            require(
                promoted_parent["merged_at"] is not None,
                "The upstream parent was closed without merging.",
            )
            result.update(
                base_ref=promoted_parent["base"]["ref"],
                previous_sha=parent["head"]["sha"],
                merge_sha=promoted_parent["merge_commit_sha"],
            )
            # A historical promotion may itself have merged into a copied stack
            # branch. Wait for those ancestors too, then drop the whole old range.
            visited = {result["source_base"]}
            while result["base_ref"] != "main":
                require(
                    result["base_ref"] not in visited,
                    "The upstream stack contains a cycle.",
                )
                visited.add(result["base_ref"])
                ancestors = [
                    candidate
                    for candidate in github.pulls(UV, head=result["base_ref"])
                    if repository_is(candidate["base"]["repo"], UV)
                    and repository_is(candidate["head"]["repo"], UV)
                    and candidate["head"]["ref"] == result["base_ref"]
                ]
                require(
                    len(ancestors) <= 1,
                    "The upstream stack has multiple possible parents.",
                )
                if not ancestors:
                    break
                ancestor = ancestors[0]
                if ancestor["state"] == "open":
                    return {
                        **result,
                        "action": "wait",
                        "reason": f"Waiting for astral-sh/uv#{ancestor['number']} to merge.",
                    }
                require(
                    ancestor["merged_at"] is not None,
                    "An upstream ancestor was closed without merging.",
                )
                result.update(
                    base_ref=ancestor["base"]["ref"],
                    merge_sha=ancestor["merge_commit_sha"],
                )
        else:
            # Do not treat a legacy copied stack branch as a normal upstream base.
            parents = [
                parent
                for parent in github.pulls(UV, head=result["source_base"])
                if repository_is(parent["base"]["repo"], UV)
                and repository_is(parent["head"]["repo"], UV)
                and parent["head"]["ref"] == result["source_base"]
            ]
            require(not parents, "The stacked base has no verified uv-dev parent.")

    upstream_base = github.ref(UV, result["base_ref"])
    require(upstream_base is not None, "The upstream base branch does not exist.")
    if result["previous_sha"]:
        base_sha = github.ref(UV_DEV, result["base_ref"])
        require(base_sha is not None, "The new base branch does not exist in uv-dev.")
        comparison = github.api(
            f"repos/{UV_DEV}/compare/{result['merge_sha']}...{base_sha}"
        )
        if comparison["status"] not in ("ahead", "identical"):
            return {
                **result,
                "action": "wait",
                "reason": "Waiting for the merged parent to sync to uv-dev.",
            }
        return {**result, "base_sha": base_sha, "action": "rebase"}
    return {**result, "action": "promote"}


def same_source(source: dict[str, Any], expected: dict[str, Any]) -> bool:
    return (
        source_is_valid(source)
        and source["state"] == "open"
        and not source["draft"]
        and source["head"]["ref"] == expected["head_ref"]
        and source["head"]["sha"] == expected["head_sha"]
        and source["base"]["ref"] == expected["source_base"]
    )


def publish(github: GitHub, expected: dict[str, Any], promoted_head: str) -> int:
    number = expected["number"]
    source = github.api(f"repos/{UV_DEV}/pulls/{number}")
    require(same_source(source, expected), "The source changed before publication.")
    current = plan(github, number, expected["head_sha"])
    require(current == expected, "The promotion plan changed before publication.")
    existing = find_promotion(github, source)
    if existing is None:
        branch = promotion_ref(number)
        current_head = github.ref(UV_DEV, branch)
        require(
            current_head is None or current_head == promoted_head,
            "The promotion branch already exists at a different revision.",
        )
        if current_head is None:
            if expected["action"] == "rebase":
                # The imported bundle has never been checked out in this writer job.
                run(
                    "git",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "credential.helper=",
                    "-c",
                    "credential.helper=!gh auth git-credential",
                    "push",
                    f"--force-with-lease=refs/heads/{branch}:",
                    f"https://github.com/{UV_DEV}.git",
                    f"{promoted_head}:refs/heads/{branch}",
                )
            else:
                require(
                    promoted_head == expected["head_sha"],
                    "Unexpected promotion revision.",
                )
                github.api(
                    f"repos/{UV_DEV}/git/refs",
                    "POST",
                    ref=f"refs/heads/{branch}",
                    sha=promoted_head,
                )
        body = re.sub(
            r"(?<![\w/])#([0-9]+)(?!\w)", rf"{UV_DEV}#\1", source["body"] or ""
        )
        existing = github.api(
            f"repos/{UV}/pulls",
            "POST",
            title=source["title"],
            body=f"{body.rstrip()}\n\n{marker(source)}",
            base=expected["base_ref"],
            head=f"astral-sh:{branch}",
            head_repo=UV_DEV,
        )
        require(
            repository_is(existing["base"]["repo"], UV)
            and repository_is(existing["head"]["repo"], UV_DEV)
            and existing["head"]["ref"] == branch
            and existing["head"]["sha"] == promoted_head,
            "The created upstream pull request has an unexpected head.",
        )
    upstream_number = existing["number"]
    labels = [
        label["name"]
        for label in source["labels"]
        if not label["name"].startswith("bot:")
    ]
    if labels:
        github.api(f"repos/{UV}/issues/{upstream_number}/labels", "POST", labels=labels)
    github.api(
        f"repos/{UV}/issues/{upstream_number}/assignees",
        "POST",
        assignees=[expected["promoter"]],
    )
    require(
        same_source(github.api(f"repos/{UV_DEV}/pulls/{number}"), expected),
        "The source changed before it could be closed.",
    )
    run(
        "gh",
        "pr",
        "close",
        str(number),
        "--repo",
        UV_DEV,
        "--comment",
        f"Promoted to [#{upstream_number}](https://github.com/{UV}/pull/{upstream_number}).",
    )
    return upstream_number


def output(name: str, value: Any) -> None:
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as file:
        file.write(f"{name}={json.dumps(value, separators=(',', ':'))}\n")


def summary(message: str) -> None:
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as file:
        file.write(f"{message}\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    identify = commands.add_parser("plan")
    identify.add_argument("--number", type=int)
    identify.add_argument("--head-sha", default="")
    publish_command = commands.add_parser("publish")
    publish_command.add_argument("--plan", required=True, type=json.loads)
    publish_command.add_argument("--promoted-head", required=True)
    verify_plan = commands.add_parser("verify-plan")
    verify_plan.add_argument("--plan", required=True, type=json.loads)
    verify = commands.add_parser("verify-regression")
    verify.add_argument("--number", type=int, required=True)
    verify.add_argument("--head-ref", required=True)
    verify.add_argument("--head-sha", required=True)
    args = parser.parse_args()
    github = GitHub()
    if args.command == "plan":
        numbers = (
            [args.number]
            if args.number
            else [
                source["number"]
                for source in github.pulls(UV_DEV, state="open")
                if source["state"] == "open"
                and not source["draft"]
                and source_is_valid(source)
            ]
        )
        plans = []
        for number in numbers:
            try:
                candidate = plan(github, number, args.head_sha)
                if candidate["action"] == "wait":
                    summary(f"- uv-dev#{number}: {candidate['reason']}")
                else:
                    plans.append(candidate)
            except PromotionError as error:
                if args.number:
                    raise
                summary(f"- uv-dev#{number}: {error}")
        require(len(plans) <= 256, "The promotion matrix exceeds 256 pull requests.")
        rebases = [candidate for candidate in plans if candidate["action"] == "rebase"]
        output("promotions", {"include": plans})
        output("rebases", {"include": rebases})
        output("promotion_count", len(plans))
        output("rebase_count", len(rebases))
    elif args.command == "verify-plan":
        require(
            plan(github, args.plan["number"], args.plan["head_sha"]) == args.plan,
            "The promotion plan changed.",
        )
    elif args.command == "publish":
        number = publish(github, args.plan, args.promoted_head)
        summary(
            f"Promoted uv-dev#{args.plan['number']} to https://github.com/{UV}/pull/{number}."
        )
    else:
        source = github.api(f"repos/{UV_DEV}/pulls/{args.number}")
        require(
            source_is_valid(source)
            and source["head"]["ref"] == args.head_ref
            and source["head"]["sha"] == args.head_sha,
            "The regression source changed.",
        )
        promoted = find_promotion(github, source)
        require(
            promoted is not None
            and promoted["base"]["ref"] == "main"
            and (promoted["state"] == "open" or promoted["merged_at"] is not None),
            "The closed regression-test pull request was not promoted at its verified head.",
        )


if __name__ == "__main__":
    main()
