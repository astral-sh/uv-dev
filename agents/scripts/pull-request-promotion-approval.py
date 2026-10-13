#!/usr/bin/env python3
"""Bind queued promotions to approvals captured by an explicit dispatch."""

import argparse
import datetime
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import urllib.parse
import zipfile

WORKFLOW_PATH = ".github/workflows/promote-pull-request.yml"
CAPTURE_JOB = "Record promotion approval"
LABELS = {"bot:promote", "bot:automerge"}
REPOSITORIES = {"astral-sh/uv-dev", "astral-sh/uv-security"}
MAX_ARTIFACT_BYTES = 64 * 1024
MAX_RECORD_BYTES = 16 * 1024


class ApprovalError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise ApprovalError(message)


def positive_integer(value):
    return type(value) is int and value > 0


def timestamp(value):
    require(isinstance(value, str), "Approval timestamp is missing.")
    try:
        result = datetime.datetime.fromisoformat(value)
    except ValueError as error:
        raise ApprovalError("Approval timestamp is invalid.") from error
    require(result.tzinfo is not None, "Approval timestamp has no timezone.")
    return result


def gh_json(endpoint, *, paginate=False):
    command = ["gh", "api", endpoint]
    if paginate:
        command.extend(["--paginate", "--slurp"])
    return json.loads(subprocess.check_output(command))


def pages(endpoint):
    value = gh_json(endpoint, paginate=True)
    require(isinstance(value, list), "GitHub returned invalid paginated data.")
    return value


def current_approval(repository, number):
    events = [
        event
        for page in pages(f"repos/{repository}/issues/{number}/events?per_page=100")
        for event in page
    ]
    relevant = [
        event
        for event in events
        if (
            event.get("event") == "labeled"
            and event.get("label", {}).get("name") in LABELS
        )
        or (
            event.get("event") == "ready_for_review"
            and repository == "astral-sh/uv-dev"
        )
    ]
    if not relevant:
        return None
    # The latest approving event selects the mode. Removing that label revokes its approval,
    # rather than reviving an older ready event. Other label cleanup does not change this mode.
    # Draft eligibility is checked separately, including during recovery's own draft transition.
    event = max(
        relevant, key=lambda item: (timestamp(item.get("created_at")), item["id"])
    )
    if (
        event.get("event") not in {"ready_for_review", "labeled"}
        or event.get("actor", {}).get("type") != "User"
    ):
        return None
    mode = event["label"]["name"] if event["event"] == "labeled" else "bot:promote"
    if repository == "astral-sh/uv-security" and mode != "bot:promote":
        return None
    if event["event"] == "labeled":
        changes = [
            change
            for change in events
            if change.get("event") in {"labeled", "unlabeled"}
            and change.get("label", {}).get("name") == mode
        ]
        latest = max(
            changes, key=lambda item: (timestamp(item.get("created_at")), item["id"])
        )
        if latest["id"] != event["id"]:
            return None
    actor = event["actor"]
    require(
        positive_integer(event.get("id")) and positive_integer(actor.get("id")),
        "Approval identity is invalid.",
    )
    require(
        isinstance(actor.get("login"), str) and actor["login"],
        "Approval actor is missing.",
    )
    return {
        "id": event["id"],
        "event": event["event"],
        "actor": {"id": actor["id"], "login": actor["login"], "type": "User"},
        "created_at": event["created_at"],
        "promotion_label": mode,
    }


def eligible_source(repository, repository_id, number, approval, expected_head):
    source = gh_json(f"repos/{repository}/pulls/{number}")
    require(source.get("state") == "open", "The source pull request is no longer open.")
    require(
        not source.get("draft") or approval["event"] == "labeled",
        "The source is no longer approved for promotion.",
    )
    if approval["event"] == "labeled":
        require(
            any(
                label.get("name") == approval["promotion_label"]
                for label in source.get("labels", [])
            ),
            "The approving label was removed.",
        )
    for side in ("base", "head"):
        repo = source.get(side, {}).get("repo") or {}
        require(
            repo.get("id") == repository_id and repo.get("full_name") == repository,
            "The source changed repositories.",
        )
    require(
        source["head"]["sha"] == expected_head,
        "The source head changed; a fresh approval is required.",
    )
    require(
        not source["head"]["ref"].startswith("automations/promote/"),
        "Promotion branches cannot be promoted again.",
    )
    subprocess.run(
        ["git", "check-ref-format", "refs/heads/" + source["head"]["ref"]], check=True
    )
    return source


def artifact_name(repository_id, number, approval_id):
    return f"promotion-approval-{repository_id}-{number}-{approval_id}"


def same_json(left, right):
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def binding(record):
    return {key: value for key, value in record.items() if key != "run"}


def validate_record(record, repository, repository_id, number, approval):
    require(
        isinstance(record, dict)
        and set(record)
        == {
            "schema",
            "repository",
            "number",
            "head_ref",
            "head_sha",
            "approval",
            "run",
        },
        "Approval record schema is invalid.",
    )
    require(
        type(record["schema"]) is int and record["schema"] == 1,
        "Unsupported approval record version.",
    )
    require(
        same_json(record["repository"], {"id": repository_id, "full_name": repository})
        and type(record["number"]) is int
        and record["number"] == number,
        "Approval record belongs to another source.",
    )
    require(
        same_json(record["approval"], approval),
        "Approval record no longer matches the current human approval.",
    )
    require(
        isinstance(record["head_sha"], str)
        and re.fullmatch(r"[0-9a-f]{40}", record["head_sha"]),
        "Approval revision is invalid.",
    )
    require(isinstance(record["head_ref"], str), "Approval branch is invalid.")
    subprocess.run(
        ["git", "check-ref-format", "refs/heads/" + record["head_ref"]], check=True
    )
    run = record["run"]
    require(
        isinstance(run, dict)
        and set(run) == {"id", "attempt", "workflow_id", "head_sha"},
        "Approval run identity is invalid.",
    )
    require(
        all(positive_integer(run[key]) for key in ("id", "attempt", "workflow_id")),
        "Approval run identifiers are invalid.",
    )
    require(
        isinstance(run["head_sha"], str)
        and re.fullmatch(r"[0-9a-f]{40}", run["head_sha"]),
        "Approval workflow revision is invalid.",
    )


def validate_run(run, repository, repository_id, workflow_id):
    require(
        run.get("workflow_id") == workflow_id
        and run.get("path", "").split("@", 1)[0] == WORKFLOW_PATH,
        "Approval artifact was not produced by the promotion workflow.",
    )
    require(
        run.get("event") == "workflow_dispatch" and run.get("head_branch") == "main",
        "Approval artifacts must originate from an explicit dispatch on main.",
    )
    for key in ("repository", "head_repository"):
        repo = run.get(key) or {}
        require(
            repo.get("id") == repository_id and repo.get("full_name") == repository,
            "Approval workflow changed repositories.",
        )


def read_artifact(artifact, repository, repository_id, workflow_id, number, approval):
    origin = artifact.get("workflow_run") or {}
    # Artifact names are public. PR and unrelated CI workflows cannot supply approval authority.
    if not (
        positive_integer(origin.get("id"))
        and origin.get("repository_id") == repository_id
        and origin.get("head_repository_id") == repository_id
        and origin.get("head_branch") == "main"
    ):
        return None
    run = gh_json(f"repos/{repository}/actions/runs/{origin['id']}")
    if not (
        run.get("workflow_id") == workflow_id
        and run.get("path", "").split("@", 1)[0] == WORKFLOW_PATH
        and run.get("event") == "workflow_dispatch"
        and run.get("head_branch") == "main"
        and all(
            (run.get(key) or {}).get("id") == repository_id
            and (run.get(key) or {}).get("full_name") == repository
            for key in ("repository", "head_repository")
        )
    ):
        return None
    require(
        run.get("id") == origin["id"] and origin.get("head_sha") == run.get("head_sha"),
        "Approval artifact origin is inconsistent.",
    )
    require(
        positive_integer(artifact.get("id")), "Approval artifact identity is invalid."
    )
    require(
        type(artifact.get("size_in_bytes")) is int
        and 0 < artifact["size_in_bytes"] <= MAX_ARTIFACT_BYTES,
        "Approval artifact is too large or empty.",
    )
    archive = subprocess.check_output(
        ["gh", "api", f"repos/{repository}/actions/artifacts/{artifact['id']}/zip"]
    )
    require(len(archive) <= MAX_ARTIFACT_BYTES, "Approval archive is too large.")
    require(
        artifact.get("digest") == "sha256:" + hashlib.sha256(archive).hexdigest(),
        "Approval artifact digest does not match.",
    )
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        entries = bundle.infolist()
        require(
            len(entries) == 1
            and entries[0].filename == "approval.json"
            and not entries[0].is_dir()
            and entries[0].file_size <= MAX_RECORD_BYTES,
            "Approval artifact must contain only approval.json.",
        )
        record = json.loads(bundle.read(entries[0]))
    validate_record(record, repository, repository_id, number, approval)
    require(
        record["run"]["id"] == run["id"]
        and record["run"]["workflow_id"] == workflow_id
        and record["run"]["head_sha"] == run["head_sha"]
        and record["run"]["attempt"] <= run["run_attempt"],
        "Approval record and workflow origin differ.",
    )
    jobs = [
        job
        for page in pages(
            f"repos/{repository}/actions/runs/{run['id']}/attempts/{record['run']['attempt']}/jobs?per_page=100"
        )
        for job in page["jobs"]
        if job.get("name") == CAPTURE_JOB
    ]
    require(
        len(jobs) == 1
        and jobs[0].get("status") == "completed"
        and jobs[0].get("conclusion") == "success"
        and jobs[0].get("head_sha") == run["head_sha"],
        "The approval capture job did not complete successfully.",
    )
    require(
        timestamp(jobs[0]["started_at"])
        <= timestamp(artifact["created_at"])
        <= timestamp(jobs[0]["completed_at"]),
        "Approval artifact was not created during its capture job.",
    )
    return record


def recorded_approval(repository, repository_id, workflow_id, number, approval):
    name = artifact_name(repository_id, number, approval["id"])
    query = urllib.parse.urlencode({"name": name, "per_page": 100})
    artifacts = [
        artifact
        for page in pages(f"repos/{repository}/actions/artifacts?{query}")
        for artifact in page["artifacts"]
    ]
    records = []
    for artifact in artifacts:
        require(
            artifact.get("name") == name,
            "GitHub returned an unrelated approval artifact.",
        )
        if artifact.get("expired") or timestamp(
            artifact["expires_at"]
        ) <= datetime.datetime.now(datetime.UTC):
            continue
        record = read_artifact(
            artifact, repository, repository_id, workflow_id, number, approval
        )
        if record is not None:
            records.append(record)
    if not records:
        return None
    require(
        all(binding(record) == binding(records[0]) for record in records),
        "Conflicting approval records require a fresh approval.",
    )
    return records[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("current", "capture", "load"))
    parser.add_argument("repository", choices=sorted(REPOSITORIES))
    parser.add_argument("number", type=int)
    parser.add_argument("--head-sha")
    parser.add_argument("--promotion-label", choices=sorted(LABELS))
    args = parser.parse_args()
    require(args.number > 0, "Invalid pull request number.")
    approval = current_approval(args.repository, args.number)
    if args.command == "current":
        print(json.dumps(approval, separators=(",", ":")))
        return
    require(
        approval is not None,
        "No current human promotion approval; a fresh approval is required.",
    )
    repository = gh_json(f"repos/{args.repository}")
    repository_id = repository["id"]
    require(
        positive_integer(repository_id)
        and repository.get("full_name") == args.repository,
        "Repository identity is invalid.",
    )
    workflow_id = gh_json(
        f"repos/{args.repository}/actions/workflows/promote-pull-request.yml"
    )["id"]
    require(positive_integer(workflow_id), "Workflow identity is invalid.")
    existing = recorded_approval(
        args.repository, repository_id, workflow_id, args.number, approval
    )
    if args.command == "load":
        require(
            existing is not None,
            "No durable approval record remains; a fresh explicit approval or dispatch is required.",
        )
        source = eligible_source(
            args.repository, repository_id, args.number, approval, existing["head_sha"]
        )
        require(
            source["head"]["ref"] == existing["head_ref"],
            "The approved source branch changed.",
        )
        print(json.dumps(existing, separators=(",", ":")))
        return
    require(
        args.head_sha is not None and re.fullmatch(r"[0-9a-f]{40}", args.head_sha),
        "An explicitly approved head SHA is required.",
    )
    require(
        args.promotion_label == approval["promotion_label"],
        "The dispatched promotion mode was superseded.",
    )
    require(
        os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch"
        and os.environ.get("GITHUB_REF") == "refs/heads/main"
        and os.environ.get("GITHUB_REPOSITORY") == args.repository
        and os.environ.get("GITHUB_REPOSITORY_ID") == str(repository_id),
        "Approval capture requires an explicit dispatch in the source repository on main.",
    )
    run_id = int(os.environ["GITHUB_RUN_ID"])
    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    require(run_id > 0 and attempt > 0, "Current workflow identity is invalid.")
    run = gh_json(f"repos/{args.repository}/actions/runs/{run_id}")
    validate_run(run, args.repository, repository_id, workflow_id)
    require(
        run.get("run_attempt") == attempt
        and run.get("head_sha") == os.environ.get("GITHUB_SHA"),
        "The current workflow attempt changed.",
    )
    require(
        timestamp(approval["created_at"]) <= timestamp(run["created_at"]),
        "A newer approval superseded this queued dispatch.",
    )
    source = eligible_source(
        args.repository, repository_id, args.number, approval, args.head_sha
    )
    record = {
        "schema": 1,
        "repository": {"id": repository_id, "full_name": args.repository},
        "number": args.number,
        "head_ref": source["head"]["ref"],
        "head_sha": args.head_sha,
        "approval": approval,
        "run": {
            "id": run_id,
            "attempt": attempt,
            "workflow_id": workflow_id,
            "head_sha": run["head_sha"],
        },
    }
    if existing is not None:
        require(
            binding(existing) == binding(record),
            "This approval already names another revision; a fresh approval is required.",
        )
        record = existing
    print(
        json.dumps(
            {
                "record": record,
                "artifact_name": artifact_name(
                    repository_id, args.number, approval["id"]
                ),
                "upload": existing is None,
            },
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    try:
        main()
    except (
        ApprovalError,
        KeyError,
        TypeError,
        ValueError,
        OSError,
        subprocess.CalledProcessError,
        zipfile.BadZipFile,
    ) as error:
        print(f"Promotion approval rejected: {error}", file=sys.stderr)
        raise SystemExit(1) from error
