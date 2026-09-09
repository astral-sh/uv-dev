"""Prepare, transport, and publish rebases without trusting agent conclusions."""

from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Protocol, assert_never

from uv_automations.artifacts import (
    CommitBundle,
    CommitRange,
    load_commit,
    persist_commit,
)
from uv_automations.git import Git, check_branch
from uv_automations.json import as_positive_integer
from uv_automations.models import (
    CommitSha,
    ManagedRepository,
    PullRequestDetails,
    PullRequestRef,
    RepositoryIdentity,
    RepositoryName,
)

_GIT_CREDENTIALS = (
    "-c",
    "credential.helper=",
    "-c",
    "credential.https://github.com.helper=",
    "-c",
    "credential.https://github.com.helper=!gh auth git-credential",
)


def _allowed_head_repositories(
    repository: ManagedRepository,
) -> frozenset[RepositoryName]:
    match repository:
        case ManagedRepository.UV:
            return frozenset(
                (
                    RepositoryName(ManagedRepository.UV.value),
                    RepositoryName(ManagedRepository.UV_DEV.value),
                )
            )
        case ManagedRepository.UV_DEV:
            return frozenset((RepositoryName(ManagedRepository.UV_DEV.value),))
        case ManagedRepository.UV_SECURITY:
            raise ValueError("Pull request rebases only support uv and uv-dev")
    assert_never(repository)


@dataclass(frozen=True, slots=True, kw_only=True)
class RebaseSource:
    repository: RepositoryIdentity
    number: int
    base_ref: str
    head_repository: RepositoryName
    head_ref: str
    head_sha: CommitSha
    previous_base: CommitSha | None = None

    def __post_init__(self) -> None:
        as_positive_integer(self.number)
        repository = ManagedRepository(self.repository.name.full_name)
        if self.head_repository not in _allowed_head_repositories(repository):
            raise ValueError(
                "The pull request head cannot be updated by the rebase app"
            )
        if (
            self.previous_base is not None
            and self.repository.name.full_name != ManagedRepository.UV_DEV.value
        ):
            raise ValueError("Previous-base rebases are restricted to uv-dev")
        check_branch(self.base_ref)
        check_branch(self.head_ref)

    @property
    def reference(self) -> PullRequestRef:
        return PullRequestRef(self.repository.name, self.number)


@dataclass(frozen=True, slots=True)
class PreparedRebase:
    source: RebaseSource
    base_sha: CommitSha

    def matches(
        self,
        pull_request: PullRequestDetails,
        *,
        head_repository: RepositoryIdentity | None = None,
    ) -> bool:
        source = self.source
        current_head_repository = pull_request.head.repository
        return (
            pull_request.reference == source.reference
            and pull_request.is_open
            and pull_request.base.repository == source.repository
            and pull_request.base.ref == source.base_ref
            and pull_request.base.sha == self.base_sha
            and current_head_repository is not None
            and current_head_repository.name == source.head_repository
            and (
                source.head_repository != source.repository.name
                or current_head_repository == source.repository
            )
            and (head_repository is None or current_head_repository == head_repository)
            and pull_request.head.ref == source.head_ref
            and pull_request.head.sha == source.head_sha
        )


@dataclass(frozen=True, slots=True)
class EmptyRebase:
    base_sha: CommitSha


@dataclass(frozen=True, slots=True)
class PersistedRebase:
    bundle: CommitBundle


type RebaseResult = EmptyRebase | PersistedRebase


@dataclass(frozen=True, slots=True)
class LoadedRebase:
    head_sha: CommitSha


@dataclass(frozen=True, slots=True)
class SkippedRebase:
    reason: str


@dataclass(frozen=True, slots=True)
class VerifiedRebaseSource:
    rebase: PreparedRebase
    head_repository: RepositoryIdentity


@dataclass(frozen=True, slots=True)
class VerifiedEmptyRebase:
    rebase: PreparedRebase
    head_repository: RepositoryIdentity


class PushOutcome(StrEnum):
    PUSHED = "pushed"
    STALE = "stale"


class CloseOutcome(StrEnum):
    CLOSED = "closed"
    STALE = "stale"


class RebaseReader(Protocol):
    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails: ...


class RebasePublisher(RebaseReader, Protocol):
    def close_pull_request(
        self, reference: PullRequestRef, *, comment: str
    ) -> None: ...


def _repository_url(repository: RepositoryName) -> str:
    return f"https://github.com/{repository}.git"


def _remote_head(
    repository: Git, remote: RepositoryName, branch: str
) -> CommitSha | None:
    reference = f"refs/heads/{branch}"
    result = repository.command(
        (
            *_GIT_CREDENTIALS,
            "ls-remote",
            "--exit-code",
            "--refs",
            _repository_url(remote),
            reference,
        ),
        check=False,
    )
    if result.returncode == 2:
        return None
    result.check_returncode()
    lines = result.stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("Git returned an unexpected number of branch heads")
    sha, separator, name = lines[0].partition("\t")
    if not separator or name != reference:
        raise ValueError("Git returned an unexpected branch head")
    return CommitSha(sha)


def _fetch_source(repository: Git, source: RebaseSource) -> bool:
    if (
        _remote_head(repository, source.head_repository, source.head_ref)
        != source.head_sha
    ):
        return False
    repository.command(
        (
            *_GIT_CREDENTIALS,
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            _repository_url(source.head_repository),
            str(source.head_sha),
        )
    )
    return repository.resolve_commit(str(source.head_sha)) == source.head_sha


def _base_head(repository: Git, base_ref: str) -> CommitSha:
    check_branch(base_ref)
    return repository.resolve_commit(f"refs/remotes/origin/{base_ref}")


def prepare_rebase(
    github: RebaseReader, repository: Git, source: RebaseSource
) -> PreparedRebase:
    rebase = PreparedRebase(source, _base_head(repository, source.base_ref))
    if not rebase.matches(github.get_pull_request(source.reference)):
        raise ValueError(
            "The pull request changed; refusing to rebase a stale revision"
        )
    if not _fetch_source(repository, source):
        raise ValueError(
            "The pull request head changed; refusing to rebase a stale revision"
        )
    if source.previous_base is not None and not repository.is_ancestor(
        source.previous_base, source.head_sha
    ):
        raise ValueError("The stacked pull request does not contain its previous base")
    repository.command(("checkout", "--detach", str(source.head_sha)))
    repository.command(("config", "user.name", "astral-automations-bot[bot]"))
    repository.command(
        (
            "config",
            "user.email",
            "305554984+astral-automations-bot[bot]@users.noreply.github.com",
        )
    )
    return rebase


def persist_rebase(
    repository: Git, base_sha: CommitSha, destination: Path
) -> RebaseResult:
    for name in ("rebase-merge", "rebase-apply"):
        path = Path(repository.output("rev-parse", "--git-path", name))
        if not path.is_absolute():
            path = repository.path / path
        if path.is_dir():
            raise ValueError("The rebase is still in progress; refusing to publish it")
    head_sha = repository.resolve_commit("HEAD")
    if not repository.is_ancestor(base_sha, head_sha):
        raise ValueError("The rebased pull request does not contain its base")
    repository.command(("diff", "--check", f"{base_sha}...{head_sha}"))
    if repository.output("status", "--porcelain=v1"):
        raise ValueError("The worktree is not clean after the rebase")
    if head_sha == base_sha:
        return EmptyRebase(base_sha)
    return PersistedRebase(
        persist_commit(repository, CommitRange(base_sha, head_sha), destination)
    )


def load_rebase(
    repository: Git,
    base_ref: str,
    commits: CommitRange,
    bundle: Path,
) -> LoadedRebase | SkippedRebase:
    if _base_head(repository, base_ref) != commits.base:
        return SkippedRebase("The pull request base changed; skipping the stale rebase")
    return LoadedRebase(load_commit(repository, bundle, commits))


def verify_rebase_source(
    github: RebaseReader, rebase: PreparedRebase
) -> VerifiedRebaseSource | SkippedRebase:
    """Pin the current repository identity before acquiring push credentials."""
    pull_request = github.get_pull_request(rebase.source.reference)
    if not rebase.matches(pull_request):
        return SkippedRebase("The pull request changed; skipping the stale rebase")
    head_repository = pull_request.head.repository
    if head_repository is None:
        return SkippedRebase("The pull request head repository was deleted")
    return VerifiedRebaseSource(rebase, head_repository)


def push_rebase(
    github: RebaseReader,
    repository: Git,
    verified: VerifiedRebaseSource,
    head_sha: CommitSha,
) -> PushOutcome:
    rebase = verified.rebase
    source = rebase.source
    if head_sha == rebase.base_sha or not repository.is_ancestor(
        rebase.base_sha, head_sha
    ):
        raise ValueError(
            "The rebased pull request must contain a nonempty commit range"
        )
    if not rebase.matches(
        github.get_pull_request(source.reference),
        head_repository=verified.head_repository,
    ):
        return PushOutcome.STALE
    if (
        _remote_head(repository, source.repository.name, source.base_ref)
        != rebase.base_sha
        or _remote_head(repository, source.head_repository, source.head_ref)
        != source.head_sha
    ):
        return PushOutcome.STALE
    repository.command(
        (
            *_GIT_CREDENTIALS,
            "push",
            f"--force-with-lease=refs/heads/{source.head_ref}:{source.head_sha}",
            _repository_url(source.head_repository),
            f"{head_sha}:refs/heads/{source.head_ref}",
        )
    )
    return PushOutcome.PUSHED


def verify_empty_rebase(
    github: RebaseReader, repository: Git, rebase: PreparedRebase
) -> VerifiedEmptyRebase | SkippedRebase:
    """Recompute the original changes without checking out the pull request."""
    source = rebase.source
    pull_request = github.get_pull_request(source.reference)
    if not rebase.matches(pull_request):
        return SkippedRebase("The pull request changed; leaving it open")
    head_repository = pull_request.head.repository
    if head_repository is None:
        return SkippedRebase("The pull request head repository was deleted")
    if _base_head(repository, source.base_ref) != rebase.base_sha:
        return SkippedRebase("The pull request base changed; leaving it open")
    if not _fetch_source(repository, source):
        return SkippedRebase("The pull request head changed; leaving it open")
    arguments = ["merge-tree", "--write-tree"]
    if source.previous_base is not None:
        if not repository.is_ancestor(source.previous_base, source.head_sha):
            raise ValueError(
                "The stacked pull request does not contain its previous base"
            )
        arguments.append(f"--merge-base={source.previous_base}")
    arguments.extend((str(rebase.base_sha), str(source.head_sha)))
    result = repository.command(arguments, check=False)
    if result.returncode == 1:
        return SkippedRebase(
            "The original pull request does not merge cleanly; leaving it open"
        )
    result.check_returncode()
    base_tree = repository.output(
        "rev-parse", "--verify", f"{rebase.base_sha}^{{tree}}"
    )
    if result.stdout.strip() != base_tree:
        return SkippedRebase(
            "The original pull request still has changes; leaving it open"
        )
    if not rebase.matches(
        github.get_pull_request(source.reference), head_repository=head_repository
    ):
        return SkippedRebase(
            "The pull request changed during verification; leaving it open"
        )
    return VerifiedEmptyRebase(rebase, head_repository)


def close_empty_rebase(
    github: RebasePublisher, verified: VerifiedEmptyRebase, *, run_id: int
) -> CloseOutcome:
    as_positive_integer(run_id)
    rebase = verified.rebase
    reference = rebase.source.reference
    if not rebase.matches(
        github.get_pull_request(reference), head_repository=verified.head_repository
    ):
        return CloseOutcome.STALE
    comment = (
        "Closing automatically because the changes in this pull request are already "
        f"present in the base branch at {rebase.base_sha}, leaving no changes after rebasing."
        f"\n\nRebase run: https://github.com/{reference.repository}/actions/runs/{run_id}"
    )
    github.close_pull_request(reference, comment=comment)
    return CloseOutcome.CLOSED
