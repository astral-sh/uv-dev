"""The GitHub reads needed by the automation workflows."""

import subprocess
from dataclasses import dataclass
from typing import Protocol

from uv_automations.json import (
    as_array,
    as_object,
    as_positive_integer,
    as_string,
    loads,
)
from uv_automations.models import (
    CommitSha,
    Mergeability,
    PullRequest,
    PullRequestRef,
    RepositoryName,
)

PULL_REQUEST_FIELDS = (
    "number,author,mergeable,url,baseRefName,headRefName,headRefOid,headRepository"
)


class PullRequestReader(Protocol):
    def list_pull_requests(
        self, repository: RepositoryName, *, author: str | None = None
    ) -> tuple[PullRequest, ...]: ...


def decode_pull_request(value: object, repository: RepositoryName) -> PullRequest:
    data = as_object(value)
    head_repository = data["headRepository"]
    author = data["author"]
    return PullRequest(
        reference=PullRequestRef(repository, as_positive_integer(data["number"])),
        author=as_string(as_object(author)["login"]) if author is not None else None,
        url=as_string(data["url"]),
        base_ref=as_string(data["baseRefName"]),
        head_ref=as_string(data["headRefName"]),
        head_sha=CommitSha(as_string(data["headRefOid"])),
        head_repository=(
            RepositoryName(as_string(as_object(head_repository)["nameWithOwner"]))
            if head_repository is not None
            else None
        ),
        mergeability=Mergeability(as_string(data["mergeable"])),
    )


@dataclass(frozen=True, slots=True)
class GitHub:
    """Use the caller's existing GitHub CLI authentication."""

    executable: str = "gh"

    def list_pull_requests(
        self, repository: RepositoryName, *, author: str | None = None
    ) -> tuple[PullRequest, ...]:
        arguments = [
            self.executable,
            "pr",
            "list",
            "--repo",
            str(repository),
            "--base",
            "main",
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            PULL_REQUEST_FIELDS,
        ]
        if author is not None:
            arguments.extend(("--author", author))
        result = subprocess.run(
            arguments, check=True, text=True, stdout=subprocess.PIPE, timeout=60
        )
        return tuple(
            decode_pull_request(value, repository)
            for value in as_array(loads(result.stdout))
        )
