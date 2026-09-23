"""Immutable GitHub Actions artifact identities, independent of artifact contents."""

import hashlib
import io
import logging
import re
import stat
import subprocess
import zlib
from dataclasses import dataclass
from threading import Event, Timer
from urllib.parse import quote, urlencode
from zipfile import ZIP_DEFLATED, ZIP_STORED, BadZipFile, ZipFile

from uv_automations.github import GitHub
from uv_automations.json import (
    as_array,
    as_boolean,
    as_object,
    as_positive_integer,
    as_string,
    loads,
    require_keys,
)
from uv_automations.models import (
    CommitSha,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
MAX_MANIFEST_BYTES = 64 * 1024
MAX_MANIFEST_ARCHIVE_BYTES = 128 * 1024
MANIFEST_FILENAME = "manifest.json"
MANIFEST_TIMEOUT_SECONDS = 60

logger = logging.getLogger(__name__)


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

    def is_main_dispatch(self, workflow: str) -> bool:
        return (
            self.head_repository == self.source.repository
            and self.path
            in {workflow, f"{workflow}@refs/heads/main", f"{workflow}@main"}
            and self.event == "workflow_dispatch"
            and self.branch == "main"
        )

    def is_successful_dispatch(self, workflow: str) -> bool:
        return (
            self.is_main_dispatch(workflow)
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


@dataclass(frozen=True, slots=True)
class ManifestArtifact:
    """A small immutable artifact whose attempt is supplied by its manifest.

    `workflow_sha` has the same verified root-dispatch-only meaning as ActionsRun;
    REST artifact metadata supplies the run's `head_sha`, not a generic proof of
    the workflow file revision.
    """

    repository: RepositoryIdentity
    identifier: int
    name: str
    run_identifier: int
    workflow_sha: CommitSha
    digest: str
    created_at: Timestamp
    size_in_bytes: int

    def __post_init__(self) -> None:
        as_positive_integer(self.identifier)
        as_positive_integer(self.run_identifier)
        as_positive_integer(self.size_in_bytes)
        if (
            self.size_in_bytes > MAX_MANIFEST_ARCHIVE_BYTES
            or re.fullmatch(r"sha256:[0-9a-f]{64}", self.digest) is None
            or not self.name
            or len(self.name) > 255
        ):
            raise ValueError("Invalid bounded JSON manifest artifact")

    def bind(self, source: ActionsRun) -> ArtifactIdentity:
        if (
            source.repository != self.repository
            or source.identifier != self.run_identifier
            or source.workflow_sha != self.workflow_sha
        ):
            raise ValueError("The manifest belongs to a different workflow run")
        return ArtifactIdentity(source, self.identifier, self.name, self.digest)


@dataclass(frozen=True, slots=True)
class ManifestPage:
    """Whether every matching artifact was returned and decoded successfully."""

    artifacts: tuple[ManifestArtifact, ...]
    complete: bool

    def __post_init__(self) -> None:
        if len(self.artifacts) > 100 or len(
            {artifact.identifier for artifact in self.artifacts}
        ) != len(self.artifacts):
            raise ValueError("Invalid bounded JSON manifest page")
        as_boolean(self.complete)


def decode_manifest_artifact(
    value: object, repository: RepositoryIdentity, name: str
) -> ManifestArtifact:
    data = as_object(value)
    run = as_object(data["workflow_run"])
    if (
        as_string(data["name"]) != name
        or as_boolean(data["expired"])
        or as_positive_integer(run["repository_id"]) != repository.database_id
        or as_positive_integer(run["head_repository_id"]) != repository.database_id
        or as_string(run["head_branch"]) != "main"
    ):
        raise ValueError("The manifest does not belong to the repository's main branch")
    return ManifestArtifact(
        repository,
        as_positive_integer(data["id"]),
        name,
        as_positive_integer(run["id"]),
        CommitSha(as_string(run["head_sha"])),
        as_string(data["digest"]),
        Timestamp.parse(as_string(data["created_at"])),
        as_positive_integer(data["size_in_bytes"]),
    )


def decode_json_manifest(artifact: ManifestArtifact, content: bytes) -> object:
    """Read one bounded JSON member without extracting any archive paths."""
    if (
        len(content) != artifact.size_in_bytes
        or len(content) > MAX_MANIFEST_ARCHIVE_BYTES
        or "sha256:" + hashlib.sha256(content).hexdigest() != artifact.digest
    ):
        raise ValueError(
            "The JSON manifest archive does not match its immutable digest"
        )
    try:
        with ZipFile(io.BytesIO(content)) as archive:
            members = archive.infolist()
            if len(members) != 1:
                raise ValueError(
                    "A JSON manifest artifact must contain exactly one file"
                )
            member = members[0]
            mode = member.external_attr >> 16
            if (
                member.filename != MANIFEST_FILENAME
                or member.is_dir()
                or member.flag_bits & 1
                or member.compress_type not in {ZIP_STORED, ZIP_DEFLATED}
                or member.file_size > MAX_MANIFEST_BYTES
                or (stat.S_IFMT(mode) and not stat.S_ISREG(mode))
            ):
                raise ValueError("Invalid bounded JSON manifest member")
            with archive.open(member) as source:
                value = source.read(MAX_MANIFEST_BYTES + 1)
    except (BadZipFile, zlib.error) as error:
        raise ValueError("Invalid JSON manifest archive") from error
    if len(value) > MAX_MANIFEST_BYTES:
        raise ValueError("The JSON manifest exceeds the size limit")
    return loads(value.decode("utf-8"))


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
    def list_manifest_artifacts(
        self, repository: RepositoryIdentity, name: str, *, limit: int
    ) -> ManifestPage:
        if not 1 <= limit <= 100:
            raise ValueError("Manifest discovery must fit in one API page")
        query = urlencode({"name": name, "per_page": limit})
        data = as_object(
            self._api("GET", f"repos/{repository.name}/actions/artifacts?{query}")
        )
        values = as_array(data["artifacts"])
        total = data["total_count"]
        if type(total) is not int or total < len(values) or len(values) > limit:
            raise ValueError("GitHub returned an invalid manifest artifact page")
        manifests: list[ManifestArtifact] = []
        for value in values:
            try:
                manifests.append(decode_manifest_artifact(value, repository, name))
            except (KeyError, TypeError, ValueError) as error:
                logger.info("Ignoring an unusable JSON manifest artifact: %s", error)
        return ManifestPage(
            tuple(
                sorted(
                    manifests,
                    key=lambda artifact: (artifact.created_at, artifact.identifier),
                    reverse=True,
                )
            ),
            complete=total == len(values) == len(manifests),
        )

    def get_manifest_artifact(
        self, repository: RepositoryIdentity, identifier: int, name: str
    ) -> ManifestArtifact:
        as_positive_integer(identifier)
        result = decode_manifest_artifact(
            self._api("GET", f"repos/{repository.name}/actions/artifacts/{identifier}"),
            repository,
            name,
        )
        if result.identifier != identifier:
            raise ValueError("GitHub returned a different manifest artifact")
        return result

    def find_manifest_artifact(
        self, source: ActionsRun, name: str
    ) -> ManifestArtifact | None:
        """Find one exact name in a run, leaving attempt binding to its manifest."""
        query = urlencode({"name": name, "per_page": 2})
        data = as_object(
            self._api(
                "GET",
                f"repos/{source.repository.name}/actions/runs/{source.identifier}/artifacts?{query}",
            )
        )
        values = as_array(data["artifacts"])
        total = data["total_count"]
        if type(total) is not int or total < 0:
            raise ValueError("GitHub returned an invalid manifest count")
        if not values and total == 0:
            return None
        if total != 1 or len(values) != 1:
            raise ValueError("Expected exactly one run-scoped manifest artifact")
        manifest = decode_manifest_artifact(values[0], source.repository, name)
        if (
            manifest.run_identifier != source.identifier
            or manifest.workflow_sha != source.workflow_sha
        ):
            raise ValueError("The manifest belongs to a different workflow run")
        return manifest

    def read_json_manifest(self, artifact: ManifestArtifact) -> object:
        arguments = [
            self.executable,
            "api",
            "--method",
            "GET",
            f"repos/{artifact.repository.name}/actions/artifacts/{artifact.identifier}/zip",
        ]
        timed_out = Event()
        with subprocess.Popen(
            arguments,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=self._environment(),
        ) as process:

            def expire() -> None:
                if process.poll() is None:
                    timed_out.set()
                    process.kill()

            timer = Timer(MANIFEST_TIMEOUT_SECONDS, expire)
            timer.daemon = True
            timer.start()
            try:
                if process.stdout is None:
                    raise RuntimeError("The manifest downloader has no output stream")
                content = process.stdout.read(MAX_MANIFEST_ARCHIVE_BYTES + 1)
                if len(content) > MAX_MANIFEST_ARCHIVE_BYTES:
                    raise ValueError("The JSON manifest archive exceeds the size limit")
                returncode = process.wait()
            finally:
                timer.cancel()
                if process.poll() is None:
                    process.kill()
                process.wait()
        if timed_out.is_set():
            raise subprocess.TimeoutExpired(arguments, MANIFEST_TIMEOUT_SECONDS)
        if returncode:
            raise subprocess.CalledProcessError(returncode, arguments)
        return decode_json_manifest(artifact, content)

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
