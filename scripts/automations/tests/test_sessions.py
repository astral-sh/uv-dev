import json
import os
import unittest
from compression import zstd
from pathlib import Path, PurePath
from tempfile import TemporaryDirectory
from unittest.mock import patch
from uuid import UUID

from uv_automations.sessions import (
    CodexSessionSnapshot,
    SessionFile,
    snapshot_sessions,
    write_sessions,
)

ROOT_ID = UUID("123e4567-e89b-12d3-a456-426614174000")
CHILD_ID = UUID("123e4567-e89b-12d3-a456-426614174001")


def rollout(
    workspace: Path,
    identifier: UUID = ROOT_ID,
    *,
    source: object = "exec",
    originator: str = "codex_github_action",
) -> bytes:
    metadata = {
        "type": "session_meta",
        "payload": {
            "id": str(identifier),
            "source": source,
            "originator": originator,
            "cwd": str(workspace),
        },
    }
    return (
        json.dumps(metadata)
        + "\n"
        + '{"type":"event_msg","payload":{"message":"test"}}\n'
    ).encode()


class CodexSessionTests(unittest.TestCase):
    def test_snapshot_normalizes_compressed_rollouts_and_preserves_one_root(
        self,
    ) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / f"rollout-{ROOT_ID}.jsonl.zst").write_bytes(
                zstd.compress(rollout(root))
            )
            child = rollout(
                root,
                CHILD_ID,
                source={
                    "subagent": {
                        "thread_spawn": {"parent_thread_id": str(ROOT_ID), "depth": 1}
                    }
                },
            )
            (source / f"rollout-{CHILD_ID}.jsonl").write_bytes(child)
            snapshot = snapshot_sessions(source, root, trusted_root=root)
            self.assertEqual(snapshot.identifier, ROOT_ID)
            self.assertEqual(len(snapshot.files), 2)
            destination = root / "destination"
            write_sessions(snapshot, destination)
            self.assertEqual(
                (destination / f"rollout-{ROOT_ID}.jsonl").read_bytes(), rollout(root)
            )
            self.assertEqual(
                (destination / f"rollout-{CHILD_ID}.jsonl").read_bytes(), child
            )
            with self.assertRaises(FileExistsError):
                write_sessions(snapshot, destination)

    def test_root_session_must_match_the_expected_workspace_and_originator(
        self,
    ) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            path = source / f"rollout-{ROOT_ID}.jsonl"
            for content in (
                rollout(root / "other"),
                rollout(root, originator="cli"),
                rollout(root, source="cli"),
                b"{}\n",
            ):
                path.write_bytes(content)
                with (
                    self.subTest(content=content),
                    self.assertRaises((TypeError, ValueError, KeyError)),
                ):
                    snapshot_sessions(source, root, trusted_root=root)

    def test_multiple_roots_and_duplicate_compressed_files_are_rejected(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            first = source / f"rollout-{ROOT_ID}.jsonl"
            second = source / f"rollout-{CHILD_ID}.jsonl"
            first.write_bytes(rollout(root))
            second.write_bytes(rollout(root, CHILD_ID))
            with self.assertRaisesRegex(ValueError, "root Codex session"):
                snapshot_sessions(source, root, trusted_root=root)
            second.unlink()
            first.with_suffix(".jsonl.zst").write_bytes(zstd.compress(rollout(root)))
            with self.assertRaisesRegex(ValueError, "duplicate|Duplicate"):
                snapshot_sessions(source, root, trusted_root=root)

    def test_unrelated_roots_or_subagents_cannot_be_retained(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / f"rollout-{ROOT_ID}.jsonl").write_bytes(rollout(root))
            child = source / f"rollout-{CHILD_ID}.jsonl"
            for descriptor in (
                "cli",
                {"subagent": "review"},
                {"subagent": {"thread_spawn": {"parent_thread_id": str(CHILD_ID)}}},
                {
                    "subagent": {
                        "thread_spawn": {
                            "parent_thread_id": "123e4567-e89b-12d3-a456-426614174999"
                        }
                    }
                },
            ):
                child.write_bytes(rollout(root, CHILD_ID, source=descriptor))
                with (
                    self.subTest(descriptor=descriptor),
                    self.assertRaises((TypeError, ValueError)),
                ):
                    snapshot_sessions(source, root, trusted_root=root)

    def test_retained_paths_and_snapshot_size_are_validated(self) -> None:
        for path in (
            PurePath("/rollout-test.jsonl"),
            PurePath("../rollout-test.jsonl"),
            PurePath("sessions/config.toml"),
        ):
            with self.subTest(path=path), self.assertRaises(ValueError):
                SessionFile(path, b"")
        file = SessionFile(PurePath("rollout-test.jsonl"), b"test")
        with self.assertRaises(ValueError):
            CodexSessionSnapshot(ROOT_ID, (file, file))

    def test_symlinks_and_oversized_rollouts_are_rejected(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            target = root / "target"
            target.write_bytes(rollout(root))
            link = source / f"rollout-{ROOT_ID}.jsonl"
            link.symlink_to(target)
            with self.assertRaises((OSError, ValueError)):
                snapshot_sessions(source, root, trusted_root=root)
            link.unlink()
            link.write_bytes(rollout(root))
            with (
                patch("uv_automations.sessions.MAX_SESSION_FILE_BYTES", 16),
                self.assertRaisesRegex(ValueError, "size limit"),
            ):
                snapshot_sessions(source, root, trusted_root=root)

    def test_an_agent_owned_ancestor_cannot_redirect_the_session_root(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            trusted = root / "runner-temp"
            trusted.mkdir()
            agent = trusted / "comments-agent"
            agent.mkdir()
            outside = root / "private-codex-home"
            (outside / "sessions").mkdir(parents=True)
            (outside / "sessions" / f"rollout-{ROOT_ID}.jsonl").write_bytes(
                rollout(root)
            )
            (agent / "codex-home").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(OSError):
                snapshot_sessions(
                    agent / "codex-home/sessions", root, trusted_root=trusted
                )

    def test_an_enumerated_directory_swap_cannot_escape_the_trusted_root(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            trusted = root / "runner-temp"
            source = trusted / "comments-agent/codex-home/sessions"
            original = source / "2026"
            original.mkdir(parents=True)
            (original / f"rollout-{ROOT_ID}.jsonl").write_bytes(rollout(root))
            outside = root / "private-sessions"
            outside.mkdir()
            (outside / f"rollout-{ROOT_ID}.jsonl").write_bytes(
                rollout(root) + b"private sentinel\n"
            )
            real_open = os.open
            swapped = False

            def open_after_swap(
                path: str | bytes | os.PathLike[str] | os.PathLike[bytes],
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                nonlocal swapped
                if path == "2026" and dir_fd is not None and not swapped:
                    swapped = True
                    original.rename(source / "original")
                    original.symlink_to(outside, target_is_directory=True)
                return real_open(path, flags, mode, dir_fd=dir_fd)

            with (
                patch("uv_automations.sessions.os.open", side_effect=open_after_swap),
                self.assertRaises(OSError),
            ):
                snapshot_sessions(source, root, trusted_root=trusted)
            self.assertTrue(swapped)

    def test_an_outside_hardlink_cannot_be_retained(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            trusted = root / "runner-temp"
            source = trusted / "comments-agent/codex-home/sessions"
            source.mkdir(parents=True)
            outside = root / "private-rollout.jsonl"
            outside.write_bytes(rollout(root) + b"private sentinel\n")
            path = source / f"rollout-{ROOT_ID}.jsonl"
            real_open = os.open

            def open_then_unlink(
                name: str | bytes | os.PathLike[str] | os.PathLike[bytes],
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                descriptor = real_open(name, flags, mode, dir_fd=dir_fd)
                if name == path.name and dir_fd is not None:
                    path.unlink()
                return descriptor

            for remove_after_open in (False, True):
                os.link(outside, path)
                self.assertEqual(path.stat().st_nlink, 2)
                with (
                    self.subTest(remove_after_open=remove_after_open),
                    patch(
                        "uv_automations.sessions.os.open",
                        side_effect=open_then_unlink
                        if remove_after_open
                        else real_open,
                    ),
                    self.assertRaisesRegex(ValueError, "hardlinked"),
                ):
                    snapshot_sessions(source, root, trusted_root=trusted)
                path.unlink(missing_ok=True)
            self.assertTrue(outside.read_bytes().endswith(b"private sentinel\n"))

    def test_a_swapped_fifo_does_not_block_before_regular_file_validation(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "sessions"
            source.mkdir()
            path = source / f"rollout-{ROOT_ID}.jsonl"
            path.write_bytes(rollout(root))
            real_open = os.open
            swapped = False

            def open_after_swap(
                name: str | bytes | os.PathLike[str] | os.PathLike[bytes],
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                nonlocal swapped
                if name == path.name and dir_fd is not None and not swapped:
                    self.assertTrue(flags & os.O_NONBLOCK)
                    swapped = True
                    path.unlink()
                    os.mkfifo(path)
                return real_open(name, flags, mode, dir_fd=dir_fd)

            with (
                patch("uv_automations.sessions.os.open", side_effect=open_after_swap),
                self.assertRaisesRegex(ValueError, "regular file"),
            ):
                snapshot_sessions(source, root, trusted_root=root)
            self.assertTrue(swapped)
