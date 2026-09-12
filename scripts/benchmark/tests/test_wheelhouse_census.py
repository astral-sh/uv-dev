"""Unit tests for the ordinary wheelhouse census driver."""

from __future__ import annotations

import base64
import hashlib
import os
import stat
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wheelhouse_census as census


def wheel_fixture(
    directory: Path,
    *,
    metadata_name: str = "uv-census-target",
    bad_record: bool = False,
) -> tuple[Path, dict]:
    normalized = "uv_census_target"
    dist_info = f"{normalized}-1.0.0.dist-info"
    contents = {
        f"{normalized}/__init__.py": b'__version__ = "1.0.0"\n',
        f"{dist_info}/METADATA": (
            f"Metadata-Version: 2.3\nName: {metadata_name}\nVersion: 1.0.0\n"
        ).encode(),
        f"{dist_info}/WHEEL": (
            b"Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\n"
            b"Tag: py3-none-any\n"
        ),
    }
    rows = []
    for name, data in contents.items():
        digest = (
            base64.urlsafe_b64encode(hashlib.sha256(data).digest())
            .rstrip(b"=")
            .decode()
        )
        rows.append(f"{name},sha256={digest},{len(data)}\n")
    if bad_record:
        rows[0] = rows[0].replace("sha256=", "sha512=", 1)
    rows.append(f"{dist_info}/RECORD,,\n")
    contents[f"{dist_info}/RECORD"] = "".join(rows).encode()
    path = directory / f"{normalized}-1.0.0-py3-none-any.whl"
    with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_STORED) as archive:
        for name, data in contents.items():
            member = zipfile.ZipInfo(name)
            member.create_system = 3
            member.external_attr = (stat.S_IFREG | 0o644) << 16
            archive.writestr(member, data)
    expected = {
        "filename": path.name,
        "name": "uv-census-target",
        "version": "1.0.0",
        "tags": ["py3-none-any"],
        "sha256": census.sha256_file(path),
        "size": path.stat().st_size,
    }
    return path, expected


class WheelValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(
            prefix="wheelhouse-census-unit-",
            dir=os.environ.get("UV_WHEELHOUSE_TEST_SCRATCH"),
        )
        self.directory = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def test_four_member_wheel_is_validated(self) -> None:
        path, expected = wheel_fixture(self.directory)
        result = census.validate_generated_wheel(path, expected)
        self.assertEqual(result["name"], "uv-census-target")
        self.assertEqual(result["sha256"], expected["sha256"])
        self.assertEqual(len(result["record_sha256"]), 64)

    def test_record_hash_mismatch_is_rejected_after_outer_hash_validation(self) -> None:
        path, expected = wheel_fixture(self.directory, bad_record=True)
        with self.assertRaisesRegex(ValueError, "RECORD mismatch"):
            census.validate_generated_wheel(path, expected)

    def test_embedded_identity_mismatch_is_rejected(self) -> None:
        path, expected = wheel_fixture(
            self.directory, metadata_name="some-other-package"
        )
        with self.assertRaisesRegex(ValueError, "metadata name differs"):
            census.validate_generated_wheel(path, expected)

    def test_manifest_path_must_be_normal_and_contained(self) -> None:
        (self.directory / "data").mkdir()
        (self.directory / "data" / "file").write_bytes(b"")
        self.assertEqual(
            census.relative_file(self.directory, "data/file"),
            self.directory / "data" / "file",
        )
        for path in ("../file", "/file", "data/./file", "data//file", "data\\file"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                census.relative_file(self.directory, path)


class TraceTests(unittest.TestCase):
    def test_paths_fd_reads_and_resumed_calls_are_attributed(self) -> None:
        trace_lines = [
            '10 1789130000.000001 openat(AT_FDCWD, "/stage/wheels/a.whl", O_RDONLY|O_CLOEXEC) = 3</stage/wheels/a.whl> <0.000010>',
            '10 1789130000.000002 newfstatat(4</stage/wheels>, "a.whl", {st_mode=S_IFREG|0644}, AT_SYMLINK_NOFOLLOW) = 0 <0.000020>',
            "11 1789130000.000003 read(3</stage/wheels/a.whl>,  <unfinished ...>",
            '11 1789130000.000004 <... read resumed>"abc", 4096) = 3 <0.000030>',
            '10 1789130000.000005 read(8</other>, "/stage/wheels/a.whl", 4096) = 19 <0.000040>',
            '10 1789130000.000006 newfstatat(4</stage/wheels>, "../outside", {}, 0) = -1 ENOENT (No such file or directory) <0.000050>',
            "10 1789130000.000007 getdents64(4</stage/wheels>, 0x1234 /* 3 entries */, 32768) = 96 <0.000060>",
            '10 1789130000.000008 readlink("/other", "/stage/wheels", 4096) = 13 <0.000070>',
            '10 1789130000.000009 statx(AT_FDCWD, "wheels/a.whl", AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, {}) = 0 <0.000080>',
            "10 1789130000.000010 close(3</stage/wheels/a.whl>) = 0 <0.000090>",
            "10 1789130000.000011 +++ exited with 0 +++",
        ]
        trace = "\n".join(trace_lines)
        result = census.attribute_trace(trace, Path("/stage/wheels"), Path("/stage"))
        self.assertTrue(result["complete"], result["unparsed_lines"])
        self.assertEqual(result["process_calls"]["read"], 2)
        self.assertEqual(result["process_errors"]["newfstatat"], 1)
        self.assertEqual(result["wheelhouse_calls"]["read"], 1)
        self.assertEqual(result["wheelhouse_calls"]["newfstatat"], 1)
        self.assertNotIn("readlink", result["wheelhouse_calls"])
        self.assertEqual(result["wheelhouse_nofollow_metadata_calls"], 2)
        self.assertEqual(result["wheel_opens"], {"a.whl": 1})
        self.assertEqual(result["wheel_reads"], {"a.whl": {"calls": 1, "bytes": 3}})

    def test_unfinished_or_unknown_lines_mark_attribution_incomplete(self) -> None:
        result = census.attribute_trace(
            "10 1789130000.0 read(3</stage/wheels/a.whl>, <unfinished ...>\n"
            "unexpected output\n",
            Path("/stage/wheels"),
        )
        self.assertFalse(result["complete"])
        self.assertEqual(len(result["unparsed_lines"]), 2)

    def test_fd_annotations_can_contain_commas(self) -> None:
        self.assertEqual(
            census.operation_paths(
                "newfstatat", '3</stage/a,b>, "wheel.whl", {}, 0', Path("/stage")
            ),
            {"/stage/a,b/wheel.whl"},
        )

    def test_command_has_the_offline_scope_and_an_isolated_environment(self) -> None:
        command = census.uv_command(
            Path("/uv"),
            Path("/python"),
            Path("/cache"),
            Path("/wheels"),
            Path("/input.in"),
        )
        self.assertEqual(
            command,
            [
                "/uv",
                "--no-config",
                "--offline",
                "--cache-dir",
                "/cache",
                "pip",
                "compile",
                "--python",
                "/python",
                "--no-index",
                "--only-binary",
                ":all:",
                "--find-links",
                "/wheels",
                "--no-header",
                "--no-annotate",
                "/input.in",
            ],
        )
        self.assertEqual(
            census.clean_environment(Path("/owned/tmp")),
            {
                "PATH": "/usr/bin:/bin",
                "LANG": "C.UTF-8",
                "TMPDIR": "/owned/tmp",
                "UV_PYTHON_DOWNLOADS": "never",
                "UV_NO_PROGRESS": "1",
            },
        )


if __name__ == "__main__":
    unittest.main()
