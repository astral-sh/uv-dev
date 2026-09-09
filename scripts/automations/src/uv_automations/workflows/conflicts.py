"""Find conflicted pull requests after GitHub computes mergeability."""

import logging
import time
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from typing import Protocol, assert_never

from uv_automations import git
from uv_automations.github import OpenPullRequestQuery, PullRequestReader
from uv_automations.models import (
    CommitSha,
    ManagedRepository,
    Mergeability,
    PullRequest,
    PullRequestDetails,
    PullRequestRef,
    RepositoryIdentity,
    RepositoryName,
)

logger = logging.getLogger(__name__)


@dataclass(frozen=True, slots=True)
class MergeabilitySummary:
    conflicting: tuple[PullRequest, ...]
    pending: int


def summarize_mergeability(
    pull_requests: Iterable[PullRequest],
) -> MergeabilitySummary:
    conflicting: list[PullRequest] = []
    pending = 0
    for pull_request in pull_requests:
        mergeability = pull_request.mergeability
        match mergeability:
            case Mergeability.CONFLICTING:
                conflicting.append(pull_request)
                continue
            case Mergeability.MERGEABLE:
                continue
            case Mergeability.UNKNOWN:
                pending += 1
                continue
        assert_never(mergeability)
    return MergeabilitySummary(tuple(conflicting), pending)


def find_conflicted_pull_requests(
    github: PullRequestReader,
    repository: RepositoryName,
    *,
    author: str | None = None,
    max_attempts: int = 5,
    retry_delay: float = 5,
    sleep: Callable[[float], None] = time.sleep,
) -> tuple[PullRequest, ...]:
    if max_attempts < 1:
        raise ValueError("The retry budget must allow at least one attempt")
    if retry_delay < 0:
        raise ValueError("The retry delay must be non-negative")

    query = OpenPullRequestQuery(repository, base="main", author=author)
    attempt = 1
    while True:
        summary = summarize_mergeability(github.list_open_pull_requests(query))
        if summary.pending == 0 or attempt == max_attempts:
            break

        logger.info(
            "Waiting for GitHub to calculate mergeability for %s pull requests "
            "(attempt %s/%s)...",
            summary.pending,
            attempt,
            max_attempts,
        )
        sleep(retry_delay)
        attempt += 1

    if summary.pending:
        logger.warning(
            "GitHub could not determine mergeability for %s pull requests.",
            summary.pending,
        )
    return summary.conflicting


def conflict_payload(pull_request: PullRequest) -> dict[str, str | int | None]:
    """Preserve the JSON consumed by pull-request-conflicts.yml."""
    return {
        "number": pull_request.reference.number,
        "author": pull_request.author,
        "url": pull_request.url,
        "base_ref": pull_request.base_ref,
        "head_ref": pull_request.head_ref,
        "head_sha": str(pull_request.head_sha),
        "head_repository": (
            str(pull_request.head_repository)
            if pull_request.head_repository is not None
            else None
        ),
    }


@dataclass(frozen=True, slots=True)
class PreviousBase:
    ref: str
    sha: CommitSha


@dataclass(frozen=True, slots=True)
class RebaseDispatch:
    reference: PullRequestRef
    expected_head: CommitSha
    previous_base: PreviousBase | None = None


def parse_dispatch(
    repository: RepositoryName,
    number: int | None,
    expected_head: CommitSha | None,
    previous_ref: str | None,
    previous_sha: CommitSha | None,
) -> RebaseDispatch | None:
    if number is None:
        if any(
            value is not None for value in (expected_head, previous_ref, previous_sha)
        ):
            raise ValueError("The dispatched rebase context is incomplete")
        return None
    if expected_head is None:
        raise ValueError("A dispatched rebase requires the expected head SHA")
    if previous_ref is None and previous_sha is None:
        previous_base = None
    elif previous_ref is not None and previous_sha is not None:
        previous_base = PreviousBase(previous_ref, previous_sha)
    else:
        raise ValueError("The previous pull request base is incomplete")
    return RebaseDispatch(
        PullRequestRef(repository, number), expected_head, previous_base
    )


@dataclass(frozen=True, slots=True)
class RebaseCandidate:
    reference: PullRequestRef
    url: str
    base_ref: str
    previous_base: CommitSha | None
    head_ref: str
    head_sha: CommitSha
    head_repository: RepositoryName | None

    def matrix_entry(self) -> dict[str, str | int]:
        if self.head_repository is None:
            raise ValueError("A rebase candidate must have a head repository")
        return {
            "number": self.reference.number,
            "base_ref": self.base_ref,
            "base_previous_sha": (
                str(self.previous_base) if self.previous_base is not None else ""
            ),
            "head_ref": self.head_ref,
            "head_sha": str(self.head_sha),
            "head_repository": str(self.head_repository),
        }


@dataclass(frozen=True, slots=True)
class RebasePlan:
    found: tuple[RebaseCandidate, ...]
    rebasable: tuple[RebaseCandidate, ...]

    def matrix(self) -> dict[str, list[dict[str, str | int]]]:
        return {"include": [candidate.matrix_entry() for candidate in self.rebasable]}

    def summary(self) -> str:
        lines = [
            "## Pull requests to rebase",
            "",
            f"Found {len(self.found)} pull requests to rebase.",
        ]
        unavailable = len(self.found) - len(self.rebasable)
        if unavailable:
            lines.extend(
                (
                    "",
                    (
                        f"Skipping {unavailable} pull requests whose heads cannot be "
                        "updated by the rebase app token."
                    ),
                )
            )
        if self.found:
            lines.append("")
            lines.extend(f"- {candidate.url}" for candidate in self.found)
        return "\n".join(lines) + "\n"


class RebaseReader(PullRequestReader, Protocol):
    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails: ...


class RebaseLabelWriter(Protocol):
    def get_pull_request_labels(self, reference: PullRequestRef) -> tuple[str, ...]: ...
    def remove_label(self, reference: PullRequestRef, label: str) -> None: ...


def _scan_author(repository: ManagedRepository) -> str | None:
    match repository:
        case ManagedRepository.UV:
            return "app/astral-automations-bot"
        case ManagedRepository.UV_DEV:
            return None
        case ManagedRepository.UV_SECURITY:
            raise ValueError("Conflict discovery does not support uv-security")
    assert_never(repository)


def _dispatched_candidate(
    github: RebaseReader,
    repository: RepositoryIdentity,
    dispatch: RebaseDispatch,
) -> RebaseCandidate:
    if dispatch.reference.repository != repository.name:
        raise ValueError("The dispatched pull request belongs to another repository")
    previous_sha = None
    if dispatch.previous_base is not None:
        if repository.name.full_name != ManagedRepository.UV_DEV.value:
            raise ValueError("Previous-base rebases are restricted to uv-dev")
        git.check_branch(dispatch.previous_base.ref)
        if dispatch.previous_base.ref != "main":
            previous_sha = dispatch.previous_base.sha

    pull_request = github.get_pull_request(dispatch.reference)
    if (
        not pull_request.is_open
        or pull_request.base.repository != repository
        or pull_request.head.repository != repository
        or pull_request.head.sha != dispatch.expected_head
    ):
        raise ValueError(
            "The pull request is no longer open at the expected head; refusing to rebase it"
        )
    return RebaseCandidate(
        reference=dispatch.reference,
        url=pull_request.url,
        base_ref=pull_request.base.ref,
        previous_base=previous_sha,
        head_ref=pull_request.head.ref,
        head_sha=pull_request.head.sha,
        head_repository=repository.name,
    )


def identify_conflicts(
    github: RebaseReader,
    repository: RepositoryIdentity,
    dispatch: RebaseDispatch | None,
) -> RebasePlan:
    author = _scan_author(ManagedRepository(repository.name.full_name))
    if dispatch is not None:
        found = (_dispatched_candidate(github, repository, dispatch),)
    else:
        found = tuple(
            RebaseCandidate(
                reference=pull_request.reference,
                url=pull_request.url,
                base_ref=pull_request.base_ref,
                previous_base=None,
                head_ref=pull_request.head_ref,
                head_sha=pull_request.head_sha,
                head_repository=pull_request.head_repository,
            )
            for pull_request in find_conflicted_pull_requests(
                github, repository.name, author=author
            )
        )
    allowed = {repository.name, RepositoryName(ManagedRepository.UV_DEV.value)}
    rebasable = tuple(
        candidate for candidate in found if candidate.head_repository in allowed
    )
    if len(rebasable) > 256:
        raise ValueError(
            f"Found {len(rebasable)} rebasable pull requests, which exceeds "
            "the GitHub Actions matrix limit of 256"
        )
    return RebasePlan(found, rebasable)


def remove_rebase_label(github: RebaseLabelWriter, reference: PullRequestRef) -> None:
    if "bot:rebase" in github.get_pull_request_labels(reference):
        github.remove_label(reference, "bot:rebase")
