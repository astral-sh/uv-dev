#!/usr/bin/env python3
"""Bind queued promotions to approvals captured by an explicit dispatch."""

import argparse
import datetime
import hashlib
import io
import itertools
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


class IncompleteCandidateUpload(ApprovalError):
    def __init__(self, record):
        super().__init__("The candidate upload did not complete successfully.")
        self.record = record


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


def eligible_source(
    repository,
    repository_id,
    number,
    approval,
    expected_head,
    expected_ref=None,
    expected_base=None,
):
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
        expected_ref is None or source["head"]["ref"] == expected_ref,
        "The approved source branch changed.",
    )
    require(
        expected_base is None or source["base"]["ref"] == expected_base,
        "The approved source base changed; a fresh approval is required.",
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
    return f"promotion-approval-v2-{repository_id}-{number}-{approval_id}"


def same_json(left, right):
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def binding(record):
    return {key: value for key, value in record.items() if key != "run"}


def validate_record(
    record, repository, repository_id, number, approval, *, candidate=False
):
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
        }
        | ({"candidate"} if candidate else {"base_ref"}),
        "Approval record schema is invalid.",
    )
    require(
        type(record["schema"]) is int and record["schema"] == 2,
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
    if not candidate:
        require(isinstance(record["base_ref"], str), "Approved source base is invalid.")
        subprocess.run(
            ["git", "check-ref-format", "refs/heads/" + record["base_ref"]], check=True
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


def validate_run(run, repository, repository_id, workflow_id, *, candidate=False):
    require(
        run.get("workflow_id") == workflow_id
        and run.get("path", "").split("@", 1)[0] == WORKFLOW_PATH,
        "Approval artifact was not produced by the promotion workflow.",
    )
    require(
        run.get("event")
        in ({"push", "workflow_dispatch"} if candidate else {"workflow_dispatch"})
        and run.get("head_branch") == "main",
        "Approval artifacts must originate from an explicit dispatch on main.",
    )
    for key in ("repository", "head_repository"):
        repo = run.get(key) or {}
        require(
            repo.get("id") == repository_id and repo.get("full_name") == repository,
            "Approval workflow changed repositories.",
        )


def read_artifact(
    artifact,
    repository,
    repository_id,
    workflow_id,
    number,
    approval,
    *,
    candidate=False,
):
    filename = "candidate.json" if candidate else "approval.json"
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
        and run.get("event")
        in ({"push", "workflow_dispatch"} if candidate else {"workflow_dispatch"})
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
            and entries[0].filename == filename
            and not entries[0].is_dir()
            and entries[0].file_size <= MAX_RECORD_BYTES,
            f"Promotion artifact must contain only {filename}.",
        )
        record = json.loads(bundle.read(entries[0]))
    if candidate:
        validate_candidate(record, repository, repository_id, number)
    else:
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
        if job.get("name")
        == (f"Promote {repository}#{number}" if candidate else CAPTURE_JOB)
    ]
    if candidate:
        require(
            len(jobs) == 1 and jobs[0].get("head_sha") == run["head_sha"],
            "The candidate writer job does not match its workflow.",
        )
        # A successful upload survives a later publication failure. Each prerequisite must
        # have succeeded in this exact attempt, with the artifact inside the upload step.
        steps = []
        incomplete_upload = False
        for name in (
            "Verify promotion revision",
            "Recheck promotion plan",
            "Capture verified promotion candidate",
            "Persist verified promotion candidate",
        ):
            matches = [
                step for step in jobs[0].get("steps", []) if step.get("name") == name
            ]
            require(
                len(matches) == 1 and matches[0].get("status") == "completed",
                "Candidate verification, recheck, and upload must all complete.",
            )
            step = matches[0]
            if name == "Persist verified promotion candidate" and step.get(
                "conclusion"
            ) in {"failure", "cancelled"}:
                incomplete_upload = True
            else:
                require(
                    step.get("conclusion") == "success",
                    "Candidate verification, recheck, and upload must all succeed.",
                )
            steps.append(step)
        require(
            all(
                before["number"] < after["number"]
                and timestamp(before["completed_at"]) <= timestamp(after["started_at"])
                for before, after in itertools.pairwise(steps)
            ),
            "Candidate proof steps ran in an unexpected order.",
        )
        require(
            timestamp(steps[-1]["started_at"])
            <= timestamp(artifact["created_at"])
            <= timestamp(steps[-1]["completed_at"]),
            "Candidate artifact was not created during its upload step.",
        )
        if incomplete_upload:
            raise IncompleteCandidateUpload(record)
    else:
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


def artifact_records(
    repository,
    repository_id,
    workflow_id,
    number,
    approval,
    name,
    *,
    candidate=False,
    incomplete_uploads=None,
):
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
            "GitHub returned an unrelated promotion artifact.",
        )
        if artifact.get("expired") or timestamp(
            artifact["expires_at"]
        ) <= datetime.datetime.now(datetime.UTC):
            continue
        try:
            record = read_artifact(
                artifact,
                repository,
                repository_id,
                workflow_id,
                number,
                approval,
                candidate=candidate,
            )
        except IncompleteCandidateUpload as error:
            # The payload is well formed, but cannot prove that its upload finished.
            if incomplete_uploads is not None:
                incomplete_uploads.append(error.record)
            continue
        if record is not None:
            records.append(record)
    return records


def recorded_approval(repository, repository_id, workflow_id, number, approval):
    records = artifact_records(
        repository,
        repository_id,
        workflow_id,
        number,
        approval,
        artifact_name(repository_id, number, approval["id"]),
    )
    if not records:
        return None
    require(
        all(binding(record) == binding(records[0]) for record in records),
        "Conflicting approval records require a fresh approval.",
    )
    return records[0]


CANDIDATE_CONTEXT = (
    "source_base",
    "source_base_sha",
    "base_ref",
    "base_sha",
    "previous_sha",
    "merge_sha",
    "merged_parent_number",
)


def candidate_artifact_name(repository_id, number, head_sha, candidate_sha):
    # Only clean Git parent updates can supply this proof, including when a branch already exists.
    return (
        f"promotion-clean-candidate-{repository_id}-{number}-{head_sha}-{candidate_sha}"
    )


def validate_candidate(record, repository, repository_id, number):
    approval = record.get("approval") if isinstance(record, dict) else None
    require(
        isinstance(approval, dict)
        and set(approval) == {"id", "event", "actor", "created_at", "promotion_label"}
        and positive_integer(approval.get("id"))
        and approval.get("event") in {"labeled", "ready_for_review"}
        and approval.get("promotion_label") in LABELS,
        "Candidate approval provenance is invalid.",
    )
    actor = approval.get("actor")
    require(
        isinstance(actor, dict)
        and set(actor) == {"id", "login", "type"}
        and positive_integer(actor.get("id"))
        and actor.get("type") == "User"
        and isinstance(actor.get("login"), str)
        and actor["login"],
        "Candidate approval actor is invalid.",
    )
    require(
        (
            approval["event"] != "ready_for_review"
            or (
                repository == "astral-sh/uv-dev"
                and approval["promotion_label"] == "bot:promote"
            )
        )
        and (
            repository != "astral-sh/uv-security"
            or approval["promotion_label"] == "bot:promote"
        ),
        "Candidate approval mode is invalid.",
    )
    timestamp(approval["created_at"])
    validate_record(record, repository, repository_id, number, approval, candidate=True)
    candidate = record["candidate"]
    require(
        isinstance(candidate, dict)
        and set(candidate) == {"head_sha", *CANDIDATE_CONTEXT},
        "Candidate transformation schema is invalid.",
    )
    for field in (
        "head_sha",
        "source_base_sha",
        "base_sha",
        "previous_sha",
        "merge_sha",
    ):
        require(
            isinstance(candidate[field], str)
            and re.fullmatch(r"[0-9a-f]{40}", candidate[field]),
            "Candidate transformation revision is invalid.",
        )
    require(
        isinstance(candidate["merged_parent_number"], str)
        and re.fullmatch(r"[1-9][0-9]*", candidate["merged_parent_number"]),
        "Candidate parent identity is invalid.",
    )
    for field in ("source_base", "base_ref"):
        require(isinstance(candidate[field], str), "Candidate branch is invalid.")
        subprocess.run(
            ["git", "check-ref-format", "refs/heads/" + candidate[field]], check=True
        )


def planned_candidate_source(repository, repository_id, number, approval, plan):
    require(
        isinstance(plan, dict)
        and type(plan.get("number")) is int
        and plan["number"] == number
        and plan.get("approval_id") == str(approval["id"])
        and plan.get("approval_event") == approval["event"]
        and plan.get("promotion_label") == approval["promotion_label"]
        and plan.get("promoter") == approval["actor"]["login"],
        "The current approval does not authorize this candidate plan.",
    )
    source = eligible_source(
        repository,
        repository_id,
        number,
        approval,
        plan["head_sha"],
        plan["head_ref"],
        plan["source_base"],
    )
    require(
        source["base"]["ref"] == plan["source_base"]
        and source["base"]["sha"] == plan["source_base_sha"],
        "The candidate source parent changed.",
    )
    return source


def candidate_binding(record):
    return {
        key: value for key, value in record.items() if key not in {"run", "approval"}
    }


def recorded_candidate(
    repository,
    repository_id,
    workflow_id,
    number,
    approval,
    plan,
    candidate_sha,
    *,
    required=True,
    incomplete_uploads=None,
):
    records = artifact_records(
        repository,
        repository_id,
        workflow_id,
        number,
        approval,
        candidate_artifact_name(repository_id, number, plan["head_sha"], candidate_sha),
        candidate=True,
        incomplete_uploads=incomplete_uploads,
    )
    require(
        all(record["candidate"]["head_sha"] == candidate_sha for record in records),
        "A candidate artifact names another revision.",
    )
    if not records:
        require(
            not required,
            "No trusted candidate record matches the existing promotion branch.",
        )
        return None
    # Historical approval proves how the candidate was produced, not permission to publish it.
    # A fresh captured approval of the same source can choose a different promotion mode.
    transformations = [candidate_binding(record) for record in records]
    require(
        all(value == transformations[0] for value in transformations),
        "Conflicting candidate records cannot authorize branch reuse.",
    )
    record = records[0]
    require(
        record["head_ref"] == plan["head_ref"]
        and record["head_sha"] == plan["head_sha"],
        "The candidate source revision or branch changed.",
    )
    candidate = record["candidate"]
    require(
        all(
            candidate[field] == plan[field]
            for field in CANDIDATE_CONTEXT
            if field != "base_sha"
        ),
        "The candidate parent or target branch changed.",
    )
    if candidate["base_sha"] != plan["base_sha"]:
        comparison = gh_json(
            f"repos/{repository}/compare/{candidate['base_sha']}...{plan['base_sha']}"
        )
        require(
            comparison.get("status") in {"ahead", "identical"},
            "The candidate base is not an ancestor of the current target.",
        )
    return record


def capture_candidate(
    repository, repository_id, workflow_id, number, approval, plan, candidate_sha
):
    require(
        plan.get("action") == "rebase" and not plan.get("existing_head_sha"),
        "Only newly verified rebases can create candidate records.",
    )
    require(
        os.environ.get("GITHUB_EVENT_NAME") in {"push", "workflow_dispatch"}
        and os.environ.get("GITHUB_REF") == "refs/heads/main"
        and os.environ.get("GITHUB_REPOSITORY") == repository
        and os.environ.get("GITHUB_REPOSITORY_ID") == str(repository_id),
        "Candidate capture requires the source promotion workflow on main.",
    )
    run_id = int(os.environ["GITHUB_RUN_ID"])
    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    require(run_id > 0 and attempt > 0, "Current workflow identity is invalid.")
    run = gh_json(f"repos/{repository}/actions/runs/{run_id}")
    validate_run(run, repository, repository_id, workflow_id, candidate=True)
    require(
        run.get("run_attempt") == attempt
        and run.get("head_sha") == os.environ.get("GITHUB_SHA"),
        "The current workflow attempt changed.",
    )
    record = {
        "schema": 2,
        "repository": {"id": repository_id, "full_name": repository},
        "number": number,
        "head_ref": plan["head_ref"],
        "head_sha": plan["head_sha"],
        "approval": approval,
        "run": {
            "id": run_id,
            "attempt": attempt,
            "workflow_id": workflow_id,
            "head_sha": run["head_sha"],
        },
        "candidate": {
            "head_sha": candidate_sha,
            **{field: plan[field] for field in CANDIDATE_CONTEXT},
        },
    }
    validate_candidate(record, repository, repository_id, number)
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", plan["base_sha"], candidate_sha],
        check=True,
    )
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", plan["merge_sha"], plan["base_sha"]],
        check=True,
    )
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=(
            "current",
            "capture",
            "load",
            "source",
            "capture-candidate",
            "load-candidate",
        ),
    )
    parser.add_argument("repository", choices=sorted(REPOSITORIES))
    parser.add_argument("number", type=int)
    parser.add_argument("--repository-id", type=int)
    parser.add_argument("--head-sha")
    parser.add_argument("--plan")
    parser.add_argument("--candidate-sha")
    parser.add_argument("--head-ref")
    parser.add_argument("--base-ref")
    parser.add_argument("--approval-id", type=int)
    parser.add_argument("--approval-event", choices=("labeled", "ready_for_review"))
    parser.add_argument("--promoter")
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
    if args.command == "source":
        require(
            repository_id == args.repository_id,
            "The source repository identity changed.",
        )
        require(
            args.head_sha is not None
            and re.fullmatch(r"[0-9a-f]{40}", args.head_sha)
            and args.head_ref is not None
            and args.base_ref is not None,
            "The planned source revision and branches are required.",
        )
        require(
            approval["id"] == args.approval_id
            and approval["event"] == args.approval_event
            and approval["promotion_label"] == args.promotion_label
            and approval["actor"]["login"] == args.promoter,
            "The human promotion event changed.",
        )
        source = eligible_source(
            args.repository,
            repository_id,
            args.number,
            approval,
            args.head_sha,
            args.head_ref,
            args.base_ref,
        )
        print(json.dumps(source, separators=(",", ":")))
        return
    workflow_id = gh_json(
        f"repos/{args.repository}/actions/workflows/promote-pull-request.yml"
    )["id"]
    require(positive_integer(workflow_id), "Workflow identity is invalid.")
    if args.command in {"capture-candidate", "load-candidate"}:
        require(
            args.repository_id == repository_id
            and args.plan is not None
            and args.candidate_sha is not None
            and re.fullmatch(r"[0-9a-f]{40}", args.candidate_sha),
            "The candidate plan, repository, and revision are required.",
        )
        plan = json.loads(args.plan)
        planned_candidate_source(
            args.repository, repository_id, args.number, approval, plan
        )
        if args.command == "load-candidate":
            record = recorded_candidate(
                args.repository,
                repository_id,
                workflow_id,
                args.number,
                approval,
                plan,
                args.candidate_sha,
            )
            print(json.dumps(record, separators=(",", ":")))
        else:
            record = capture_candidate(
                args.repository,
                repository_id,
                workflow_id,
                args.number,
                approval,
                plan,
                args.candidate_sha,
            )
            incomplete_uploads = []
            existing_candidate = recorded_candidate(
                args.repository,
                repository_id,
                workflow_id,
                args.number,
                approval,
                plan,
                args.candidate_sha,
                required=False,
                incomplete_uploads=incomplete_uploads,
            )
            overwrite = False
            if existing_candidate is not None:
                record = existing_candidate
            else:
                same_run = [
                    old
                    for old in incomplete_uploads
                    if old["run"]["id"] == record["run"]["id"]
                ]
                require(
                    all(
                        old["run"]["attempt"] < record["run"]["attempt"]
                        and candidate_binding(old) == candidate_binding(record)
                        for old in same_run
                    ),
                    "Only an identical incomplete candidate from an earlier attempt may be replaced.",
                )
                overwrite = bool(same_run)
            print(
                json.dumps(
                    {
                        "record": record,
                        "upload": existing_candidate is None,
                        "overwrite": overwrite,
                        "artifact_name": candidate_artifact_name(
                            repository_id,
                            args.number,
                            plan["head_sha"],
                            args.candidate_sha,
                        ),
                    },
                    separators=(",", ":"),
                )
            )
        return
    existing = recorded_approval(
        args.repository, repository_id, workflow_id, args.number, approval
    )
    if args.command == "load":
        require(
            existing is not None,
            "No durable approval record remains; a fresh explicit approval or dispatch is required.",
        )
        eligible_source(
            args.repository,
            repository_id,
            args.number,
            approval,
            existing["head_sha"],
            existing["head_ref"],
            existing["base_ref"],
        )
        print(json.dumps(existing, separators=(",", ":")))
        return
    require(
        args.head_sha is not None and re.fullmatch(r"[0-9a-f]{40}", args.head_sha),
        "An explicitly approved head SHA is required.",
    )
    require(
        isinstance(args.base_ref, str) and args.base_ref,
        "An explicitly approved source base is required.",
    )
    subprocess.run(
        ["git", "check-ref-format", "refs/heads/" + args.base_ref], check=True
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
        args.repository,
        repository_id,
        args.number,
        approval,
        args.head_sha,
        expected_base=args.base_ref,
    )
    record = {
        "schema": 2,
        "repository": {"id": repository_id, "full_name": args.repository},
        "number": args.number,
        "head_ref": source["head"]["ref"],
        "head_sha": args.head_sha,
        "base_ref": args.base_ref,
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
