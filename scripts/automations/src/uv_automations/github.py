"""Typed GitHub operations used by the automation workflows."""

import json
import os
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Literal, Protocol
from urllib.parse import quote

from uv_automations.json import (
    as_array,
    as_object,
    as_positive_integer,
    as_string,
    loads,
)
from uv_automations.models import (
    CommitSha,
    Issue,
    IssueAuthor,
    IssueRef,
    Label,
    Mergeability,
    PullRequest,
    PullRequestDetails,
    PullRequestRef,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
)

PULL_REQUEST_FIELDS = (
    "number,author,mergeable,url,baseRefName,headRefName,headRefOid,headRepository"
)
LABEL_CONTEXT_FIELDS = (
    "number,title,body,author,baseRefName,headRefName,headRefOid,isDraft,"
    "labels,files,additions,deletions,changedFiles"
)
ISSUE_FIELDS = "number,title,body,author,url"


@dataclass(frozen=True, slots=True)
class PullRequestLabelContext:
    head_sha: CommitSha
    event_json: str


@dataclass(frozen=True, slots=True)
class OpenPullRequestQuery:
    repository: RepositoryName
    base: str
    author: str | None = None
    limit: int = 1000

    def __post_init__(self) -> None:
        as_positive_integer(self.limit)


class PullRequestReader(Protocol):
    def list_open_pull_requests(
        self, query: OpenPullRequestQuery
    ) -> tuple[PullRequest, ...]: ...


class IssueReader(Protocol):
    def get_issue(self, reference: IssueRef) -> Issue: ...


def _decode_issue_author(value: object) -> IssueAuthor | None:
    if value is None:
        return None
    data = as_object(value)
    is_bot = data["is_bot"]
    if type(is_bot) is not bool:
        raise TypeError("Expected a JSON boolean")
    name = data["name"]
    return IssueAuthor(
        node_id=as_string(data["id"]),
        is_bot=is_bot,
        login=as_string(data["login"]),
        name=as_string(name) if name is not None else None,
    )


def decode_issue(value: object, reference: IssueRef) -> Issue:
    data = as_object(value)
    if (
        as_positive_integer(data["number"]) != reference.number
        or as_string(data["url"]) != reference.url
    ):
        raise ValueError("GitHub returned an unexpected issue")
    return Issue(
        reference=reference,
        title=as_string(data["title"]),
        body=as_string(data["body"]),
        author=_decode_issue_author(data["author"]),
    )


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


def _decode_repository(value: object) -> RepositoryIdentity | None:
    if value is None:
        return None
    data = as_object(value)
    return RepositoryIdentity(
        RepositoryName(as_string(data["full_name"])),
        as_positive_integer(data["id"]),
    )


def _decode_revision(value: object) -> PullRequestRevision:
    data = as_object(value)
    return PullRequestRevision(
        repository=_decode_repository(data["repo"]),
        ref=as_string(data["ref"]),
        sha=CommitSha(as_string(data["sha"])),
    )


def _decode_label_names(value: object) -> tuple[str, ...]:
    return tuple(as_string(as_object(label)["name"]) for label in as_array(value))


def decode_pull_request_details(
    value: object, reference: PullRequestRef
) -> PullRequestDetails:
    data = as_object(value)
    if as_positive_integer(data["number"]) != reference.number:
        raise ValueError("GitHub returned an unexpected pull request")
    return PullRequestDetails(
        reference=reference,
        state=PullRequestState(as_string(data["state"])),
        url=as_string(data["html_url"]),
        base=_decode_revision(data["base"]),
        head=_decode_revision(data["head"]),
        labels=_decode_label_names(data["labels"]),
    )


@dataclass(frozen=True, slots=True)
class GitHub:
    """Use the caller's existing GitHub CLI authentication."""

    executable: str = "gh"
    token_variable: str | None = field(default=None, kw_only=True)

    def _environment(self) -> dict[str, str] | None:
        if self.token_variable is None:
            return None
        environment = os.environ.copy()
        token = environment.get(self.token_variable)
        if not token:
            raise ValueError(f"Missing GitHub token environment: {self.token_variable}")
        environment["GH_TOKEN"] = token
        return environment

    def _command(
        self, arguments: Sequence[str], *, payload: object | None = None
    ) -> object:
        result = subprocess.run(
            [self.executable, *arguments],
            input=(
                json.dumps(payload, allow_nan=False) if payload is not None else None
            ),
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            env=self._environment(),
            timeout=60,
        )
        return loads(result.stdout) if result.stdout.strip() else None

    def _api(
        self,
        method: Literal["GET", "POST", "DELETE"],
        path: str,
        *,
        payload: object | None = None,
    ) -> object:
        arguments = ["api", "--method", method, path]
        if payload is not None:
            arguments.extend(("--input", "-"))
        return self._command(arguments, payload=payload)

    def list_open_pull_requests(
        self, query: OpenPullRequestQuery
    ) -> tuple[PullRequest, ...]:
        """Read the bounded result set described by `query`."""
        arguments = [
            "pr",
            "list",
            "--repo",
            str(query.repository),
            "--base",
            query.base,
            "--state",
            "open",
            "--limit",
            str(query.limit),
            "--json",
            PULL_REQUEST_FIELDS,
        ]
        if query.author is not None:
            arguments.extend(("--author", query.author))
        return tuple(
            decode_pull_request(value, query.repository)
            for value in as_array(self._command(arguments))
        )

    def get_issue(self, reference: IssueRef) -> Issue:
        return decode_issue(
            self._command(
                [
                    "issue",
                    "view",
                    str(reference.number),
                    "--repo",
                    str(reference.repository),
                    "--json",
                    ISSUE_FIELDS,
                ]
            ),
            reference,
        )

    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails:
        return decode_pull_request_details(
            self._api("GET", f"repos/{reference.repository}/pulls/{reference.number}"),
            reference,
        )

    def get_label_context(self, reference: PullRequestRef) -> PullRequestLabelContext:
        event = as_object(
            self._command(
                [
                    "pr",
                    "view",
                    str(reference.number),
                    "--repo",
                    str(reference.repository),
                    "--json",
                    LABEL_CONTEXT_FIELDS,
                ]
            )
        )
        if as_positive_integer(event["number"]) != reference.number:
            raise ValueError("GitHub returned an unexpected pull request")
        return PullRequestLabelContext(
            head_sha=CommitSha(as_string(event["headRefOid"])),
            event_json=json.dumps(event, allow_nan=False),
        )

    def list_labels(self, repository: RepositoryName) -> tuple[Label, ...]:
        labels = as_array(
            self._command(
                [
                    "label",
                    "list",
                    "--repo",
                    str(repository),
                    "--limit",
                    "1000",
                    "--json",
                    "name,description",
                ]
            )
        )
        result: list[Label] = []
        for value in labels:
            data = as_object(value)
            description = data["description"]
            result.append(
                Label(
                    as_string(data["name"]),
                    as_string(description) if description is not None else None,
                )
            )
        return tuple(result)

    def add_labels(self, reference: PullRequestRef, labels: tuple[str, ...]) -> None:
        self._api(
            "POST",
            f"repos/{reference.repository}/issues/{reference.number}/labels",
            payload={"labels": labels},
        )

    def get_pull_request_labels(self, reference: PullRequestRef) -> tuple[str, ...]:
        issue = as_object(
            self._api("GET", f"repos/{reference.repository}/issues/{reference.number}")
        )
        return _decode_label_names(issue["labels"])

    def remove_label(self, reference: PullRequestRef, label: str) -> None:
        self._api(
            "DELETE",
            f"repos/{reference.repository}/issues/{reference.number}/labels/"
            f"{quote(label, safe='')}",
        )
