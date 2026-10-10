"""Resolve a release run's source commit and reject conflicting existing tags."""

import argparse
import json
import re
import subprocess

TAG_QUERY = """
query($owner: String!, $name: String!, $tag: String!) {
  repository(owner: $owner, name: $name) {
    ref(qualifiedName: $tag) { target { __typename oid } }
  }
}
"""


def github(*arguments: str) -> dict:
    return json.loads(subprocess.check_output(["gh", "api", *arguments], text=True))


def full_commit(value: str) -> str:
    if re.fullmatch(r"[0-9a-fA-F]{40}", value) is None:
        raise ValueError("Expected a full source commit SHA")
    return value.lower()


def run_commit(repository: str, run_id: int) -> str:
    run = github(f"repos/{repository}/actions/runs/{run_id}")
    if (
        run.get("path") != ".github/workflows/release.yml"
        or run.get("event") != "workflow_dispatch"
        or run.get("head_branch") != "main"
        or run.get("status") != "completed"
        or run.get("repository", {}).get("full_name") != repository
    ):
        raise ValueError(
            "Recovery requires a completed release dispatch from main in the selected repository"
        )
    return full_commit(run["head_sha"])


def check_tag(repository: str, tag: str, commit: str) -> None:
    owner, name = repository.split("/", 1)
    data = github(
        "graphql",
        "-f",
        f"owner={owner}",
        "-f",
        f"name={name}",
        "-f",
        f"tag=refs/tags/{tag}",
        "-f",
        f"query={TAG_QUERY}",
    )
    if data.get("errors"):
        raise ValueError("Could not read the release tag")
    remote = data["data"]["repository"]
    if remote is None:
        raise ValueError("Could not read the release repository")
    reference = remote["ref"]
    if reference is None:
        return
    target = reference["target"]
    kind, oid = target["__typename"].lower(), full_commit(target["oid"])
    # Annotated tags may point to other annotated tags. Peel each object before
    # comparing the source commit, with a bound on the number of API requests.
    for _ in range(16):
        if kind == "commit":
            if oid != commit:
                raise ValueError(
                    f"Release tag {tag} points to {oid}, expected {commit}"
                )
            return
        if kind != "tag":
            raise ValueError(f"Release tag {tag} does not point to a commit")
        target = github(f"repos/{repository}/git/tags/{oid}")["object"]
        kind, oid = target["type"], full_commit(target["sha"])
    raise ValueError(f"Release tag {tag} has too many nested annotated tags")


def check_identity(
    repository: str, commit: str | None, run_id: int | None, tag: str | None
) -> str:
    source = full_commit(commit) if commit is not None else None
    if run_id is not None:
        if run_id <= 0:
            raise ValueError("Expected a positive workflow run ID")
        built = run_commit(repository, run_id)
        if source is not None and source != built:
            raise ValueError(
                f"Requested commit {source} differs from release run {run_id} source {built}"
            )
        source = built
    if source is None:
        raise ValueError("A source commit or release run ID is required")
    if tag is not None:
        check_tag(repository, tag, source)
    return source


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--commit")
    parser.add_argument("--run-id", type=int)
    parser.add_argument("--tag")
    args = parser.parse_args()
    try:
        print(check_identity(args.repository, args.commit, args.run_id, args.tag))
    except (ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"Release identity check failed: {error}\n")


if __name__ == "__main__":
    main()
