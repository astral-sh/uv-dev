# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for publishing a repaired source distribution."""

from __future__ import annotations

import importlib.util
import io
import os
import stat
import subprocess
import sys
import tarfile
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import ModuleType
from unittest.mock import patch


def load_repair(path: Path) -> ModuleType:
    specification = importlib.util.spec_from_file_location("_sdist_repair", path)
    if specification is None or specification.loader is None:
        raise ImportError(f"Cannot load sdist repair script: {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


REPAIR = load_repair(Path(__file__).with_name("repair-sdist-cargo-lock.py"))
NAME = "uv_build-0.12.15"
ORIGINAL_LOCK = b"original locked packages\n"
REPAIRED_LOCK = b"pruned locked packages\n"


class SdistRepairTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.sdist = self.root / f"{NAME}.tar.gz"
        with tarfile.open(self.sdist, "w:gz") as archive:
            for name, content in (
                (f"{NAME}/Cargo.lock", ORIGINAL_LOCK),
                (f"{NAME}/Cargo.toml", b"[package]\nname = 'uv-build'\n"),
                (f"{NAME}/README.md", b"source distribution\n"),
            ):
                member = tarfile.TarInfo(name)
                member.size = len(content)
                member.mode = 0o644
                archive.addfile(member, io.BytesIO(content))
        self.sdist.chmod(0o640)
        self.original = self.sdist.read_bytes()
        self.calls = []

    def cargo(self, argv, **kwargs):
        self.calls.append((argv, kwargs))
        directory = Path(kwargs["cwd"])
        self.assertEqual(directory.name, NAME)
        if argv == ["cargo", "update", "--workspace"]:
            self.assertEqual(kwargs, {"cwd": str(directory), "check": True})
            (directory / "Cargo.lock").write_bytes(REPAIRED_LOCK)
            return subprocess.CompletedProcess(argv, 0)
        self.assertEqual(argv, ["cargo", "metadata", "--locked", "--format-version=1"])
        self.assertEqual(
            kwargs, {"cwd": str(directory), "capture_output": True, "check": False}
        )
        return subprocess.CompletedProcess(argv, 0, b"{}", b"")

    def repair(self, path=None):
        with patch.object(REPAIR.subprocess, "run", side_effect=self.cargo):
            REPAIR.fix_sdist_lockfile(str(path or self.sdist))

    def assert_original(self):
        self.assertEqual(self.sdist.read_bytes(), self.original)
        self.assertEqual(stat.S_IMODE(self.sdist.stat().st_mode), 0o640)

    def assert_repaired(self):
        with tarfile.open(self.sdist, "r:gz") as archive:
            files = {
                member.name: self.read_member(archive, member.name)
                for member in archive
                if member.isfile()
            }
        self.assertEqual(
            files,
            {
                f"{NAME}/Cargo.lock": REPAIRED_LOCK,
                f"{NAME}/Cargo.toml": b"[package]\nname = 'uv-build'\n",
                f"{NAME}/README.md": b"source distribution\n",
            },
        )
        self.assertEqual(stat.S_IMODE(self.sdist.stat().st_mode), 0o640)

    def read_member(self, archive, name):
        member = archive.extractfile(name)
        if member is None:
            self.fail(f"Missing file member: {name}")
        return member.read()

    def test_success_replaces_complete_archive(self):
        self.repair()
        self.assert_repaired()
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(list(self.root.iterdir()), [self.sdist])

    def test_repack_failure_keeps_original(self):
        original_addfile = tarfile.TarFile.addfile
        writes = 0

        def addfile(archive, *args, **kwargs):
            nonlocal writes
            writes += 1
            if writes == 2:
                raise OSError("archive write failed")
            return original_addfile(archive, *args, **kwargs)

        with (
            patch.object(tarfile.TarFile, "addfile", addfile),
            self.assertRaisesRegex(OSError, "archive write failed"),
        ):
            self.repair()
        self.assert_original()
        self.assertEqual(list(self.root.iterdir()), [self.sdist])

    def test_archive_close_failure_keeps_original(self):
        original_close = tarfile.TarFile.close

        def close(archive):
            original_close(archive)
            if archive.mode == "w":
                raise OSError("archive close failed")

        with (
            patch.object(tarfile.TarFile, "close", close),
            self.assertRaisesRegex(OSError, "archive close failed"),
        ):
            self.repair()
        self.assert_original()
        self.assertEqual(list(self.root.iterdir()), [self.sdist])

    def test_replace_failure_keeps_original(self):
        with (
            patch.object(REPAIR.os, "replace", side_effect=OSError("replace failed")),
            self.assertRaisesRegex(OSError, "replace failed"),
        ):
            self.repair()
        self.assert_original()
        self.assertEqual(list(self.root.iterdir()), [self.sdist])

    def test_replacement_uses_completed_sibling_archive(self):
        original_replace = os.replace
        replacements = []

        def replace(source, destination):
            source = Path(source)
            destination = Path(destination)
            self.assert_original()
            self.assertEqual(source.parent.parent, self.root)
            self.assertEqual(destination, self.sdist)
            with tarfile.open(source, "r:gz") as archive:
                self.assertEqual(
                    self.read_member(archive, f"{NAME}/Cargo.lock"), REPAIRED_LOCK
                )
            replacements.append((source, destination))
            original_replace(source, destination)

        with patch.object(REPAIR.os, "replace", replace):
            self.repair()
        self.assertEqual(len(replacements), 1)
        self.assert_repaired()
        self.assertEqual(list(self.root.iterdir()), [self.sdist])

    def test_selected_symlink_keeps_pointing_to_repaired_archive(self):
        alias = self.root / "selected.tar.gz"
        alias.symlink_to(self.sdist.name)
        self.repair(alias)
        self.assertTrue(alias.is_symlink())
        self.assertEqual(alias.readlink(), Path(self.sdist.name))
        self.assert_repaired()
        self.assertEqual(alias.read_bytes(), self.sdist.read_bytes())
        self.assertEqual(set(self.root.iterdir()), {self.sdist, alias})

    def test_cargo_update_failure_keeps_original(self):
        with (
            patch.object(
                REPAIR.subprocess,
                "run",
                side_effect=subprocess.CalledProcessError(1, "cargo"),
            ),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            REPAIR.fix_sdist_lockfile(str(self.sdist))
        self.assert_original()

    def test_metadata_failure_keeps_original(self):
        def cargo(argv, **kwargs):
            result = self.cargo(argv, **kwargs)
            if argv[1] == "metadata":
                result.returncode = 1
                result.stderr = b"lockfile mismatch"
            return result

        with (
            patch.object(REPAIR.subprocess, "run", side_effect=cargo),
            self.assertRaises(SystemExit) as caught,
        ):
            REPAIR.fix_sdist_lockfile(str(self.sdist))
        self.assertEqual(caught.exception.code, 1)
        self.assert_original()


if __name__ == "__main__":
    unittest.main()
