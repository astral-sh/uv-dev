"""Find conflicted pull requests after GitHub computes mergeability."""

import logging
import time
from collections.abc import Callable
from typing import assert_never

from uv_automations.github import PullRequestReader
from uv_automations.models import Mergeability, PullRequest, RepositoryName

logger = logging.getLogger(__name__)


def find_conflicted_pull_requests(
    github: PullRequestReader,
    repository: RepositoryName,
    *,
    author: str | None = None,
    max_attempts: int = 5,
    retry_delay: float = 5,
    sleep: Callable[[float], None] = time.sleep,
) -> tuple[PullRequest, ...]:
    if max_attempts < 1 or retry_delay < 0:
        raise ValueError("The retry budget and delay must be non-negative")

    for attempt in range(1, max_attempts + 1):
        pull_requests = github.list_pull_requests(repository, author=author)
        conflicts: list[PullRequest] = []
        unknown = 0
        for pull_request in pull_requests:
            match pull_request.mergeability:
                case Mergeability.CONFLICTING:
                    conflicts.append(pull_request)
                    continue
                case Mergeability.MERGEABLE:
                    continue
                case Mergeability.UNKNOWN:
                    unknown += 1
                    continue
            assert_never(pull_request.mergeability)

        if unknown == 0 or attempt == max_attempts:
            if unknown:
                logger.warning(
                    "GitHub could not determine mergeability for %s pull requests.",
                    unknown,
                )
            return tuple(conflicts)

        logger.info(
            "Waiting for GitHub to calculate mergeability for %s pull requests "
            "(attempt %s/%s)...",
            unknown,
            attempt,
            max_attempts,
        )
        sleep(retry_delay)

    raise AssertionError("The retry loop must return")


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
