import io
import os
import subprocess
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from uv_automations.artifacts import (
    CommitBundle,
    CommitRange,
    load_commit,
    persist_commit,
)
from uv_automations.cli import main
from uv_automations.git import Git
from uv_automations.models import CommitSha


def create_repository(path: Path) -> Git:
    path.mkdir()
    repository = Git(path)
    repository.command(("init", "--quiet", "--initial-branch", "main"))
    repository.command(("config", "user.name", "Automation test"))
    repository.command(("config", "user.email", "automation@example.com"))
    return repository


def commit_file(repository: Git, name: str, content: str) -> CommitSha:
    (repository.path / name).write_text(content, encoding="utf-8")
    repository.command(("add", "--", name))
    repository.command(("commit", "--quiet", "--message", name))
    return repository.resolve_commit("HEAD")


class CommitArtifactTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        directory = TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.source = create_repository(self.root / "source")
        self.base = commit_file(self.source, "base.txt", "base\n")
        self.consumer = create_repository(self.root / "consumer")
        self.consumer.command(
            ("fetch", "--quiet", "--no-tags", str(self.source.path), str(self.base))
        )
        self.consumer.command(("checkout", "--quiet", "--detach", str(self.base)))
        self.head = commit_file(self.source, "change.txt", "change\n")
        self.commits = CommitRange(self.base, self.head)
        self.bundle = self.root / "commit.bundle"

    def test_round_trip_preserves_consumer_state(self) -> None:
        initial_head = self.consumer.resolve_commit("HEAD")
        initial_refs = self.consumer.output("show-ref", "--head")
        fetch_head = self.consumer.path / ".git" / "FETCH_HEAD"
        initial_fetch_head = fetch_head.read_bytes()
        self.assertNotEqual(
            self.consumer.command(
                ("cat-file", "-e", str(self.head)), check=False
            ).returncode,
            0,
        )

        self.assertEqual(
            persist_commit(self.source, self.commits, self.bundle),
            CommitBundle(self.bundle, self.commits),
        )
        self.assertEqual(
            self.source.output("bundle", "list-heads", str(self.bundle)),
            f"{self.head} {self.commits.reference}",
        )
        self.assertEqual(
            self.source.command(
                ("show-ref", "--verify", "--quiet", self.commits.reference), check=False
            ).returncode,
            1,
        )

        self.assertEqual(
            load_commit(self.consumer, self.bundle, self.commits), self.head
        )
        self.assertEqual(self.consumer.resolve_commit("HEAD"), initial_head)
        self.assertEqual(self.consumer.output("show-ref", "--head"), initial_refs)
        self.assertEqual(fetch_head.read_bytes(), initial_fetch_head)
        self.assertFalse((self.consumer.path / "change.txt").exists())

    def test_empty_range_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "extend its base"):
            CommitRange(self.base, self.base)

    def test_unrelated_history_is_rejected(self) -> None:
        tree = self.source.output("rev-parse", f"{self.base}^{{tree}}")
        unrelated = CommitSha(
            self.source.output("commit-tree", tree, "-m", "unrelated")
        )
        with self.assertRaisesRegex(ValueError, "trusted base"):
            persist_commit(self.source, CommitRange(unrelated, self.head), self.bundle)
        self.assertFalse(self.bundle.exists())

    def test_tag_object_is_not_a_commit(self) -> None:
        self.source.command(("tag", "-a", "annotated", "-m", "tag", str(self.head)))
        tag = CommitSha(self.source.output("rev-parse", "refs/tags/annotated"))
        with self.assertRaisesRegex(ValueError, "Expected a Git commit"):
            persist_commit(self.source, CommitRange(self.base, tag), self.bundle)

    def test_replacement_objects_cannot_change_ancestry(self) -> None:
        tree = self.source.output("rev-parse", f"{self.base}^{{tree}}")
        unrelated = CommitSha(
            self.source.output("commit-tree", tree, "-m", "unrelated")
        )
        self.source.command(("update-ref", f"refs/replace/{unrelated}", str(self.head)))
        self.assertFalse(self.source.is_ancestor(self.base, unrelated))
        with self.assertRaisesRegex(ValueError, "trusted base"):
            persist_commit(self.source, CommitRange(self.base, unrelated), self.bundle)

    def test_existing_bundle_is_not_replaced(self) -> None:
        self.bundle.write_bytes(b"existing")
        with self.assertRaises(FileExistsError):
            persist_commit(self.source, self.commits, self.bundle)
        self.assertEqual(self.bundle.read_bytes(), b"existing")
        self.assertEqual(
            self.source.command(
                ("show-ref", "--verify", "--quiet", self.commits.reference), check=False
            ).returncode,
            1,
        )

    def test_existing_symlink_is_not_followed(self) -> None:
        target = self.root / "target"
        target.write_bytes(b"existing")
        self.bundle.symlink_to(target)
        with self.assertRaises(FileExistsError):
            persist_commit(self.source, self.commits, self.bundle)
        self.assertTrue(self.bundle.is_symlink())
        self.assertEqual(target.read_bytes(), b"existing")

    def test_existing_temporary_ref_is_not_replaced_or_deleted(self) -> None:
        self.source.command(("update-ref", self.commits.reference, str(self.head)))
        with self.assertRaises(subprocess.CalledProcessError):
            persist_commit(self.source, self.commits, self.bundle)
        self.assertEqual(self.source.resolve_commit(self.commits.reference), self.head)
        self.assertFalse(self.bundle.exists())

    def test_import_rejects_a_different_advertised_commit(self) -> None:
        persist_commit(self.source, self.commits, self.bundle)
        other = commit_file(self.source, "other.txt", "other\n")
        with self.assertRaisesRegex(ValueError, "exactly the expected commit"):
            load_commit(self.consumer, self.bundle, CommitRange(self.base, other))

    def test_import_rejects_extra_refs(self) -> None:
        self.source.command(("update-ref", self.commits.reference, str(self.head)))
        self.source.command(("update-ref", "refs/heads/extra", str(self.head)))
        self.source.command(
            (
                "bundle",
                "create",
                str(self.bundle),
                f"{self.base}..{self.commits.reference}",
                "refs/heads/extra",
            )
        )
        with self.assertRaisesRegex(ValueError, "exactly the expected commit"):
            load_commit(self.consumer, self.bundle, self.commits)

    def test_import_rejects_non_regular_files(self) -> None:
        persist_commit(self.source, self.commits, self.bundle)
        link = self.root / "link.bundle"
        link.symlink_to(self.bundle)
        for path in (link, self.root / "missing.bundle", self.root):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, "regular"):
                load_commit(self.consumer, path, self.commits)

    def test_import_requires_a_known_base(self) -> None:
        persist_commit(self.source, self.commits, self.bundle)
        empty = create_repository(self.root / "empty")
        with self.assertRaises(subprocess.CalledProcessError):
            load_commit(empty, self.bundle, self.commits)

    def test_git_ignores_inherited_repository_state(self) -> None:
        with patch.dict(
            os.environ,
            {
                "GIT_DIR": str(self.consumer.path / ".git"),
                "GIT_WORK_TREE": str(self.consumer.path),
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "core.hooksPath",
                "GIT_CONFIG_VALUE_0": str(self.root / "hooks"),
            },
        ):
            self.assertEqual(self.source.resolve_commit("HEAD"), self.head)
            persist_commit(self.source, self.commits, self.bundle)

    def test_command_line_round_trip(self) -> None:
        output = self.root / "github-output"
        with patch("uv_automations.cli.logging.basicConfig"):
            main(
                [
                    "commits",
                    "persist",
                    "--repository",
                    str(self.source.path),
                    "--base",
                    str(self.base),
                    "--destination",
                    str(self.bundle),
                    "--github-output",
                    str(output),
                ]
            )
        self.assertEqual(
            output.read_text(encoding="utf-8"),
            f"head-sha={self.head}\npath={self.bundle}\n",
        )

        result = io.StringIO()
        with patch("uv_automations.cli.logging.basicConfig"), redirect_stdout(result):
            main(
                [
                    "commits",
                    "load",
                    "--repository",
                    str(self.consumer.path),
                    "--base",
                    str(self.base),
                    "--head",
                    str(self.head),
                    "--bundle",
                    str(self.bundle),
                ]
            )
        self.assertEqual(result.getvalue(), f'{{"head-sha":"{self.head}"}}\n')
