import base64
import hashlib
import io
import os
import stat
import subprocess
import sys
import unittest
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

from uv_automations.github_actions import (
    MAX_MANIFEST_ARCHIVE_BYTES,
    MAX_MANIFEST_BYTES,
    ActionsGitHub,
    ActionsRun,
    ManifestArtifact,
    decode_json_manifest,
    decode_manifest_artifact,
)
from uv_automations.json import as_object
from uv_automations.models import (
    CommitSha,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

REPOSITORY = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
SOURCE = ActionsRun(REPOSITORY, 123, 2, CommitSha("a" * 40))
CREATED_AT = Timestamp.parse("2026-09-08T12:00:00Z")
NAME = "pull-request-comments-checkpoint-456"


def archive_contents(
    entries: tuple[tuple[str | ZipInfo, bytes], ...] = (
        ("manifest.json", b'{"value":1}'),
    ),
) -> bytes:
    output = io.BytesIO()
    with ZipFile(output, "w", compression=ZIP_DEFLATED) as archive:
        for name, content in entries:
            archive.writestr(name, content)
    return output.getvalue()


def manifest_artifact(content: bytes) -> ManifestArtifact:
    return ManifestArtifact(
        REPOSITORY,
        789,
        NAME,
        SOURCE.identifier,
        SOURCE.workflow_sha,
        "sha256:" + hashlib.sha256(content).hexdigest(),
        CREATED_AT,
        len(content),
    )


def artifact_payload(content: bytes) -> dict[str, object]:
    artifact = manifest_artifact(content)
    return {
        "id": artifact.identifier,
        "name": artifact.name,
        "expired": False,
        "size_in_bytes": artifact.size_in_bytes,
        "digest": artifact.digest,
        "created_at": str(artifact.created_at),
        "workflow_run": {
            "id": artifact.run_identifier,
            "repository_id": REPOSITORY.database_id,
            "head_repository_id": REPOSITORY.database_id,
            "head_sha": str(artifact.workflow_sha),
            "head_branch": "main",
        },
    }


def downloader(path: Path, body: str) -> Path:
    path.write_text(f"#!{sys.executable}\n{body}\n", encoding="utf-8")
    path.chmod(0o755)
    return path


class ActionManifestTests(unittest.TestCase):
    def test_manifest_identity_can_only_bind_its_own_workflow_run(self) -> None:
        content = archive_contents()
        artifact = decode_manifest_artifact(artifact_payload(content), REPOSITORY, NAME)
        self.assertEqual(artifact.bind(SOURCE).source, SOURCE)
        self.assertEqual(artifact.bind(replace(SOURCE, attempt=3)).source.attempt, 3)
        for changed in (
            replace(SOURCE, identifier=1),
            replace(SOURCE, workflow_sha=CommitSha("b" * 40)),
            replace(SOURCE, repository=replace(REPOSITORY, database_id=1)),
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                artifact.bind(changed)

    def test_manifest_size_requires_an_exact_positive_integer(self) -> None:
        content = archive_contents()
        artifact = manifest_artifact(content)
        for size in (True, float(len(content)), 1.5, 0, -1):
            with self.subTest(size=size), self.assertRaises(ValueError):
                replace(artifact, size_in_bytes=size)

    def test_manifest_is_one_digest_verified_bounded_json_member(self) -> None:
        content = archive_contents()
        artifact = manifest_artifact(content)
        self.assertEqual(decode_json_manifest(artifact, content), {"value": 1})
        with self.assertRaisesRegex(ValueError, "digest"):
            decode_json_manifest(artifact, content[:-1] + b"x")
        symlink = ZipInfo("manifest.json")
        symlink.create_system = 3
        symlink.external_attr = (stat.S_IFLNK | 0o777) << 16
        for entries in (
            (("../manifest.json", b"{}"),),
            (("manifest.json", b"{}"), ("extra.json", b"{}")),
            ((symlink, b"{}"),),
            (("manifest.json", b" " * (MAX_MANIFEST_BYTES + 1)),),
            (("manifest.json", b'{"duplicate":1,"duplicate":2}'),),
        ):
            candidate = archive_contents(entries)
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                decode_json_manifest(manifest_artifact(candidate), candidate)

    def test_discovery_is_scoped_by_exact_name_and_one_page(self) -> None:
        content = archive_contents()
        older = artifact_payload(content)
        newer = {**older, "id": 790, "created_at": "2026-09-08T12:01:00Z"}
        unusable = {**older, "id": 791, "expired": True}
        with patch.object(
            ActionsGitHub,
            "_api",
            return_value={"artifacts": [older, unusable, newer], "total_count": 3},
        ) as api:
            page = ActionsGitHub().list_manifest_artifacts(REPOSITORY, NAME, limit=20)
        self.assertEqual(
            [artifact.identifier for artifact in page.artifacts], [790, 789]
        )
        self.assertFalse(page.complete)
        self.assertEqual(
            api.call_args.args,
            (
                "GET",
                (
                    "repos/astral-sh/uv-dev/actions/artifacts?"
                    "name=pull-request-comments-checkpoint-456&per_page=20"
                ),
            ),
        )
        with patch.object(
            ActionsGitHub,
            "_api",
            return_value={"artifacts": [newer], "total_count": 2},
        ):
            incomplete = ActionsGitHub().list_manifest_artifacts(
                REPOSITORY, NAME, limit=1
            )
        self.assertFalse(incomplete.complete)

    def test_run_scoped_lookup_does_not_invent_a_producer_attempt(self) -> None:
        content = archive_contents()
        value = artifact_payload(content)
        with patch.object(
            ActionsGitHub,
            "_api",
            return_value={"artifacts": [value], "total_count": 1},
        ) as api:
            manifest = ActionsGitHub().find_manifest_artifact(
                replace(SOURCE, attempt=3), NAME
            )
        self.assertEqual(manifest, manifest_artifact(content))
        api.assert_called_once_with(
            "GET",
            "repos/astral-sh/uv-dev/actions/runs/123/artifacts?"
            "name=pull-request-comments-checkpoint-456&per_page=2",
        )
        with patch.object(
            ActionsGitHub,
            "_api",
            return_value={"artifacts": [], "total_count": 0},
        ):
            self.assertIsNone(ActionsGitHub().find_manifest_artifact(SOURCE, NAME))
        for response in (
            {"artifacts": [], "total_count": False},
            {"artifacts": [value, value], "total_count": 2},
            {
                "artifacts": [
                    {
                        **value,
                        "workflow_run": {**as_object(value["workflow_run"]), "id": 999},
                    }
                ],
                "total_count": 1,
            },
        ):
            with (
                self.subTest(response=response),
                patch.object(ActionsGitHub, "_api", return_value=response),
                self.assertRaises(ValueError),
            ):
                ActionsGitHub().find_manifest_artifact(SOURCE, NAME)

    def test_binary_transport_uses_the_selected_read_credential(self) -> None:
        content = archive_contents()
        encoded = base64.b64encode(content).decode()
        with TemporaryDirectory() as directory:
            executable = downloader(
                Path(directory) / "gh-test",
                "import base64, os, sys\n"
                "if os.environ.get('GH_TOKEN') != 'read-token': sys.exit(2)\n"
                f"sys.stdout.buffer.write(base64.b64decode({encoded!r}))",
            )
            with patch.dict(
                os.environ, {"GH_TOKEN": "writer-token", "GH_READ_TOKEN": "read-token"}
            ):
                value = ActionsGitHub(
                    executable=str(executable), token_variable="GH_READ_TOKEN"
                ).read_json_manifest(manifest_artifact(content))
        self.assertEqual(value, {"value": 1})

    def test_binary_transport_enforces_its_byte_and_time_limits(self) -> None:
        content = archive_contents()
        with TemporaryDirectory() as directory:
            executable = downloader(
                Path(directory) / "oversized",
                f"import sys\nsys.stdout.buffer.write(b'x' * {MAX_MANIFEST_ARCHIVE_BYTES + 1})",
            )
            with self.assertRaisesRegex(ValueError, "size limit"):
                ActionsGitHub(executable=str(executable)).read_json_manifest(
                    manifest_artifact(content)
                )
            executable = downloader(
                Path(directory) / "slow", "import time\ntime.sleep(2)"
            )
            with (
                patch("uv_automations.github_actions.MANIFEST_TIMEOUT_SECONDS", 0.01),
                self.assertRaises(subprocess.TimeoutExpired),
            ):
                ActionsGitHub(executable=str(executable)).read_json_manifest(
                    manifest_artifact(content)
                )
