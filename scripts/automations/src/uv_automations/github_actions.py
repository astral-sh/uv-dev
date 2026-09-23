"""Immutable GitHub Actions artifact identities, independent of artifact contents."""

import re
from dataclasses import dataclass
from urllib.parse import quote, urlencode

from uv_automations.github import GitHub
from uv_automations.json import (
    as_array,
    as_boolean,
    as_object,
    as_positive_integer,
    as_string,
    require_keys,
)
from uv_automations.models import (
    CommitSha,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
DISPATCH_API_VERSION = "2026-03-10"


@dataclass(frozen=True, slots=True)
class WorkflowDispatch:
    repository: RepositoryIdentity
    identifier: int

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)

    @property
    def url(self) -> str:
        return (
            f"https://github.com/{self.repository.name}/actions/runs/{self.identifier}"
        )


@dataclass(frozen=True, slots=True)
class ActionsRun:
    """Expected identity for this library's trusted root-dispatch contract.

    Callers supply `workflow_sha` from `GITHUB_WORKFLOW_SHA`. The REST decoder
    initially fills it from the run's `head_sha`; those are the same revision
    only for the subsequently verified root `workflow_dispatch` on `main`.
    This field is not generic workflow-file provenance for PR or reusable runs.
    """

    repository: RepositoryIdentity
    identifier: int
    attempt: int
    workflow_sha: CommitSha

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        as_positive_integer(self.attempt)

    def same_run(self, other: ActionsRun) -> bool:
        """Compare immutable run identity, independently of a retry attempt."""
        return (
            self.repository == other.repository
            and self.identifier == other.identifier
            and self.workflow_sha == other.workflow_sha
        )

    def to_json(self) -> dict[str, object]:
        return {
            "repository": str(self.repository.name),
            "repository_id": self.repository.database_id,
            "run_id": self.identifier,
            "run_attempt": self.attempt,
            "workflow_sha": str(self.workflow_sha),
        }

    @classmethod
    def from_json(cls, value: object) -> ActionsRun:
        data = as_object(value)
        require_keys(
            data,
            {"repository", "repository_id", "run_id", "run_attempt", "workflow_sha"},
        )
        return cls(
            RepositoryIdentity(
                RepositoryName(as_string(data["repository"])),
                as_positive_integer(data["repository_id"]),
            ),
            as_positive_integer(data["run_id"]),
            as_positive_integer(data["run_attempt"]),
            CommitSha(as_string(data["workflow_sha"])),
        )


@dataclass(frozen=True, slots=True)
class WorkflowRun:
    source: ActionsRun
    head_repository: RepositoryIdentity | None
    path: str
    event: str
    branch: str
    status: str
    conclusion: str | None
    started_at: Timestamp

    def is_successful_dispatch(self, workflow: str) -> bool:
        return (
            self.head_repository == self.source.repository
            and self.path
            in {workflow, f"{workflow}@refs/heads/main", f"{workflow}@main"}
            and self.event == "workflow_dispatch"
            and self.branch == "main"
            and self.status == "completed"
            and self.conclusion == "success"
        )


@dataclass(frozen=True, slots=True)
class ArtifactIdentity:
    """An immutable artifact with a caller-validated producer attempt.

    GitHub's artifact metadata identifies the workflow run, not its retry
    attempt. A consumer binds `source.attempt` through its trusted producer's
    attempt-scoped name or manifest and verifies that exact workflow attempt.
    """

    source: ActionsRun
    identifier: int
    name: str
    digest: str

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        if re.fullmatch(r"sha256:[0-9a-f]{64}", self.digest) is None:
            raise ValueError("Expected an immutable artifact SHA-256 digest")
        if not self.name or len(self.name) > 255:
            raise ValueError("Invalid artifact name")

    def to_json(self) -> dict[str, object]:
        return {
            "source": self.source.to_json(),
            "artifact_id": self.identifier,
            "name": self.name,
            "digest": self.digest,
        }

    @classmethod
    def from_json(cls, value: object) -> ArtifactIdentity:
        data = as_object(value)
        require_keys(data, {"source", "artifact_id", "name", "digest"})
        return cls(
            ActionsRun.from_json(data["source"]),
            as_positive_integer(data["artifact_id"]),
            as_string(data["name"]),
            as_string(data["digest"]),
        )


def _repository(value: object) -> RepositoryIdentity:
    data = as_object(value)
    return RepositoryIdentity(
        RepositoryName(as_string(data["full_name"])), as_positive_integer(data["id"])
    )


def decode_workflow_run(value: object) -> WorkflowRun:
    """Decode REST run metadata without asserting workflow-file provenance.

    A consumer must check the root main-dispatch contract and, for its current
    run, compare the result with its independently supplied Actions identity.
    In other event types, REST `head_sha` can identify candidate code instead.
    """
    data = as_object(value)
    head_repository = data["head_repository"]
    conclusion = data["conclusion"]
    return WorkflowRun(
        source=ActionsRun(
            _repository(data["repository"]),
            as_positive_integer(data["id"]),
            as_positive_integer(data["run_attempt"]),
            CommitSha(as_string(data["head_sha"])),
        ),
        head_repository=(
            _repository(head_repository) if head_repository is not None else None
        ),
        path=as_string(data["path"]),
        event=as_string(data["event"]),
        branch=as_string(data["head_branch"]),
        status=as_string(data["status"]),
        conclusion=as_string(conclusion) if conclusion is not None else None,
        started_at=Timestamp.parse(as_string(data["run_started_at"])),
    )


def decode_artifact(value: object, source: ActionsRun, name: str) -> ArtifactIdentity:
    """Check REST metadata while preserving the caller-supplied attempt.

    This does not attest `source.attempt`; the caller's trusted producer
    contract must establish that binding before trusting artifact contents.
    """
    data = as_object(value)
    run = as_object(data["workflow_run"])
    size = as_positive_integer(data["size_in_bytes"])
    if (
        as_string(data["name"]) != name
        or as_boolean(data["expired"])
        or size > MAX_ARTIFACT_BYTES
        or as_positive_integer(run["id"]) != source.identifier
        or as_positive_integer(run["repository_id"]) != source.repository.database_id
        or as_positive_integer(run["head_repository_id"])
        != source.repository.database_id
        or CommitSha(as_string(run["head_sha"])) != source.workflow_sha
        or as_string(run["head_branch"]) != "main"
    ):
        raise ValueError("The artifact does not match its trusted workflow run")
    return ArtifactIdentity(
        source,
        as_positive_integer(data["id"]),
        name,
        as_string(data["digest"]),
    )


class ActionsGitHub(GitHub):
    def dispatch_main_workflow(
        self,
        repository: RepositoryIdentity,
        workflow: str,
        inputs: dict[str, str],
    ) -> WorkflowDispatch:
        """Dispatch a named trusted workflow and retain its returned run identity."""
        if re.fullmatch(r"[A-Za-z0-9_-]+\.ya?ml", workflow) is None:
            raise ValueError("Expected a workflow file name")
        if len(inputs) > 25 or any(
            not isinstance(key, str) or not isinstance(value, str)
            for key, value in inputs.items()
        ):
            raise ValueError("Invalid workflow dispatch inputs")
        if _repository(self._api("GET", f"repos/{repository.name}")) != repository:
            raise ValueError("The workflow dispatch repository identity changed")
        data = as_object(
            self._command(
                [
                    "api",
                    "--method",
                    "POST",
                    "--header",
                    f"X-GitHub-Api-Version: {DISPATCH_API_VERSION}",
                    f"repos/{repository.name}/actions/workflows/{quote(workflow, safe='')}/dispatches",
                    "--input",
                    "-",
                ],
                payload={"ref": "main", "inputs": inputs},
            )
        )
        result = WorkflowDispatch(
            repository, as_positive_integer(data["workflow_run_id"])
        )
        if (
            as_string(data["html_url"]) != result.url
            or as_string(data["run_url"])
            != f"https://api.github.com/repos/{repository.name}/actions/runs/{result.identifier}"
        ):
            raise ValueError("GitHub returned a different workflow dispatch")
        return result

    def list_successful_workflow_runs(
        self, repository: RepositoryName, workflow: str, *, limit: int
    ) -> tuple[WorkflowRun, ...]:
        if not 1 <= limit <= 100:
            raise ValueError("Workflow-run discovery must fit in one API page")
        query = urlencode(
            {
                "event": "workflow_dispatch",
                "status": "success",
                "branch": "main",
                "per_page": limit,
            }
        )
        data = as_object(
            self._api(
                "GET",
                f"repos/{repository}/actions/workflows/{quote(workflow, safe='')}/runs?{query}",
            )
        )
        runs = as_array(data["workflow_runs"])
        if len(runs) > limit:
            raise ValueError("GitHub returned too many workflow runs")
        return tuple(decode_workflow_run(value) for value in runs)

    def read_workflow_run(
        self, repository: RepositoryIdentity, identifier: int, attempt: int
    ) -> WorkflowRun:
        as_positive_integer(identifier)
        as_positive_integer(attempt)
        result = decode_workflow_run(
            self._api(
                "GET",
                f"repos/{repository.name}/actions/runs/{identifier}/attempts/{attempt}",
            )
        )
        if (
            result.source.repository != repository
            or result.source.identifier != identifier
            or result.source.attempt != attempt
        ):
            raise ValueError("GitHub returned a different workflow run attempt")
        return result

    def get_workflow_run(self, source: ActionsRun) -> WorkflowRun:
        """Compare REST identity with the caller's independently trusted values."""
        result = self.read_workflow_run(
            source.repository, source.identifier, source.attempt
        )
        if result.source != source:
            raise ValueError("GitHub returned a different workflow run attempt")
        return result

    def find_artifact(self, source: ActionsRun, name: str) -> ArtifactIdentity | None:
        query = urlencode({"name": name, "per_page": 2})
        data = as_object(
            self._api(
                "GET",
                f"repos/{source.repository.name}/actions/runs/{source.identifier}/artifacts?{query}",
            )
        )
        values = as_array(data["artifacts"])
        if not values:
            return None
        if data["total_count"] != 1 or len(values) != 1:
            raise ValueError("Expected exactly one matching workflow-run artifact")
        return decode_artifact(values[0], source, name)

    def get_artifact(
        self, source: ActionsRun, identifier: int, name: str
    ) -> ArtifactIdentity:
        as_positive_integer(identifier)
        artifact = decode_artifact(
            self._api(
                "GET",
                f"repos/{source.repository.name}/actions/artifacts/{identifier}",
            ),
            source,
            name,
        )
        if artifact.identifier != identifier:
            raise ValueError("GitHub returned a different artifact")
        return artifact
