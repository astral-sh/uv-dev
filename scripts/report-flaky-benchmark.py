# /// script
# requires-python = ">=3.10"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Collect a CodSpeed report and file actionable benchmark flakes in uv-dev."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

REPOSITORY = "astral-sh/uv-dev"
REPOSITORY_ID = 1302176231
CODSPEED_APP_ID = 257293
CODSPEED_BOT_ID = 117304815
REPORT_MARKER = "<!-- __CODSPEED_PERFORMANCE_REPORT_COMMENT__ -->"
BENCHMARK_LINK = re.compile(r"\[(.*?)\]\((https://app\.codspeed\.io/[^\s)]+)\)")
COMPARISON = re.compile(
    r"Comparing <code>[^<]+</code> \(([0-9a-f]{7,40})\) "
    r"with <code>[^<]+</code> \(([0-9a-f]{7,40})\)"
)


def gh(*arguments: str, payload: dict | None = None):
    result = subprocess.run(
        ["gh", *arguments],
        input=json.dumps(payload) if payload is not None else None,
        text=True,
        check=True,
        capture_output=True,
    )
    return json.loads(result.stdout)


def candidates(body: str) -> list[dict]:
    """Keep only performance-table rows with a canonical benchmark identity."""
    result = {}
    for line in body.splitlines():
        if not line.startswith("|"):
            continue
        for match in BENCHMARK_LINK.finditer(line):
            name, url = match.groups()
            parsed = urlsplit(url)
            if parsed.netloc != "app.codspeed.io" or not parsed.path.startswith(
                f"/{REPOSITORY}/branches/"
            ):
                continue
            query = parse_qs(parsed.query)
            uri = query.get("uri", [])
            mode = query.get("runnerMode", [])
            if len(uri) != 1 or mode not in (["Simulation"], ["Walltime"]):
                continue
            result[uri[0], mode[0]] = {
                "uri": uri[0],
                "mode": mode[0],
                "name": name.strip("` "),
                "url": url,
                "row": line,
            }
    return list(result.values())


def validate_inputs(pull_request: str, head_sha: str, comment_id: str) -> None:
    if not re.fullmatch(r"[1-9][0-9]*", pull_request):
        raise ValueError("Invalid pull request number")
    if not re.fullmatch(r"[0-9a-f]{40}", head_sha):
        raise ValueError("Invalid pull request head SHA")
    if not re.fullmatch(r"[1-9][0-9]*", comment_id):
        raise ValueError("Invalid issue comment ID")


def current_report(
    pull_request: str, head_sha: str, comment_id: str, updated_at: str
) -> tuple[dict, dict] | None:
    validate_inputs(pull_request, head_sha, comment_id)
    comment = gh("api", f"repos/{REPOSITORY}/issues/comments/{comment_id}")
    if (
        comment.get("issue_url")
        != f"https://api.github.com/repos/{REPOSITORY}/issues/{pull_request}"
        or comment.get("user", {}).get("id") != CODSPEED_BOT_ID
        or comment.get("user", {}).get("type") != "Bot"
        or (comment.get("performed_via_github_app") or {}).get("id") != CODSPEED_APP_ID
        or comment.get("updated_at") != updated_at
        or not comment.get("body", "").startswith(REPORT_MARKER)
    ):
        return None
    pull = gh("api", f"repos/{REPOSITORY}/pulls/{pull_request}")
    for side in ("base", "head"):
        repository = pull.get(side, {}).get("repo") or {}
        if (
            repository.get("id") != REPOSITORY_ID
            or repository.get("full_name") != REPOSITORY
        ):
            return None
    comparison = COMPARISON.search(comment["body"])
    if (
        pull.get("state") != "open"
        or pull.get("head", {}).get("sha") != head_sha
        or comparison is None
        or not head_sha.startswith(comparison[1])
    ):
        return None
    return comment, pull


def collect(args: argparse.Namespace) -> None:
    report = current_report(
        args.pull_request, args.head_sha, args.comment_id, args.updated_at
    )
    if report is None:
        print(
            "The CodSpeed report is stale or does not belong to a trusted PR.",
            file=sys.stderr,
        )
        return
    comment, pull = report
    benchmarks = candidates(comment["body"])
    if not benchmarks:
        print("The CodSpeed report contains no performance changes.", file=sys.stderr)
        return
    comparison = COMPARISON.search(comment["body"])
    assert comparison is not None
    base = gh("api", f"repos/{REPOSITORY}/commits/{comparison[2]}")["sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", base) or not base.startswith(comparison[2]):
        raise ValueError("Invalid CodSpeed comparison base")
    diff = gh("api", f"repos/{REPOSITORY}/compare/{base}...{args.head_sha}")
    context = {
        "repository": REPOSITORY,
        "pull_request": {
            "number": pull["number"],
            "title": pull["title"],
            "body": pull["body"],
            "url": pull["html_url"],
        },
        "head_sha": args.head_sha,
        "base_sha": base,
        "comment": {
            "id": comment["id"],
            "updated_at": comment["updated_at"],
            "url": comment["html_url"],
            "body": comment["body"],
        },
        "benchmarks": benchmarks,
        # GitHub limits this response to 300 changed files; the agent can fetch
        # additional source at the recorded revisions when necessary.
        "comparison": {key: diff[key] for key in ("html_url", "status", "files")},
    }
    args.context.write_text(json.dumps(context, indent=2) + "\n")
    print("ready=true")


def benchmark_marker(uri: str, mode: str) -> str:
    identity = json.dumps([uri, mode], separators=(",", ":")).encode()
    return f"<!-- uv-benchmark-flake:{hashlib.sha256(identity).hexdigest()} -->"


def issue_payloads(context: dict, diagnosis: dict) -> list[dict]:
    allowed = {(item["uri"], item["mode"]): item for item in context["benchmarks"]}
    seen = set()
    result = []
    for finding in diagnosis["findings"]:
        identity = finding["uri"], finding["mode"]
        if identity not in allowed or identity in seen:
            raise ValueError("Diagnosis contains an unknown or repeated benchmark")
        seen.add(identity)
        decision = finding["decision"]
        if decision in ("ignore", "duplicate"):
            continue
        if decision != "create":
            raise ValueError("Invalid benchmark diagnosis decision")
        title, body = finding["issue"]["title"], finding["issue"]["body"]
        if (
            not isinstance(title, str)
            or not 1 <= len(title.strip()) <= 256
            or "\n" in title
            or "\r" in title
            or not isinstance(body, str)
            or not body.strip()
            or len(body) > 50000
            or re.search(r"@[A-Za-z0-9][A-Za-z0-9-]*", title + body)
        ):
            raise ValueError("Invalid benchmark issue text")
        benchmark = allowed[identity]
        marker = benchmark_marker(*identity)
        source = (
            f"\n\nCodSpeed report: {context['comment']['url']}\n"
            f"Benchmark: {benchmark['url']}\n"
            f"Comparison: `{context['base_sha']}` → `{context['head_sha']}`\n\n{marker}\n"
        )
        result.append(
            {
                "title": title.strip(),
                "body": body.rstrip() + source,
                "labels": ["area:benchmarks", "internal:ci-flake"],
            }
        )
    return result


def report(args: argparse.Namespace) -> None:
    context = json.loads(args.context.read_text())
    diagnosis = json.loads(args.diagnosis.read_text())
    payloads = issue_payloads(context, diagnosis)
    if not payloads:
        return
    current = current_report(
        str(context["pull_request"]["number"]),
        context["head_sha"],
        str(context["comment"]["id"]),
        context["comment"]["updated_at"],
    )
    if current is None or current[0]["body"] != context["comment"]["body"]:
        print("The CodSpeed report changed during diagnosis; skipping.")
        return
    pages = gh(
        "api",
        "--paginate",
        "--slurp",
        f"repos/{REPOSITORY}/issues?state=all&labels=area%3Abenchmarks&per_page=100",
    )
    existing = [
        issue for page in pages for issue in page if "pull_request" not in issue
    ]
    for payload in payloads:
        marker = payload["body"].splitlines()[-1]
        duplicate = next(
            (
                issue
                for issue in existing
                if marker in (issue.get("body") or "")
                or issue["title"] == payload["title"]
            ),
            None,
        )
        if duplicate is None:
            duplicate = gh(
                "api",
                "--method",
                "POST",
                f"repos/{REPOSITORY}/issues",
                "--input",
                "-",
                payload=payload,
            )
            existing.append(duplicate)
        print(duplicate["html_url"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    collect_parser = commands.add_parser("collect")
    for argument in ("pull-request", "head-sha", "comment-id", "updated-at"):
        collect_parser.add_argument(f"--{argument}", required=True)
    collect_parser.add_argument("--context", type=Path, required=True)
    report_parser = commands.add_parser("report")
    report_parser.add_argument("--context", type=Path, required=True)
    report_parser.add_argument("--diagnosis", type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get("GITHUB_REPOSITORY", REPOSITORY) != REPOSITORY:
        raise ValueError("This workflow only reports uv-dev benchmarks")
    if args.command == "collect":
        collect(args)
    else:
        report(args)


if __name__ == "__main__":
    main()
