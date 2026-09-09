import errno
import hashlib
import os
import shlex
import subprocess
import unittest
from collections.abc import Sequence
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from uv_automations.artifacts import CommitRange, load_commit, persist_commit
from uv_automations.checkouts import inspect_candidate
from uv_automations.git import Git
from uv_automations.models import CommitSha

HAS_SECURE_DIRECTORY_DESCRIPTORS = (
    os.open in os.supports_dir_fd
    and os.listdir in os.supports_fd
    and bool(getattr(os, "O_DIRECTORY", 0))
    and bool(getattr(os, "O_NOFOLLOW", 0))
    and bool(getattr(os, "O_NONBLOCK", 0))
)


def create_repository(path: Path) -> Git:
    path.mkdir()
    repository = Git(path)
    repository.command(("init", "--quiet", "--initial-branch", "main"))
    repository.command(("config", "user.name", "Automation test"))
    repository.command(("config", "user.email", "automation@example.com"))
    return repository


def commit_file(repository: Git, name: str, content: str) -> CommitSha:
    path = repository.path / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    repository.command(("add", "--", name))
    repository.command(("commit", "--quiet", "--message", name))
    return repository.resolve_commit("HEAD")


def marker_script(path: Path, marker: Path, *, output: str = "") -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"#!/bin/sh\nprintf '%s\\n' triggered >> {shlex.quote(str(marker))}\n{output}\n",
        encoding="utf-8",
    )
    path.chmod(0o755)
    return path


@unittest.skipUnless(
    HAS_SECURE_DIRECTORY_DESCRIPTORS,
    "requires no-follow directory descriptors",
)
class CandidateInspectionTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        directory = TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.scratch = self.root / "scratch"
        self.scratch.mkdir()
        self.source = create_repository(self.root / "source")
        self.base = commit_file(self.source, "tracked.txt", "A\n")

    def test_clean_detached_head_uses_private_bare_metadata(self) -> None:
        self.source.command(("checkout", "--quiet", "--detach", str(self.base)))
        head = commit_file(self.source, "change.txt", "change\n")
        source_index = self.source.path / ".git" / "index"
        original_index = source_index.read_bytes()
        original_refs = self.source.output("show-ref", "--head")
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            private = candidate.repository.path
            self.assertTrue(private.is_relative_to(self.scratch))
            self.assertFalse(private.is_relative_to(self.source.path))
            self.assertEqual(
                candidate.repository.output("rev-parse", "--is-bare-repository"), "true"
            )
            self.assertEqual(candidate.head, head)
            self.assertTrue(candidate.repository.is_ancestor(self.base, head))
            self.assertFalse((private / "objects" / "info" / "alternates").exists())
            candidate.require_clean()
            candidate.require_clean()
        self.assertFalse(private.exists())
        self.assertEqual(source_index.read_bytes(), original_index)
        self.assertEqual(self.source.output("show-ref", "--head"), original_refs)

    def test_bundle_round_trip_uses_only_cloned_objects(self) -> None:
        consumer = create_repository(self.root / "consumer")
        consumer.command(
            ("fetch", "--quiet", "--no-tags", str(self.source.path), str(self.base))
        )
        consumer.command(("checkout", "--quiet", "--detach", str(self.base)))
        self.source.command(("checkout", "--quiet", "--detach", str(self.base)))
        head = commit_file(self.source, "change.txt", "change\n")
        commits = CommitRange(self.base, head)
        destination = self.root / "commit.bundle"
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            persist_commit(candidate.repository, commits, destination)
        self.assertEqual(load_commit(consumer, destination, commits), head)
        self.assertEqual(consumer.resolve_commit("HEAD"), self.base)
        self.assertFalse((consumer.path / "change.txt").exists())

    def test_swapped_git_directory_cannot_redirect_transport(self) -> None:
        outside = create_repository(self.root / "outside")
        outside.command(
            ("fetch", "--quiet", "--no-tags", str(self.source.path), str(self.base))
        )
        outside.command(("checkout", "--quiet", "--detach", str(self.base)))
        outside.command(("commit", "--quiet", "--allow-empty", "-m", "outside"))
        outside_head = outside.resolve_commit("HEAD")
        original_command = Git.command
        cloned = False

        def command(
            repository: Git,
            arguments: Sequence[str],
            *,
            check: bool = True,
            input: str | None = None,
        ) -> subprocess.CompletedProcess[str]:
            nonlocal cloned
            self.assertNotEqual(repository.path, self.source.path)
            if "clone" in arguments:
                self.assertFalse(cloned)
                self.assertTrue(Path(arguments[-2]).is_relative_to(self.scratch))
                (self.source.path / ".git").rename(self.root / "held.git")
                (self.source.path / ".git").symlink_to(
                    outside.path / ".git", target_is_directory=True
                )
                cloned = True
            return original_command(repository, arguments, check=check, input=input)

        with (
            patch.object(Git, "command", command),
            inspect_candidate(
                self.source, base=self.base, scratch=self.scratch
            ) as candidate,
        ):
            candidate.require_clean()
            self.assertEqual(candidate.head, self.base)
            self.assertNotEqual(
                candidate.repository.command(
                    ("cat-file", "-e", str(outside_head)), check=False
                ).returncode,
                0,
            )
        self.assertTrue(cloned)

    def test_swapped_checkout_directory_cannot_redirect_transport(self) -> None:
        outside = create_repository(self.root / "outside")
        outside.command(
            ("fetch", "--quiet", "--no-tags", str(self.source.path), str(self.base))
        )
        outside.command(("checkout", "--quiet", "--detach", str(self.base)))
        outside.command(("commit", "--quiet", "--allow-empty", "-m", "outside"))
        original_command = Git.command

        def command(
            repository: Git,
            arguments: Sequence[str],
            *,
            check: bool = True,
            input: str | None = None,
        ) -> subprocess.CompletedProcess[str]:
            if "clone" in arguments:
                self.assertTrue(Path(arguments[-2]).is_relative_to(self.scratch))
                self.source.path.rename(self.root / "held-source")
                self.source.path.symlink_to(outside.path, target_is_directory=True)
            return original_command(repository, arguments, check=check, input=input)

        with (
            patch.object(Git, "command", command),
            inspect_candidate(
                self.source, base=self.base, scratch=self.scratch
            ) as candidate,
        ):
            candidate.require_clean()
            self.assertEqual(candidate.head, self.base)

    def test_alternate_object_stores_are_rejected(self) -> None:
        outside = create_repository(self.root / "outside")
        outside.command(
            ("fetch", "--quiet", "--no-tags", str(self.source.path), str(self.base))
        )
        outside.command(("checkout", "--quiet", "--detach", str(self.base)))
        marker_commit = commit_file(outside, "marker.txt", "outside-only marker\n")
        marker_object = outside.output("rev-parse", f"{marker_commit}:marker.txt")
        outside.command(("rm", "--quiet", "--", "marker.txt"))
        outside.command(("commit", "--quiet", "-m", "return to original tree"))
        outside_head = outside.resolve_commit("HEAD")
        objects = self.source.path / ".git" / "objects"
        self.assertFalse((objects / marker_object[:2] / marker_object[2:]).exists())
        info = objects / "info"
        info.mkdir(exist_ok=True)
        (info / "alternates").write_text(
            f"{outside.path / '.git' / 'objects'}\n", encoding="utf-8"
        )
        (self.source.path / ".git" / "refs" / "heads" / "main").write_text(
            f"{outside_head}\n", encoding="ascii"
        )
        with (
            self.assertRaisesRegex(ValueError, "alternate object stores"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an outside object store")

    def test_packed_head_reference_is_supported(self) -> None:
        self.source.command(("pack-refs", "--all", "--prune"))
        self.assertFalse(
            (self.source.path / ".git" / "refs" / "heads" / "main").exists()
        )
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            self.assertEqual(candidate.head, self.base)

    def test_duplicate_packed_head_is_rejected(self) -> None:
        self.source.command(("pack-refs", "--all", "--prune"))
        packed = self.source.path / ".git" / "packed-refs"
        with packed.open("a", encoding="ascii") as output:
            output.write(f"{self.base} refs/heads/main\n")
        with (
            self.assertRaisesRegex(ValueError, "exactly one packed reference"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted duplicate packed HEAD entries")

    def test_symbolic_loose_head_reference_is_rejected(self) -> None:
        (self.source.path / ".git" / "refs" / "heads" / "main").write_text(
            "ref: refs/heads/other\n", encoding="ascii"
        )
        with (
            self.assertRaisesRegex(ValueError, "full Git commit SHA"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed an additional symbolic reference")

    def test_packed_objects_do_not_require_source_caches(self) -> None:
        self.source.command(("repack", "-adb"))
        self.source.command(("multi-pack-index", "write"))
        self.source.command(("commit-graph", "write", "--reachable"))
        objects = self.source.path / ".git" / "objects"
        self.assertTrue(tuple((objects / "pack").glob("*.pack")))
        self.assertTrue((objects / "pack" / "multi-pack-index").exists())
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            snapshot_objects = (
                candidate.repository.path.parent / "source.git" / "objects"
            )
            self.assertFalse((snapshot_objects / "pack" / "multi-pack-index").exists())
            self.assertFalse((snapshot_objects / "info" / "commit-graph").exists())
            self.assertTrue(
                all(
                    path.suffix in {".pack", ".idx"}
                    for path in (snapshot_objects / "pack").iterdir()
                )
            )

    def test_symlinked_reference_directory_is_rejected(self) -> None:
        heads = self.source.path / ".git" / "refs" / "heads"
        saved = self.root / "saved-heads"
        heads.rename(saved)
        heads.symlink_to(saved, target_is_directory=True)
        with (
            self.assertRaisesRegex(ValueError, "ordinary checkout directories"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed a reference-directory symlink")

    def test_symlinked_object_store_is_rejected(self) -> None:
        objects = self.source.path / ".git" / "objects"
        saved = self.root / "saved-objects"
        objects.rename(saved)
        objects.symlink_to(saved, target_is_directory=True)
        with (
            self.assertRaisesRegex(ValueError, "ordinary checkout directories"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed an object-store symlink")

    def test_symlinked_loose_object_is_rejected(self) -> None:
        name = str(self.base)
        loose = self.source.path / ".git" / "objects" / name[:2] / name[2:]
        saved = self.root / "saved-object"
        loose.rename(saved)
        loose.symlink_to(saved)
        with (
            self.assertRaisesRegex(ValueError, "unsafe candidate metadata"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed a loose-object symlink")

    def test_hardlinked_loose_object_is_rejected(self) -> None:
        name = str(self.base)
        loose = self.source.path / ".git" / "objects" / name[:2] / name[2:]
        os.link(loose, self.root / "linked-object")
        with (
            self.assertRaisesRegex(ValueError, "hardlinked"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an object shared with another path")

    def test_shallow_and_reftable_repositories_are_rejected(self) -> None:
        metadata = self.source.path / ".git"
        shallow = metadata / "shallow"
        shallow.write_text(f"{self.base}\n", encoding="ascii")
        with (
            self.assertRaisesRegex(ValueError, "shallow repositories"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a shallow repository")
        shallow.unlink()
        (metadata / "reftable").mkdir()
        with (
            self.assertRaisesRegex(ValueError, "reftable repositories"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a reftable repository")

    def test_promisor_and_incomplete_pack_pairs_are_rejected(self) -> None:
        self.source.command(("repack", "-ad"))
        pack = next((self.source.path / ".git" / "objects" / "pack").glob("*.pack"))
        promisor = pack.with_suffix(".promisor")
        promisor.write_bytes(b"")
        with (
            self.assertRaisesRegex(ValueError, "promisor packs"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a promisor pack")
        promisor.unlink()
        pack.with_suffix(".idx").unlink()
        with (
            self.assertRaisesRegex(ValueError, "complete pack/index pairs"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an unindexed pack")

    def test_corrupt_object_data_is_rejected(self) -> None:
        name = str(self.base)
        loose = self.source.path / ".git" / "objects" / name[:2] / name[2:]
        loose.chmod(0o644)
        loose.write_bytes(b"not a Git object")
        with (
            self.assertRaisesRegex(ValueError, "incomplete or corrupt"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted corrupt source object data")

    def test_in_place_source_changes_are_rejected(self) -> None:
        index = self.source.path / ".git" / "index"
        original_index = index.read_bytes()
        identity = index.stat()
        original_fstat = os.fstat
        reads = 0

        def fstat(descriptor: int) -> os.stat_result:
            nonlocal reads
            metadata = original_fstat(descriptor)
            if (metadata.st_dev, metadata.st_ino) == (identity.st_dev, identity.st_ino):
                reads += 1
                if reads == 2:
                    index.write_bytes(
                        original_index[:-1] + bytes([original_index[-1] ^ 1])
                    )
                    return original_fstat(descriptor)
            return metadata

        with (
            patch("uv_automations.checkouts.os.fstat", fstat),
            self.assertRaisesRegex(ValueError, "changed while reading"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a source file changed while being copied")
        self.assertEqual(reads, 2)

    def test_snapshot_file_and_byte_budgets_are_enforced(self) -> None:
        for name, value in (("_MAX_SNAPSHOT_FILES", 2), ("_MAX_SNAPSHOT_BYTES", 1)):
            with (
                self.subTest(limit=name),
                patch(f"uv_automations.checkouts.{name}", value),
                self.assertRaisesRegex(ValueError, "snapshot limits"),
                inspect_candidate(self.source, base=self.base, scratch=self.scratch),
            ):
                self.fail("Exceeded the source snapshot budget")

    def test_reject_tracked_and_untracked_leftovers(self) -> None:
        for name in ("tracked.txt", "untracked.txt"):
            path = self.source.path / name
            path.write_text("dirty\n", encoding="utf-8")
            with (
                self.subTest(name=name),
                inspect_candidate(
                    self.source, base=self.base, scratch=self.scratch
                ) as candidate,
                self.assertRaisesRegex(ValueError, "not clean"),
            ):
                candidate.require_clean()
            if name == "tracked.txt":
                path.write_text("A\n", encoding="utf-8")
            else:
                path.unlink()

    def test_reject_staged_only_leftovers(self) -> None:
        path = self.source.path / "tracked.txt"
        path.write_text("B\n", encoding="utf-8")
        self.source.command(("add", "--", "tracked.txt"))
        path.write_text("A\n", encoding="utf-8")
        with (
            inspect_candidate(
                self.source, base=self.base, scratch=self.scratch
            ) as candidate,
            self.assertRaisesRegex(ValueError, "index is not clean"),
        ):
            candidate.require_clean()

    def test_source_index_flags_cannot_hide_worktree_changes(self) -> None:
        path = self.source.path / "tracked.txt"
        for flag in ("--assume-unchanged", "--skip-worktree"):
            self.source.command(("update-index", flag, "--", "tracked.txt"))
            path.write_text("B\n", encoding="utf-8")
            with (
                self.subTest(flag=flag),
                inspect_candidate(
                    self.source, base=self.base, scratch=self.scratch
                ) as candidate,
                self.assertRaisesRegex(ValueError, "not clean"),
            ):
                candidate.require_clean()
            path.write_text("A\n", encoding="utf-8")
            self.source.command(
                (
                    "update-index",
                    "--no-assume-unchanged",
                    "--no-skip-worktree",
                    "--",
                    "tracked.txt",
                )
            )

    def test_forged_cached_tree_cannot_hide_staged_content(self) -> None:
        head = commit_file(self.source, "dir/tracked.txt", "nested A\n")
        self.source.command(("config", "index.version", "2"))
        self.source.command(("update-index", "--index-version=2"))
        self.source.output("write-tree")
        old_object = bytes.fromhex(
            self.source.output("rev-parse", "HEAD:dir/tracked.txt")
        )
        new_object = bytes.fromhex(
            self.source.command(
                ("hash-object", "-w", "--stdin"), input="nested B\n"
            ).stdout.strip()
        )
        index = self.source.path / ".git" / "index"
        contents = bytearray(index.read_bytes())
        self.assertEqual(contents[:8], b"DIRC\0\0\0\2")
        self.assertIn(b"TREE", contents)
        self.assertEqual(contents[:-20].count(old_object), 1)
        offset = contents.index(old_object, 12)
        contents[offset : offset + 20] = new_object
        contents[-20:] = hashlib.sha1(contents[:-20]).digest()
        index.write_bytes(contents)
        with (
            inspect_candidate(
                self.source, base=head, scratch=self.scratch
            ) as candidate,
            self.assertRaisesRegex(ValueError, "index is not clean"),
        ):
            candidate.require_clean()

    def test_reject_active_rebase_state(self) -> None:
        for name in ("rebase-merge", "rebase-apply"):
            state = self.source.path / ".git" / name
            state.mkdir()
            with (
                self.subTest(name=name),
                inspect_candidate(
                    self.source, base=self.base, scratch=self.scratch
                ) as candidate,
                self.assertRaisesRegex(ValueError, "rebase is still in progress"),
            ):
                candidate.require_clean()
            state.rmdir()

    def test_fsmonitor_command_is_not_executed(self) -> None:
        marker = self.root / "fsmonitor-ran"
        script = marker_script(
            self.root / "fsmonitor", marker, output="printf 'token\\0'"
        )
        self.source.command(("config", "core.fsmonitor", str(script)))
        self.source.command(("config", "core.fsmonitorHookVersion", "2"))
        self.source.command(("-c", f"core.fsmonitor={script}", "status", "--porcelain"))
        self.assertTrue(marker.exists())
        marker.unlink()
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
        self.assertFalse(marker.exists())

    def test_source_clean_filter_and_info_attributes_are_not_used(self) -> None:
        marker = self.root / "filter-ran"
        script = marker_script(self.root / "filter", marker, output="cat")
        self.source.command(("config", "filter.marker.clean", str(script)))
        attributes = self.source.path / ".git" / "info" / "attributes"
        attributes.parent.mkdir(exist_ok=True)
        attributes.write_text("tracked.txt filter=marker\n", encoding="utf-8")
        (self.source.path / "tracked.txt").write_text("A\n", encoding="utf-8")
        self.source.command(("status", "--porcelain"))
        self.assertTrue(marker.exists())
        marker.unlink()
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            self.assertFalse(
                (candidate.repository.path / "info" / "attributes").exists()
            )
        self.assertFalse(marker.exists())

    def test_changed_attributes_cannot_hide_dirty_content(self) -> None:
        base = commit_file(self.source, ".gitattributes", "* text=auto eol=lf\n")
        commit_file(self.source, ".gitattributes", "tracked.txt filter=marker\n")
        marker = self.root / "filter-ran"
        script = marker_script(self.root / "filter", marker, output="printf 'A\\n'")
        self.source.command(("config", "filter.marker.clean", str(script)))
        (self.source.path / "tracked.txt").write_text("B\n", encoding="utf-8")
        self.assertEqual(self.source.output("status", "--porcelain"), "")
        self.assertTrue(marker.exists())
        marker.unlink()
        with (
            inspect_candidate(
                self.source, base=base, scratch=self.scratch
            ) as candidate,
            self.assertRaisesRegex(ValueError, "not clean"),
        ):
            candidate.require_clean()
        self.assertFalse(marker.exists())

    def test_object_checks_keep_the_trusted_attribute_source(self) -> None:
        base = commit_file(self.source, ".gitattributes", "* text=auto eol=lf\n")
        commit_file(self.source, ".gitattributes", "tracked.txt -diff\n")
        head = commit_file(self.source, "tracked.txt", "A \n")
        with inspect_candidate(
            self.source, base=base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            self.assertEqual(
                candidate.repository.output("config", "attr.tree"), str(base)
            )
            self.assertNotEqual(
                candidate.repository.command(
                    ("diff", "--check", f"{base}..{head}", "--"), check=False
                ).returncode,
                0,
            )

    def test_configuration_defined_hook_is_not_executed(self) -> None:
        marker = self.root / "hook-ran"
        script = marker_script(self.root / "hook", marker)
        self.source.command(("config", "hook.sentinel.command", str(script)))
        self.source.command(("config", "hook.sentinel.event", "post-index-change"))
        if self.source.command(
            ("hook", "list", "post-index-change"), check=False
        ).returncode:
            self.skipTest("Git does not support configuration-defined hooks")
        self.source.command(("update-index", "--assume-unchanged", "--", "tracked.txt"))
        self.source.command(
            ("update-index", "--no-assume-unchanged", "--", "tracked.txt")
        )
        self.assertTrue(marker.exists())
        marker.unlink()
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
        self.assertFalse(marker.exists())

    def test_filesystem_hook_is_not_copied_or_executed(self) -> None:
        marker = self.root / "hook-ran"
        hooks = self.source.path / ".git" / "hooks"
        marker_script(hooks / "post-checkout", marker)
        self.source.command(
            (
                "-c",
                f"core.hooksPath={hooks}",
                "checkout",
                "--quiet",
                "--detach",
                str(self.base),
            )
        )
        self.assertTrue(marker.exists())
        marker.unlink()
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            self.assertFalse(
                (candidate.repository.path / "hooks" / "post-checkout").exists()
            )
        self.assertFalse(marker.exists())

    def test_clone_does_not_discover_scratch_ancestor_configuration(self) -> None:
        ancestor = create_repository(self.root / "ancestor")
        ancestor_head = commit_file(ancestor, "ancestor.txt", "ancestor\n")
        scratch = ancestor.path / "protected-scratch"
        scratch.mkdir()
        marker = self.root / "ancestor-hook-ran"
        script = marker_script(self.root / "ancestor-hook", marker)
        ancestor.command(("config", "hook.sentinel.command", str(script)))
        ancestor.command(("config", "hook.sentinel.event", "reference-transaction"))
        if ancestor.command(
            ("hook", "list", "reference-transaction"), check=False
        ).returncode:
            self.skipTest("Git does not support configuration-defined hooks")
        ancestor.command(("update-ref", "refs/heads/proof", str(ancestor_head)))
        self.assertTrue(marker.exists())
        marker.unlink()
        with inspect_candidate(
            self.source, base=self.base, scratch=scratch
        ) as candidate:
            candidate.require_clean()
        self.assertFalse(marker.exists())

    def test_split_index_is_copied_without_sharing_source_data(self) -> None:
        self.source.command(("config", "core.splitIndex", "true"))
        self.source.command(("update-index", "--split-index"))
        shared = tuple((self.source.path / ".git").glob("sharedindex.*"))
        self.assertTrue(shared)
        before = {path.name: path.read_bytes() for path in shared}
        with inspect_candidate(
            self.source, base=self.base, scratch=self.scratch
        ) as candidate:
            candidate.require_clean()
            for path in shared:
                copied = candidate.repository.path / path.name
                self.assertEqual(copied.read_bytes(), before[path.name])
                self.assertNotEqual(copied.stat().st_ino, path.stat().st_ino)
        self.assertEqual({path.name: path.read_bytes() for path in shared}, before)

    def test_missing_split_index_data_fails_closed(self) -> None:
        self.source.command(("config", "core.splitIndex", "true"))
        self.source.command(("update-index", "--split-index"))
        shared = Path(self.source.output("rev-parse", "--shared-index-path"))
        if not shared.is_absolute():
            shared = self.source.path / shared
        shared.unlink()
        with (
            self.assertRaises((ValueError, subprocess.CalledProcessError)),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an incomplete split index")

    def test_missing_source_index_fails_closed(self) -> None:
        (self.source.path / ".git" / "index").unlink()
        with (
            self.assertRaisesRegex(ValueError, "Missing or unsafe candidate metadata"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a missing index")

    def test_symlinked_index_fails_closed(self) -> None:
        index = self.source.path / ".git" / "index"
        saved = self.root / "saved-index"
        index.rename(saved)
        index.symlink_to(saved)
        with (
            self.assertRaisesRegex(ValueError, "unsafe candidate metadata"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed an index symlink")

    def test_symlinked_shared_index_fails_closed(self) -> None:
        self.source.command(("config", "core.splitIndex", "true"))
        self.source.command(("update-index", "--split-index"))
        shared = Path(self.source.output("rev-parse", "--shared-index-path"))
        if not shared.is_absolute():
            shared = self.source.path / shared
        saved = self.root / "saved-shared-index"
        shared.rename(saved)
        shared.symlink_to(saved)
        with (
            self.assertRaisesRegex(ValueError, "unsafe candidate metadata"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Followed a shared-index symlink")

    def test_fifo_index_fails_closed_without_blocking(self) -> None:
        index = self.source.path / ".git" / "index"
        index.unlink()
        os.mkfifo(index)
        with (
            self.assertRaisesRegex(ValueError, "regular file"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an index FIFO")

    def test_gitfiles_and_commondir_are_rejected(self) -> None:
        metadata = self.source.path / ".git"
        common = metadata / "commondir"
        common.write_text("../elsewhere\n", encoding="utf-8")
        with (
            self.assertRaisesRegex(ValueError, "linked worktrees"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted commondir metadata")
        common.unlink()
        moved = self.root / "moved.git"
        metadata.rename(moved)
        metadata.write_text(f"gitdir: {moved}\n", encoding="utf-8")
        with (
            self.assertRaisesRegex(ValueError, "ordinary checkout directories"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a gitfile")

    def test_index_and_tree_gitlinks_are_rejected(self) -> None:
        self.source.command(
            ("update-index", "--add", "--cacheinfo", f"160000,{self.base},nested")
        )
        with (
            self.assertRaisesRegex(ValueError, "submodules"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an index gitlink")
        self.source.command(("commit", "--quiet", "--message", "gitlink"))
        with (
            self.assertRaisesRegex(ValueError, "submodules"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted a tree gitlink")

    def test_inherited_git_control_variables_are_ignored(self) -> None:
        marker = self.root / "inherited-ran"
        script = marker_script(self.root / "inherited", marker)
        with (
            patch.dict(
                os.environ,
                {
                    "GIT_DIR": str(self.root / "other.git"),
                    "GIT_EXEC_PATH": str(self.root / "missing-executables"),
                    "GIT_TEMPLATE_DIR": str(self.source.path / ".git"),
                    "GIT_EXTERNAL_DIFF": str(script),
                    "GIT_ATTR_SOURCE": "not-a-revision",
                    "GIT_ALLOW_PROTOCOL": "ext",
                    "GIT_CONFIG_COUNT": "2",
                    "GIT_CONFIG_KEY_0": "hook.inherited.command",
                    "GIT_CONFIG_VALUE_0": str(script),
                    "GIT_CONFIG_KEY_1": "hook.inherited.event",
                    "GIT_CONFIG_VALUE_1": "reference-transaction",
                },
            ),
            inspect_candidate(
                self.source, base=self.base, scratch=self.scratch
            ) as candidate,
        ):
            candidate.require_clean()
        self.assertFalse(marker.exists())

    def test_scratch_inside_checkout_is_rejected(self) -> None:
        scratch = self.source.path / "scratch"
        scratch.mkdir()
        for destination in (self.source.path, scratch):
            with (
                self.subTest(destination=destination),
                self.assertRaisesRegex(ValueError, "outside the checkout"),
                inspect_candidate(self.source, base=self.base, scratch=destination),
            ):
                self.fail("Accepted agent-writable scratch")

    def test_older_git_fails_without_an_unsafe_fallback(self) -> None:
        with (
            patch(
                "uv_automations.checkouts.Git.output",
                return_value="git version 2.50.0.windows.1",
            ),
            self.assertRaisesRegex(ValueError, "requires Git 2.50.1"),
            inspect_candidate(self.source, base=self.base, scratch=self.scratch),
        ):
            self.fail("Accepted an unsupported Git version")


class CandidateRequirementsTests(unittest.TestCase):
    def test_missing_nofollow_support_fails_closed(self) -> None:
        with (
            patch("uv_automations.checkouts.os.supports_dir_fd", set()),
            self.assertRaises(OSError) as error,
            inspect_candidate(
                Git(Path("source")), base=CommitSha("a" * 40), scratch=Path("scratch")
            ),
        ):
            self.fail("Accepted unsupported filesystem operations")
        self.assertEqual(error.exception.errno, errno.ENOTSUP)

    def test_missing_nofollow_flags_fail_closed(self) -> None:
        for flag in ("O_DIRECTORY", "O_NOFOLLOW", "O_NONBLOCK"):
            with (
                self.subTest(flag=flag),
                patch(f"uv_automations.checkouts.os.{flag}", 0, create=True),
                self.assertRaises(OSError) as error,
                inspect_candidate(
                    Git(Path("source")),
                    base=CommitSha("a" * 40),
                    scratch=Path("scratch"),
                ),
            ):
                self.fail("Accepted unsupported filesystem operations")
            self.assertEqual(error.exception.errno, errno.ENOTSUP)
