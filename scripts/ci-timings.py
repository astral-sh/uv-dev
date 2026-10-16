"""Compare uv-dev PR CI timings without checking out or running PR code."""

import argparse
import hashlib
import html
import json
import os
import re
import statistics
import subprocess
import sys
from collections import Counter
from datetime import datetime, timedelta
from pathlib import Path
from urllib.parse import urlencode

REPOSITORY = "astral-sh/uv-dev"
REPOSITORY_ID = 1302176231
WORKFLOW = ".github/workflows/ci.yml"
BOT_ID = 305554984
MARKER = "<!-- uv-ci-timings:v1 -->"
STATE_PATTERN = re.compile(r"<!-- uv-ci-timings-state: (.*?) -->")
BASELINE_RUNS = 20
MIN_SAMPLES = 5
LOOKBACK_DAYS = 14
MIN_SECONDS = 60
MIN_FRACTION = 0.25
MAD_MULTIPLIER = 3 * 1.4826


class GitHub:
    def request(self, path, *, body=None, method="GET"):
        command = ["gh", "api", "--method", method, f"repos/{REPOSITORY}/{path}"]
        environment = os.environ.copy()
        if method != "GET":
            environment["GH_TOKEN"] = environment["COMMENT_TOKEN"]
            command.extend(["--input", "-"])
        result = subprocess.run(
            command,
            input=json.dumps(body) if body is not None else None,
            text=True,
            capture_output=True,
            check=False,
            env=environment,
        )
        if result.returncode:
            raise RuntimeError(f"GitHub {method} request failed: {path}")
        return json.loads(result.stdout)

    def items(self, path, key=None, *, pages=10):
        separator = "&" if "?" in path else "?"
        for page in range(1, pages + 1):
            result = self.request(f"{path}{separator}per_page=100&page={page}")
            values = result[key] if key else result
            yield from values
            if len(values) < 100:
                return
        raise RuntimeError(f"GitHub pagination limit exceeded: {path}")


def timestamp(value):
    return datetime.fromisoformat(value)


def trusted_run(run):
    return (
        run.get("repository", {}).get("id") == REPOSITORY_ID
        and (run.get("head_repository") or {}).get("id") == REPOSITORY_ID
        and run.get("path") == WORKFLOW
        and run.get("event") == "pull_request"
    )


def source_context(
    api, run_id, head_sha, attempt, *, historical=False, pull_number=None
):
    run = api.request(f"actions/runs/{run_id}")
    if not trusted_run(run) or run["head_sha"] != head_sha:
        raise ValueError("Expected uv-dev PR CI at the requested commit")
    if (
        run["status"] != "completed"
        or run["conclusion"] not in {"success", "failure", "timed_out"}
        or run["run_attempt"] != attempt
    ):
        return None
    references = run["pull_requests"]
    if pull_number is not None:
        if not historical:
            raise ValueError(
                "An explicit PR number is only supported for historical analysis"
            )
        references = [api.request(f"pulls/{pull_number}")]
    if not references:
        references = list(api.items(f"commits/{head_sha}/pulls"))
    if historical and not any(
        reference.get("base", {}).get("repo", {}).get("id") == REPOSITORY_ID
        for reference in references
    ):
        query = urlencode({"state": "all", "head": f"astral-sh:{run['head_branch']}"})
        references = list(api.items(f"pulls?{query}"))
    candidates = []
    for reference in references:
        # Commit associations can include the upstream PR after promotion.
        if reference.get("base", {}).get("repo", {}).get("id") != REPOSITORY_ID:
            continue
        pull = api.request(f"pulls/{reference['number']}")
        if (
            pull["base"]["repo"]["id"] == REPOSITORY_ID
            and (pull["head"]["repo"] or {}).get("id") == REPOSITORY_ID
            and pull["head"]["ref"] == run["head_branch"]
            and (historical or pull["head"]["sha"] == head_sha)
            and (historical or pull["state"] == "open")
            and (historical or pull["base"]["ref"] == reference["base"]["ref"])
        ):
            candidates.append(pull)
    if len(candidates) != 1:
        return None
    return run, candidates[0]


def job_key(job):
    return job["name"], tuple(sorted(job["labels"]))


def key_id(key):
    return hashlib.sha256(json.dumps(key).encode()).hexdigest()[:20]


def measure_job(job, *, before=None):
    if job["status"] != "completed" or job["conclusion"] != "success":
        return None
    if not job["labels"] or not job["steps"]:
        return None
    steps = {}
    occurrences = Counter()
    for step in job["steps"]:
        name = step["name"]
        occurrences[name] += 1
        if step["conclusion"] == "skipped":
            continue
        if (
            step["status"] != "completed"
            or step["conclusion"] != "success"
            or not step["started_at"]
            or not step["completed_at"]
        ):
            return None
        started = timestamp(step["started_at"])
        completed = timestamp(step["completed_at"])
        if completed < started or (before and completed >= before):
            return None
        steps[(name, occurrences[name])] = (completed - started).total_seconds()
    seconds = sum(steps.values())
    if seconds <= 0:
        return None
    return {"id": job["id"], "seconds": seconds, "steps": steps}


def measurements(jobs, *, before=None):
    # Jobs with the same display name and runner labels cannot be matched unambiguously.
    counts = Counter(job_key(job) for job in jobs)
    result = {}
    for job in jobs:
        key = job_key(job)
        if counts[key] == 1 and (measurement := measure_job(job, before=before)):
            result[key] = measurement
    return result


def jobs_for_attempt(api, run_id, attempt):
    return list(api.items(f"actions/runs/{run_id}/attempts/{attempt}/jobs", "jobs"))


def baseline_runs(api, source, pull_number):
    cutoff = timestamp(source["created_at"])
    oldest = cutoff - timedelta(days=LOOKBACK_DAYS)
    query = urlencode(
        {
            "event": "pull_request",
            "status": "completed",
            "created": f"{oldest.isoformat()}..{cutoff.isoformat()}",
        }
    )
    branches = {source["head_branch"]}
    commits = {source["head_sha"]}
    selected = []
    for run in api.items(
        f"actions/workflows/{source['workflow_id']}/runs?{query}", "workflow_runs"
    ):
        if (
            not trusted_run(run)
            or run["conclusion"] not in {"success", "failure", "timed_out"}
            or not oldest <= timestamp(run["created_at"]) < cutoff
            or run["head_branch"] in branches
            or run["head_sha"] in commits
            or any(pull["number"] == pull_number for pull in run["pull_requests"])
        ):
            continue
        # Use only the first attempt, even when the run was subsequently retried.
        measured = measurements(jobs_for_attempt(api, run["id"], 1), before=cutoff)
        if not measured:
            continue
        branches.add(run["head_branch"])
        commits.add(run["head_sha"])
        selected.append((run, measured))
        if len(selected) == BASELINE_RUNS:
            break
    return selected


def comparison(seconds, samples):
    if len(samples) < MIN_SAMPLES:
        return None
    median = statistics.median(samples)
    deviation = statistics.median(abs(value - median) for value in samples)
    threshold = max(MIN_SECONDS, median * MIN_FRACTION, deviation * MAD_MULTIPLIER)
    return {
        "median": median,
        "mad": deviation,
        "threshold": threshold,
        "increase": seconds - median,
        "regressed": seconds - median >= threshold,
    }


def compare(source, pull, jobs, baselines, *, historical=False):
    measured = measurements(jobs)
    results = []
    for key, current in sorted(measured.items()):
        samples = [
            {
                "run_id": run["id"],
                "job_id": values[key]["id"],
                "seconds": values[key]["seconds"],
            }
            for run, values in baselines
            if key in values
        ]
        result = {
            "key": key_id(key),
            "name": key[0],
            "labels": key[1],
            "job_id": current["id"],
            "seconds": current["seconds"],
            "samples": samples,
            "comparison": comparison(
                current["seconds"], [sample["seconds"] for sample in samples]
            ),
            "steps": [],
        }
        for step, seconds in current["steps"].items():
            step_samples = [
                values[key]["steps"][step]
                for _, values in baselines
                if key in values and step in values[key]["steps"]
            ]
            if len(step_samples) >= MIN_SAMPLES:
                median = statistics.median(step_samples)
                if seconds - median >= 15:
                    result["steps"].append(
                        {
                            "name": step[0],
                            "occurrence": step[1],
                            "seconds": seconds,
                            "median": median,
                        }
                    )
        result["steps"].sort(
            key=lambda step: step["seconds"] - step["median"], reverse=True
        )
        results.append(result)
    return {
        "version": 1,
        "repository": REPOSITORY,
        "run_id": source["id"],
        "attempt": source["run_attempt"],
        "head_sha": source["head_sha"],
        "pull_request": pull["number"],
        "base_ref": pull["base"]["ref"],
        "historical": historical,
        "baseline_runs": [run["id"] for run, _ in baselines],
        "settings": {
            "baseline_runs": BASELINE_RUNS,
            "min_samples": MIN_SAMPLES,
            "lookback_days": LOOKBACK_DAYS,
            "min_seconds": MIN_SECONDS,
            "min_fraction": MIN_FRACTION,
            "mad_multiplier": MAD_MULTIPLIER,
        },
        "jobs": results,
        "unmeasured_jobs": len(jobs) - len(measured),
    }


def regressions(report):
    return sorted(
        [
            job
            for job in report["jobs"]
            if job["comparison"] and job["comparison"]["regressed"]
        ],
        key=lambda job: job["comparison"]["increase"],
        reverse=True,
    )


def inline(value):
    # Job and step names come from PR-controlled workflows. Keep them inert in Markdown.
    value = " ".join(value.split())[:180]
    escaped = "".join(
        f"&#{ord(character)};"
        if character in "@|[]()`*_~\\"
        else html.escape(character)
        for character in value
    )
    return f"<code>{escaped}</code>"


def duration(seconds):
    minutes, remainder = divmod(round(seconds), 60)
    return f"{minutes}m {remainder:02d}s" if minutes else f"{remainder}s"


def render(report, retained=()):
    findings = regressions(report)
    comparable = [job for job in report["jobs"] if job["comparison"]]
    run_url = f"https://github.com/{REPOSITORY}/actions/runs/{report['run_id']}/attempts/{report['attempt']}"
    state = {
        "run_id": report["run_id"],
        "attempt": report["attempt"],
        "head_sha": report["head_sha"],
        "warnings": [
            {
                "key": job["key"],
                "name": job["name"],
                "run_id": report["run_id"],
                "job_id": job["job_id"],
                "seconds": job["seconds"],
                "median": job["comparison"]["median"],
            }
            for job in findings
        ]
        + list(retained),
    }
    encoded_state = (
        json.dumps(state, separators=(",", ":"))
        .replace("<", "\\u003c")
        .replace(">", "\\u003e")
        .replace("@", "\\u0040")
    )
    lines = [
        MARKER,
        f"<!-- uv-ci-timings-state: {encoded_state} -->",
        "### Potential CI time regressions"
        if findings or retained
        else "### CI timing update",
        "",
        f"[CI run]({run_url}) for `{report['head_sha'][:12]}` (attempt {report['attempt']}).",
        "",
    ]
    if findings:
        lines.extend(
            [
                "These jobs took longer than the median for matching jobs in recent CI runs from other `uv-dev` PRs:",
                "",
                "| Job | This run | Baseline median | Increase | Samples |",
                "| --- | ---: | ---: | ---: | ---: |",
            ]
        )
        for job in findings[:10]:
            comparison = job["comparison"]
            percent = comparison["increase"] / comparison["median"] * 100
            url = f"https://github.com/{REPOSITORY}/actions/runs/{report['run_id']}/job/{job['job_id']}"
            lines.append(
                f"| [{inline(job['name'])}]({url}) | {duration(job['seconds'])} "
                f"| {duration(comparison['median'])} | +{duration(comparison['increase'])} "
                f"(+{percent:.0f}%) | {len(job['samples'])} |"
            )
        if len(findings) > 10:
            lines.extend(
                [
                    "",
                    f"{len(findings) - 10} additional jobs met the timing thresholds for a warning; see the timing artifact.",
                ]
            )
        details = [(job, step) for job in findings[:10] for step in job["steps"][:3]]
        if details:
            lines.extend(
                [
                    "",
                    "<details>",
                    "<summary>Largest step increases within these jobs</summary>",
                    "",
                ]
            )
            for job, step in details:
                lines.append(
                    f"- {inline(job['name'])} → {inline(step['name'])}: "
                    f"{duration(step['median'])} → {duration(step['seconds'])}."
                )
            lines.extend(["", "</details>"])
        lines.extend(
            [
                "",
                "This is a timing signal, not proof that the PR caused the slowdown. Cache misses and runner load can also increase runtime.",
            ]
        )
    elif comparable and not retained:
        lines.append(
            "No jobs with comparable samples met the timing thresholds for a warning."
        )
    elif not retained:
        lines.append("No jobs could be compared in this attempt.")
    if retained:
        lines.extend(
            [
                "",
                "Earlier warnings at this commit were not reassessed because this attempt lacks comparable results for these jobs:",
                "",
            ]
        )
        for warning in retained[:10]:
            url = f"https://github.com/{REPOSITORY}/actions/runs/{warning['run_id']}/job/{warning['job_id']}"
            lines.append(
                f"- [{inline(warning['name'])}]({url}): {duration(warning['seconds'])} against a {duration(warning['median'])} median."
            )
        if len(retained) > 10:
            lines.append(
                f"- {len(retained) - 10} additional earlier warnings remain unassessed."
            )
    lines.extend(
        [
            "",
            (
                f"Compared {len(comparable)} jobs; {len(report['jobs']) - len(comparable)} successful jobs "
                f"had fewer than {MIN_SAMPLES} matching samples. {report['unmeasured_jobs']} jobs were "
                "skipped, unsuccessful, or lacked unambiguous timing data."
            ),
            "",
            (
                f"Baseline: up to {BASELINE_RUNS} CI runs created in the preceding {LOOKBACK_DAYS} days "
                "from different PR branches, using their first attempt and matching job names and runner labels. "
                "Only successful jobs whose measured steps finished before this run was created contribute samples. "
                "Runtime is the sum of step durations, excluding queue and dependency waits. "
                f"A warning requires at least {MIN_SAMPLES} samples and an increase of at least "
                f"{MIN_SECONDS} seconds, {MIN_FRACTION:.0%}, and three scaled median absolute deviations."
            ),
        ]
    )
    return "\n".join(lines) + "\n"


def current_report(api, report):
    context = source_context(
        api, report["run_id"], report["head_sha"], report["attempt"]
    )
    if context is None:
        return False
    source, pull = context
    if (
        pull["number"] != report["pull_request"]
        or pull["base"]["ref"] != report["base_ref"]
    ):
        return False
    query = urlencode({"event": "pull_request", "head_sha": report["head_sha"]})
    for run in api.items(
        f"actions/workflows/{source['workflow_id']}/runs?{query}", "workflow_runs"
    ):
        if (
            trusted_run(run)
            and run["head_branch"] == source["head_branch"]
            and run["id"] > source["id"]
            and run["conclusion"] != "cancelled"
        ):
            return False
    return True


def publish(api, report):
    if (
        os.environ.get("GITHUB_REPOSITORY") != REPOSITORY
        or os.environ.get("GITHUB_REF") != "refs/heads/main"
        or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
        or report["repository"] != REPOSITORY
        or report["historical"]
    ):
        raise ValueError("Publishing requires a uv-dev main-branch workflow dispatch")
    comments = list(api.items(f"issues/{report['pull_request']}/comments"))
    own = [
        comment
        for comment in comments
        if comment["user"]["id"] == BOT_ID
        and (comment.get("performed_via_github_app") or {}).get("slug")
        == "astral-automations-bot"
        and comment["body"].startswith(MARKER)
    ]
    existing = max(own, key=lambda comment: comment["id"]) if own else None
    findings = regressions(report)
    if not findings and not existing:
        return "No timing warnings; no comment needed."
    retained = []
    if existing:
        previous = STATE_PATTERN.search(existing["body"])
        if previous:
            state = json.loads(previous[1])
            compared = {job["key"] for job in report["jobs"] if job["comparison"]}
            if state["head_sha"] == report["head_sha"]:
                retained = [
                    warning
                    for warning in state["warnings"]
                    if warning["key"] not in compared
                ]
    body = render(report, retained)
    if existing and existing["body"] == body:
        return "Timing report is already up to date."
    # Recheck after collection, immediately before the comment mutation.
    if not current_report(api, report):
        return "The PR or CI run changed; skipping a stale report."
    if existing:
        api.request(
            f"issues/comments/{existing['id']}", method="PATCH", body={"body": body}
        )
        return "Updated the CI timing comment."
    api.request(
        f"issues/{report['pull_request']}/comments", method="POST", body={"body": body}
    )
    return "Created the CI timing comment."


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    for name in ("context", "analyze"):
        command = subcommands.add_parser(name)
        command.add_argument("--run-id", type=int, required=True)
        command.add_argument("--head-sha", required=True)
        command.add_argument("--attempt", type=int, required=True)
        if name == "analyze":
            command.add_argument("--output", type=Path, required=True)
            command.add_argument(
                "--pull-request",
                type=int,
                help="PR number for historical runs whose branch was deleted",
            )
            command.add_argument(
                "--historical",
                action="store_true",
                help="Read-only analysis of a closed or superseded PR run",
            )
    command = subcommands.add_parser("publish")
    command.add_argument("report", type=Path)
    arguments = parser.parse_args()
    api = GitHub()
    if arguments.command == "publish":
        print(publish(api, json.loads(arguments.report.read_text())))
        return
    if (
        arguments.run_id <= 0
        or arguments.attempt <= 0
        or not re.fullmatch(r"[0-9a-f]{40}", arguments.head_sha)
    ):
        parser.error("Expected a positive run ID and attempt, and a full commit SHA")
    historical = getattr(arguments, "historical", False)
    context = source_context(
        api,
        arguments.run_id,
        arguments.head_sha,
        arguments.attempt,
        historical=historical,
        pull_number=getattr(arguments, "pull_request", None),
    )
    if context is None:
        print("The run is incomplete, stale, or has no matching PR.", file=sys.stderr)
        return
    source, pull = context
    if arguments.command == "context":
        print(f"pull-request={pull['number']}")
        return
    jobs = jobs_for_attempt(api, source["id"], source["run_attempt"])
    baselines = baseline_runs(api, source, pull["number"])
    report = compare(source, pull, jobs, baselines, historical=historical)
    arguments.output.mkdir(parents=True, exist_ok=True)
    (arguments.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    (arguments.output / "comment.md").write_text(render(report))
    print(render(report))


if __name__ == "__main__":
    main()
