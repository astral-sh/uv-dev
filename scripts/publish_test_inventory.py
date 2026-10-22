"""Compare CI test inventories and publish a pull request report."""

from __future__ import annotations

import argparse
import html
import io
import json
import os
import re
import subprocess
import sys
import urllib.parse
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

REPOSITORIES = {"astral-sh/uv": 699532645, "astral-sh/uv-dev": 1302176231}
BOT_ID = 305554984
MARKER = "<!-- generated-comment uv test-inventory -->"
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024
MAX_XML_BYTES = 32 * 1024 * 1024
MAX_TOTAL_XML_BYTES = 128 * 1024 * 1024
MAX_ARTIFACTS = 32
MAX_DETAILS_CHARACTERS = 52_000


def api(repository: str, endpoint: str, *, method: str = "GET", **data):
    command = ["gh", "api", f"repos/{repository}/{endpoint}", "--method", method]
    if data:
        command += ["--input", "-"]
    result = subprocess.run(
        command,
        input=json.dumps(data) if data else None,
        text=True,
        capture_output=True,
        check=False,
        timeout=120,
    )
    if result.returncode:
        if method == "GET" and "(HTTP 404)" in result.stderr:
            return None
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout) if result.stdout else None


def pages(repository: str, endpoint: str, key: str | None = None):
    separator = "&" if "?" in endpoint else "?"
    for page in range(1, 101):
        payload = api(repository, f"{endpoint}{separator}per_page=100&page={page}")
        items = payload[key] if key else payload
        yield from items
        if len(items) < 100:
            return
    raise ValueError("The GitHub response exceeds the pagination limit.")


def current_run(repository: str, run_id: int, attempt: int, head_sha: str):
    run = api(repository, f"actions/runs/{run_id}")
    if (
        run is None
        or run["repository"]["id"] != REPOSITORIES[repository]
        or run["event"] != "pull_request"
        or run["path"] != ".github/workflows/ci.yml"
        or run["head_sha"] != head_sha
    ):
        raise ValueError("The source is not the requested pull request CI run.")
    if (
        run["run_attempt"] != attempt
        or run["status"] != "completed"
        or run["conclusion"] != "success"
    ):
        print(
            "The CI run is no longer a completed, successful attempt.", file=sys.stderr
        )
        return None
    return run


def matching_pull(repository: str, run: dict):
    head_repository = run["head_repository"]
    if not head_repository:
        return None
    if run["pull_requests"]:
        candidates = [
            api(repository, f"pulls/{pull['number']}") for pull in run["pull_requests"]
        ]
    else:
        query = urllib.parse.urlencode(
            {
                "state": "open",
                "head": f"{head_repository['owner']['login']}:{run['head_branch']}",
            }
        )
        candidates = list(pages(repository, f"pulls?{query}"))
    matches = [
        pull
        for pull in candidates
        if pull
        and pull["state"] == "open"
        and pull["base"]["repo"]["id"] == REPOSITORIES[repository]
        and (pull["head"]["repo"] or {}).get("id") == head_repository["id"]
        and pull["head"]["sha"] == run["head_sha"]
        and pull["head"]["ref"] == run["head_branch"]
    ]
    return matches[0] if len(matches) == 1 else None


def artifacts(repository: str, run_id: int) -> dict[str, dict]:
    found = {}
    for artifact in pages(repository, f"actions/runs/{run_id}/artifacts", "artifacts"):
        name = artifact["name"]
        if artifact["expired"] or not (
            name.startswith("junit-results-") or name == "test-inventory-revision"
        ):
            continue
        if name not in found or artifact["id"] > found[name]["id"]:
            found[name] = artifact
    if len(found) > MAX_ARTIFACTS:
        raise ValueError("Too many test inventory artifacts.")
    return found


def download(repository: str, artifact: dict) -> zipfile.ZipFile:
    if artifact["size_in_bytes"] > MAX_ARCHIVE_BYTES:
        raise ValueError("The artifact exceeds the download limit.")
    result = subprocess.run(
        ["gh", "api", f"repos/{repository}/actions/artifacts/{artifact['id']}/zip"],
        stdout=subprocess.PIPE,
        check=True,
        timeout=120,
    )
    if len(result.stdout) > MAX_ARCHIVE_BYTES:
        raise ValueError("The artifact exceeds the download limit.")
    return zipfile.ZipFile(io.BytesIO(result.stdout))


def read_member(archive: zipfile.ZipFile, member: zipfile.ZipInfo, limit: int) -> bytes:
    if member.is_dir() or member.file_size > limit:
        raise ValueError("The artifact member exceeds the uncompressed size limit.")
    # Read bounded members in memory; archive paths never become filesystem paths.
    with archive.open(member) as file:
        contents = file.read(limit + 1)
    if len(contents) > limit:
        raise ValueError("The artifact member exceeds the uncompressed size limit.")
    return contents


def revision(repository: str, artifact: dict) -> dict:
    with download(repository, artifact) as archive:
        members = [
            member
            for member in archive.infolist()
            if member.filename == "revision.json"
        ]
        if len(members) != 1:
            raise ValueError("Expected exactly one revision.json member.")
        result = json.loads(read_member(archive, members[0], 4096))
    if not isinstance(result, dict) or set(result) != {"merge", "base", "head"}:
        raise ValueError("Invalid tested revision.")
    if not all(
        isinstance(sha, str) and re.fullmatch("[0-9a-f]{40}", sha)
        for sha in result.values()
    ):
        raise ValueError("Invalid tested revision SHA.")
    return result


def inventory(contents: bytes) -> dict[str, set[tuple[str, str]]]:
    document = contents.decode("utf-8")
    if "<!DOCTYPE" in document or "<!ENTITY" in document or "\0" in document:
        raise ValueError("JUnit reports must not declare entities or document types.")
    root = ET.fromstring(document)
    if root.tag not in {"testsuites", "testsuite"}:
        raise ValueError("Expected a JUnit test suite.")
    result = {}
    for suite in root.iter("testsuite"):
        name = suite.attrib["name"]
        tests = result.setdefault(name, set())
        for case in suite.findall("testcase"):
            tests.add((case.get("classname", ""), case.attrib["name"]))
    return result


def read_inventory(
    repository: str, selected: list[dict]
) -> dict[str, set[tuple[str, str]]]:
    result = {}
    total_bytes = 0
    for artifact in selected:
        with download(repository, artifact) as archive:
            members = [
                member
                for member in archive.infolist()
                if member.filename.endswith("junit.xml")
            ]
            if not members or len({member.filename for member in members}) != len(
                members
            ):
                raise ValueError(
                    "Expected uniquely named JUnit reports in each artifact."
                )
            for member in members:
                total_bytes += member.file_size
                if total_bytes > MAX_TOTAL_XML_BYTES:
                    raise ValueError(
                        "The combined JUnit reports exceed the size limit."
                    )
                for suite, tests in inventory(
                    read_member(archive, member, MAX_XML_BYTES)
                ).items():
                    result.setdefault(suite, set()).update(tests)
    return result


def latest_run(repository: str, branch: str, sha: str, events: set[str]):
    query = urllib.parse.urlencode(
        {"branch": branch, "head_sha": sha, "status": "completed"}
    )
    runs = pages(repository, f"actions/workflows/ci.yml/runs?{query}", "workflow_runs")
    return next(
        (
            run
            for run in runs
            if run["event"] in events
            and run["head_sha"] == sha
            and run["head_branch"] == branch
            and run["repository"]["id"] == REPOSITORIES[repository]
            and run["path"] == ".github/workflows/ci.yml"
        ),
        None,
    )


def baseline_run(repository: str, branch: str, sha: str):
    if run := latest_run(repository, branch, sha, {"push", "workflow_dispatch"}):
        return run
    # A stacked base may have CI only on the PR that produced its merge commit.
    for pull in pages(repository, f"commits/{sha}/pulls"):
        if not (
            pull["merged_at"]
            and pull["merge_commit_sha"] == sha
            and pull["base"]["ref"] == branch
            and pull["base"]["repo"]["id"] == REPOSITORIES[repository]
        ):
            continue
        run = latest_run(
            repository, pull["head"]["ref"], pull["head"]["sha"], {"pull_request"}
        )
        if run is None:
            continue
        metadata = artifacts(repository, run["id"]).get("test-inventory-revision")
        if metadata and revision(repository, metadata)["merge"] == sha:
            return run
    return None


def render(baseline: dict, current: dict, branch: str) -> tuple[str, bool]:
    diffs = [
        (
            suite,
            sorted(current.get(suite, set()) - baseline.get(suite, set())),
            sorted(baseline.get(suite, set()) - current.get(suite, set())),
        )
        for suite in sorted(baseline.keys() | current.keys())
        if baseline.get(suite, set()) != current.get(suite, set())
    ]
    title = "## uv test inventory changes\n\n"
    base = f"<code>{html.escape(branch)}</code>"
    if not diffs:
        return (
            title
            + f"No differences were found in the matching CI test inventories for this PR and the {base} base revision.",
            False,
        )
    body = (
        title
        + f"The matching CI test inventories differ from the {base} base revision.\n\n"
        f"- Added tests: **{sum(len(added) for _, added, _ in diffs)}**\n"
        f"- Removed tests: **{sum(len(removed) for _, _, removed in diffs)}**\n"
        f"- Changed suites: **{len(diffs)}**"
    )
    details = ""
    for suite, added, removed in diffs:
        block = (
            f"<details>\n<summary><code>{html.escape(suite)}</code>: "
            f"+{len(added)} / -{len(removed)}</summary>\n\n"
        )
        for label, tests in (("Added", added), ("Removed", removed)):
            block += f"**{label}:**"
            block += "\n" if tests else " none\n"
            for classname, name in tests:
                key = f"{classname}::{name}" if classname else name
                block += f"- <code>{html.escape(key)}</code>\n"
            block += "\n"
        block += "</details>\n\n"
        if len(details) + len(block) > MAX_DETAILS_CHARACTERS:
            details += "_Suite details were truncated to stay within GitHub's comment size limit._"
            break
        details += block
    return f"{body}\n\n{details.rstrip()}", True


def prepare(repository: str, run_id: int, attempt: int, head_sha: str):
    run = current_run(repository, run_id, attempt, head_sha)
    if run is None or (pull := matching_pull(repository, run)) is None:
        return None
    current_artifacts = artifacts(repository, run_id)
    metadata = current_artifacts.get("test-inventory-revision")
    if metadata is None:
        print("The CI run did not record its tested revision.", file=sys.stderr)
        return None
    tested = revision(repository, metadata)
    if tested["head"] != head_sha or tested["merge"] != pull["merge_commit_sha"]:
        print("The PR no longer matches the tested merge revision.", file=sys.stderr)
        return None
    commit = api(repository, f"commits/{tested['merge']}")
    if commit is None or [parent["sha"] for parent in commit["parents"]] != [
        tested["base"],
        head_sha,
    ]:
        raise ValueError(
            "The tested revision is not a merge of the requested PR head and base."
        )
    baseline = baseline_run(repository, pull["base"]["ref"], tested["base"])
    body = "## uv test inventory changes\n\nA comparable test inventory is unavailable for this revision."
    changed = False
    if baseline is not None:
        baseline_artifacts = artifacts(repository, baseline["id"])
        shared = sorted(
            name
            for name in current_artifacts.keys() & baseline_artifacts.keys()
            if name.startswith("junit-results-")
        )
        if shared:
            body, changed = render(
                read_inventory(
                    repository, [baseline_artifacts[name] for name in shared]
                ),
                read_inventory(
                    repository, [current_artifacts[name] for name in shared]
                ),
                pull["base"]["ref"],
            )
            body += f"\n\n[Baseline CI](https://github.com/{repository}/actions/runs/{baseline['id']}) · `{tested['base'][:7]}`"
    return {
        "pull_request": pull["number"],
        "merge_sha": tested["merge"],
        "changed": changed,
        "body": body,
    }


def comment_id(value: str) -> int:
    if not re.fullmatch(r"[1-9][0-9]*", value) or len(value) > 16:
        raise ValueError("The stored comment ID is invalid.")
    return int(value)


def publish(
    repository: str,
    run_id: int,
    attempt: int,
    head_sha: str,
    report: dict,
    stored_id: int | None = None,
) -> int | None:
    run = current_run(repository, run_id, attempt, head_sha)
    if run is None or (pull := matching_pull(repository, run)) is None:
        return None
    if (
        pull["number"] != report["pull_request"]
        or pull["merge_commit_sha"] != report["merge_sha"]
    ):
        print(
            "The PR changed while the test inventory was being compared.",
            file=sys.stderr,
        )
        return None
    existing = api(repository, f"issues/comments/{stored_id}") if stored_id else None
    if existing:
        lines = existing["body"].splitlines()
        if (
            existing["user"]["id"] != BOT_ID
            or existing["issue_url"]
            != f"https://api.github.com/repos/{repository}/issues/{pull['number']}"
            or not lines
            or lines[0] != MARKER
            or len(lines) < 2
            or not (
                match := re.fullmatch(
                    r"<!-- report-run:([1-9][0-9]*):([1-9][0-9]*) -->", lines[1]
                )
            )
        ):
            raise ValueError(
                "The stored ID does not identify this PR's test inventory report."
            )
        if tuple(map(int, match.groups())) > (run_id, attempt):
            print("A newer run already published this report.", file=sys.stderr)
            return existing["id"]
    elif not report["changed"]:
        return None
    body = (
        f"{MARKER}\n<!-- report-run:{run_id}:{attempt} -->\n\n"
        f"{report['body']}\n\n"
        f"[Workflow run](https://github.com/{repository}/actions/runs/{run_id}/attempts/{attempt}) · `{head_sha[:7]}`\n"
    )
    if existing:
        if existing["body"] != body:
            api(
                repository,
                f"issues/comments/{existing['id']}",
                method="PATCH",
                body=body,
            )
        return existing["id"]
    return api(
        repository, f"issues/{pull['number']}/comments", method="POST", body=body
    )["id"]


def output(name: str, value: str) -> None:
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as file:
        file.write(f"{name}={value}\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "publish"])
    parser.add_argument("--run-id", type=int, required=True)
    parser.add_argument("--run-attempt", type=int, required=True)
    parser.add_argument("--head-sha", required=True)
    args = parser.parse_args()
    repository = os.environ["GITHUB_REPOSITORY"]
    if (
        repository not in REPOSITORIES
        or args.run_id <= 0
        or args.run_attempt <= 0
        or not re.fullmatch("[0-9a-f]{40}", args.head_sha)
    ):
        parser.error("Invalid repository, run ID, attempt, or head SHA.")
    if args.command == "prepare":
        if report := prepare(repository, args.run_id, args.run_attempt, args.head_sha):
            output("pull-request", str(report["pull_request"]))
            output("report", json.dumps(report))
    else:
        found = os.environ["STORED_COMMENT_FOUND"]
        if found not in {"true", "false"}:
            raise ValueError("The key-value lookup did not return a result.")
        stored_id = (
            comment_id(os.environ["STORED_COMMENT_ID"]) if found == "true" else None
        )
        published = publish(
            repository,
            args.run_id,
            args.run_attempt,
            args.head_sha,
            json.loads(os.environ["REPORT"]),
            stored_id,
        )
        if published is not None:
            output("comment-id", str(published))


if __name__ == "__main__":
    main()
