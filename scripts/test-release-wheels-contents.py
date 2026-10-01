# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for the release wheel-content check."""

from __future__ import annotations

import importlib.util
import io
import sys
import unittest
import warnings
from contextlib import redirect_stderr
from pathlib import Path
from tempfile import TemporaryDirectory
from types import ModuleType
from zipfile import ZipFile


def load_checker(path: Path) -> ModuleType:
    specification = importlib.util.spec_from_file_location("_wheel_contents", path)
    if specification is None or specification.loader is None:
        raise ImportError(f"Cannot load wheel-content checker: {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


CHECKER = load_checker(Path(__file__).with_name("check_uv_wheel_contents.py"))
VERSION = "0.12.15"


class WheelContentsTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def files(self, package="uv", version=VERSION, windows=False):
        expected = CHECKER.uv_expected if package == "uv" else CHECKER.uv_build_expected
        if package == "uv" and windows:
            expected = expected | {"uv-VERSION.data/scripts/uvw"}
        return sorted(
            entry.replace("VERSION", version)
            + (".exe" if windows and ".data/scripts/" in entry else "")
            for entry in expected
        )

    def wheel(
        self,
        entries,
        package="uv",
        version=VERSION,
        platform="manylinux_2_17_x86_64",
        build=None,
    ):
        build_tag = f"-{build}" if build is not None else ""
        path = self.root / f"{package}-{version}{build_tag}-py3-none-{platform}.whl"
        with ZipFile(path, "w") as archive, warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            for index, entry in enumerate(entries):
                archive.writestr(entry, f"entry {index}")
        return path

    def assert_rejected(self, wheel, diagnostic):
        stderr = io.StringIO()
        with redirect_stderr(stderr), self.assertRaises(SystemExit) as caught:
            CHECKER.check_uv_wheel(wheel)
        self.assertEqual(caught.exception.code, 1)
        self.assertIn(diagnostic, stderr.getvalue())

    def test_release_wheels_are_accepted(self):
        for package in ("uv", "uv_build"):
            for platform in (
                "manylinux_2_17_x86_64.manylinux2014_x86_64",
                "macosx_11_0_arm64",
                "win32",
                "win_amd64",
                "win_arm64",
            ):
                with self.subTest(package=package, platform=platform):
                    windows = platform.startswith("win")
                    CHECKER.check_uv_wheel(
                        self.wheel(
                            self.files(package, windows=windows),
                            package=package,
                            platform=platform,
                        )
                    )

    def test_filename_version_is_used(self):
        for version in ("0.13.0rc1", "0.12.15+local"):
            with self.subTest(version=version):
                CHECKER.check_uv_wheel(
                    self.wheel(self.files(version=version), version=version, build="1")
                )

    def test_duplicate_member_is_rejected(self):
        entries = self.files()
        self.assert_rejected(
            self.wheel([*entries, "uv/__init__.py"]),
            "Duplicate wheel entries: ['uv/__init__.py']",
        )

    def test_wrong_version_directory_is_rejected(self):
        entries = [entry.replace(VERSION, "0.12.14") for entry in self.files()]
        self.assert_rejected(self.wheel(entries), "Missing wheel entries:")

    def test_exe_inside_module_name_is_rejected(self):
        entries = [
            entry.replace("uv/__init__.py", "uv/__init__.exe.py")
            for entry in self.files()
        ]
        self.assert_rejected(self.wheel(entries), "Unexpected wheel entries:")

    def test_unix_executables_must_not_have_exe_suffix(self):
        entries = [
            entry + ".exe" if ".data/scripts/" in entry else entry
            for entry in self.files()
        ]
        self.assert_rejected(self.wheel(entries), "Unexpected wheel entries:")

    def test_windows_executables_require_exe_suffix(self):
        entries = [entry.removesuffix(".exe") for entry in self.files(windows=True)]
        self.assert_rejected(
            self.wheel(entries, platform="win_amd64"), "Missing wheel entries:"
        )

    def test_missing_license_is_rejected(self):
        entries = [entry for entry in self.files() if not entry.endswith("LICENSE-MIT")]
        self.assert_rejected(self.wheel(entries), "Missing wheel entries:")

    def test_extra_member_is_rejected(self):
        self.assert_rejected(
            self.wheel([*self.files(), "unexpected.txt"]), "Unexpected wheel entries:"
        )


if __name__ == "__main__":
    unittest.main()
