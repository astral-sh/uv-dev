"""Values shared across otherwise independent automation workflows."""

import re
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from enum import StrEnum
from typing import assert_never, override

from uv_automations.json import as_positive_integer


class ManagedRepository(StrEnum):
    UV = "astral-sh/uv"
    UV_DEV = "astral-sh/uv-dev"
    UV_SECURITY = "astral-sh/uv-security"


@dataclass(frozen=True, slots=True)
class RepositoryName:
    full_name: str

    def __post_init__(self) -> None:
        if re.fullmatch(
            r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+", self.full_name
        ) is None or self.full_name.rsplit("/", 1)[1] in {".", ".."}:
            raise ValueError(f"Invalid GitHub repository name: {self.full_name!r}")

    @override
    def __str__(self) -> str:
        return self.full_name


@dataclass(frozen=True, slots=True)
class CommitSha:
    value: str

    def __post_init__(self) -> None:
        if re.fullmatch(r"[0-9a-f]{40}", self.value) is None:
            raise ValueError(f"Expected a full Git commit SHA: {self.value!r}")

    @override
    def __str__(self) -> str:
        return self.value


@dataclass(frozen=True, slots=True, order=True)
class Timestamp:
    """GitHub's UTC, whole-second timestamp representation."""

    value: datetime

    def __post_init__(self) -> None:
        if self.value.utcoffset() != timedelta(0) or self.value.microsecond:
            raise ValueError("Expected a UTC timestamp with whole-second precision")

    @classmethod
    def parse(cls, value: str) -> Timestamp:
        if (
            re.fullmatch(
                r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", value
            )
            is None
        ):
            raise ValueError("Expected a canonical UTC timestamp")
        return cls(datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=UTC))

    @classmethod
    def now(cls) -> Timestamp:
        return cls(datetime.now(UTC).replace(microsecond=0))

    def overlap(self) -> Timestamp:
        # GitHub's `since` filters are exclusive and have second precision.
        return Timestamp(self.value - timedelta(seconds=1))

    @override
    def __str__(self) -> str:
        return self.value.strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass(frozen=True, slots=True)
class RepositoryIdentity:
    name: RepositoryName
    database_id: int

    def __post_init__(self) -> None:
        as_positive_integer(self.database_id)


@dataclass(frozen=True, slots=True)
class PullRequestRef:
    repository: RepositoryName
    number: int

    def __post_init__(self) -> None:
        as_positive_integer(self.number)


class PullRequestState(StrEnum):
    OPEN = "open"
    CLOSED = "closed"


@dataclass(frozen=True, slots=True)
class PullRequestRevision:
    repository: RepositoryIdentity | None
    ref: str
    sha: CommitSha


@dataclass(frozen=True, slots=True)
class PullRequestDetails:
    reference: PullRequestRef
    state: PullRequestState
    url: str
    base: PullRequestRevision
    head: PullRequestRevision
    labels: tuple[str, ...]

    @property
    def is_open(self) -> bool:
        match self.state:
            case PullRequestState.OPEN:
                return True
            case PullRequestState.CLOSED:
                return False
        assert_never(self.state)


@dataclass(frozen=True, slots=True)
class Label:
    name: str
    description: str | None


class Mergeability(StrEnum):
    CONFLICTING = "CONFLICTING"
    MERGEABLE = "MERGEABLE"
    UNKNOWN = "UNKNOWN"


@dataclass(frozen=True, slots=True)
class PullRequest:
    reference: PullRequestRef
    author: str | None
    url: str
    base_ref: str
    head_ref: str
    head_sha: CommitSha
    # Deleted forks do not have a head repository.
    head_repository: RepositoryName | None
    mergeability: Mergeability
