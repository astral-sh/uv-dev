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

    def test_pathname_command_retains_execve_and_exit_records(self) -> None:
        command = census.pathname_trace_command(
            Path("/strace"),
            Path("/evidence/pathnames.txt"),
            ["/checked/uv", "pip", "compile"],
        )
        self.assertEqual(
            command,
            [
                "/strace",
                "-f",
                "-q",
                "-ttt",
                "-T",
                "-yy",
                "-s",
                "4096",
                "-e",
                "trace=openat,read,close,getdents64,statx,newfstatat,readlink,readlinkat,execve",
                "-o",
                "/evidence/pathnames.txt",
                "--",
                "/checked/uv",
                "pip",
                "compile",
            ],
        )
        self.assertNotIn("execve", census.TRACE_CALLS)


class CaptureTraceTests(unittest.TestCase):
    directory = Path("/stage/wheels")
    cwd = Path("/stage")
    uv = Path("/checked/uv")
    initial_exec = (
        '10 1789129999.999999 execve("/checked/uv", '
        '["/checked/uv", "pip", "compile"], 0x1234 /* 5 vars */) = 0 <0.000010>\n'
    )
    terminal_exit = "10 1789130001.000000 +++ exited with 0 +++\n"
    data = "getdents64(4</stage/wheels>, 0x1234 /* 4 entries */, 32768) = 128"
    eof = "getdents64(4</stage/wheels>, 0x1234 /* 0 entries */, 32768) = 0"
    a = (
        'newfstatat(4</stage/wheels>, "a.whl", '
        "{st_dev=makedev(0x8, 0x1), st_mode=S_IFREG|0644, st_size=3, ...}, "
        "AT_SYMLINK_NOFOLLOW) = 0"
    )
    b = (
        'statx(AT_FDCWD, "wheels/b.whl", '
        "AT_STATX_SYNC_AS_STAT|AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, "
        "{stx_mask=STATX_BASIC_STATS|STATX_MNT_ID, "
        "stx_mode=S_IFREG|0644, stx_size=5, ...}) = 0"
    )

    def setUp(self) -> None:
        self.entries = [
            {"filename": "a.whl", "size": 3},
            {"filename": "b.whl", "size": 5},
        ]

    @staticmethod
    def trace(*bodies: str) -> str:
        return "".join(
            f"10 1789130000.{index:06} {body} <0.000010>\n"
            for index, body in enumerate(bodies, 1)
        )

    def evidence(
        self, text: str, directory: Path | None = None, *, complete_process: bool = True
    ) -> dict:
        directory = directory or self.directory
        if complete_process:
            text = self.initial_exec + text + self.terminal_exit
        attribution = census.attribute_trace(text, directory, self.cwd)
        attribution["catalog_coverage"] = census.catalog_trace_coverage(
            text, directory, self.entries, self.cwd
        )
        attribution["tracee_termination"] = census.tracee_termination(text, self.uv)
        return attribution

    def require_gate(self, attribution: dict) -> None:
        census.require_capture_trace(attribution, Path("/evidence/pathnames.txt"))

    def assert_rejected(self, text: str, *, parser_complete: bool = True) -> dict:
        attribution = self.evidence(text)
        self.assertEqual(attribution["complete"], parser_complete)
        with self.assertRaisesRegex(
            ValueError,
            "incomplete (pathname attribution|catalog coverage|tracee termination)",
        ):
            self.require_gate(attribution)
        return attribution["catalog_coverage"]

    def test_empty_trace_is_parser_complete_but_rejected(self) -> None:
        coverage = self.assert_rejected("")
        self.assertEqual(coverage["missing_entries"], ["a.whl", "b.whl"])
        self.assertEqual(coverage["enumeration_data_calls"], 0)

    def test_unrelated_trace_is_parser_complete_but_rejected(self) -> None:
        coverage = self.assert_rejected(
            self.trace(
                "close(3</elsewhere>) = 0", self.data.replace("wheels", "wheels-old")
            )
        )
        self.assertFalse(coverage["accepted"])
        self.assertEqual(coverage["enumeration_data_calls"], 0)

    def test_clean_boundary_truncation_without_directory_eof_is_rejected(self) -> None:
        coverage = self.assert_rejected(self.trace(self.data, self.a, self.b))
        self.assertEqual(coverage["missing_entries"], [])
        self.assertEqual(coverage["enumeration_eof_calls"], 0)

    def test_unfinished_trace_is_rejected_even_with_complete_coverage(self) -> None:
        text = self.trace(self.data, self.a, self.b, self.eof)
        text += "11 1789130000.000010 read(8</elsewhere>, <unfinished ...>\n"
        coverage = self.assert_rejected(text, parser_complete=False)
        self.assertTrue(coverage["accepted"])

    def test_duplicate_lookup_does_not_hide_an_omitted_entry(self) -> None:
        coverage = self.assert_rejected(self.trace(self.data, self.a, self.a, self.eof))
        self.assertEqual(coverage["successful_nofollow_metadata"], {"a.whl": 2})
        self.assertEqual(coverage["missing_entries"], ["b.whl"])

    def test_complete_absolute_relative_and_resumed_trace_is_accepted(self) -> None:
        text = self.trace(
            'openat(AT_FDCWD, "/stage/wheels", O_RDONLY|O_DIRECTORY) = 4</stage/wheels>',
            self.data,
            self.a,
            self.a.replace(
                '4</stage/wheels>, "a.whl"', 'AT_FDCWD, "/stage/wheels/a.whl"'
            ),
            self.b,
        )
        text += (
            '11 1789130000.000010 statx(AT_FDCWD, "/stage/wheels/b.whl", '
            "AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, <unfinished ...>\n"
            "11 1789130000.000011 <... statx resumed>{stx_mask=STATX_BASIC_STATS, "
            "stx_mode=S_IFREG|0644, stx_size=5, ...}) = 0 <0.000020>\n"
        )
        text += self.trace(self.eof, "close(4</stage/wheels>) = 0")
        attribution = self.evidence(text)
        self.require_gate(attribution)
        self.assertTrue(attribution["complete"])
        self.assertEqual(
            attribution["catalog_coverage"]["successful_nofollow_metadata"],
            {"a.whl": 2, "b.whl": 2},
        )
        self.assertEqual(attribution["wheel_reads"], {})

    def test_failed_or_following_lookup_does_not_cover_an_entry(self) -> None:
        for body in (
            self.b.replace("= 0", "= -1 ENOENT (No such file or directory)"),
            self.b.replace("AT_STATX_SYNC_AS_STAT|AT_SYMLINK_NOFOLLOW", "0"),
        ):
            with self.subTest(body=body):
                coverage = self.assert_rejected(
                    self.trace(self.data, self.a, body, self.eof)
                )
                self.assertEqual(coverage["missing_entries"], ["b.whl"])

    def test_flag_text_in_a_path_does_not_supply_the_flag(self) -> None:
        directory = Path("/stage/AT_SYMLINK_NOFOLLOW")
        text = (
            self.trace(self.data, self.a, self.b, self.eof)
            .replace("wheels", "AT_SYMLINK_NOFOLLOW")
            .replace(
                "AT_STATX_SYNC_AS_STAT|AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS",
                "0, STATX_BASIC_STATS",
            )
        )
        attribution = self.evidence(text, directory)
        self.assertTrue(attribution["complete"])
        self.assertEqual(attribution["wheelhouse_nofollow_metadata_calls"], 1)
        with self.assertRaisesRegex(ValueError, "incomplete catalog coverage"):
            self.require_gate(attribution)
        self.assertEqual(attribution["catalog_coverage"]["missing_entries"], ["b.whl"])

    def test_unexpected_top_level_and_nested_paths_are_rejected(self) -> None:
        for extra, path in (
            (self.a.replace('"a.whl"', '"extra.whl"'), "/stage/wheels/extra.whl"),
            (self.a.replace('"a.whl"', '"nested/a.whl"'), "/stage/wheels/nested/a.whl"),
            (
                'openat(AT_FDCWD, "/elsewhere", O_RDONLY) = 8</stage/wheels/extra.whl>',
                "/stage/wheels/extra.whl",
            ),
        ):
            with self.subTest(extra=extra):
                coverage = self.assert_rejected(
                    self.trace(self.data, self.a, self.b, extra, self.eof)
                )
                self.assertEqual(coverage["missing_entries"], [])
                self.assertEqual(coverage["unexpected_paths"], [path])
                self.assertEqual(coverage["contradictions"][0]["path"], path)

    def test_later_success_does_not_hide_metadata_contradictions(self) -> None:
        for conflicting, reason in (
            (
                self.a.replace("= 0", "= -1 ENOENT (No such file or directory)"),
                "failed metadata lookup",
            ),
            (self.a.replace("S_IFREG", "S_IFDIR"), "metadata file type differs"),
            (self.a.replace("st_size=3", "st_size=4"), "metadata file size differs"),
        ):
            with self.subTest(reason=reason):
                coverage = self.assert_rejected(
                    self.trace(self.data, conflicting, self.a, self.b, self.eof)
                )
                self.assertEqual(coverage["missing_entries"], [])
                self.assertIn(
                    reason, [item["reason"] for item in coverage["contradictions"]]
                )

    def test_partial_statx_does_not_conflict_with_later_complete_metadata(self) -> None:
        partial = self.b.replace(
            "STATX_BASIC_STATS|STATX_MNT_ID", "STATX_TYPE"
        ).replace(", stx_size=5", "")
        coverage = self.assert_rejected(
            self.trace(self.data, self.a, partial, self.eof)
        )
        self.assertEqual(coverage["missing_entries"], ["b.whl"])
        self.assertEqual(coverage["contradictions"], [])
        attribution = self.evidence(
            self.trace(self.data, self.a, partial, self.b, self.eof)
        )
        self.require_gate(attribution)
        self.assertEqual(
            attribution["catalog_coverage"]["successful_nofollow_metadata"],
            {"a.whl": 1, "b.whl": 1},
        )

    def test_empty_path_and_deleted_descriptors_cannot_supply_coverage(self) -> None:
        empty_path = self.b.replace(
            'AT_FDCWD, "wheels/b.whl"', '8</stage/wheels/b.whl>, ""'
        ).replace(
            "AT_STATX_SYNC_AS_STAT|AT_SYMLINK_NOFOLLOW",
            "AT_EMPTY_PATH|AT_SYMLINK_NOFOLLOW",
        )
        coverage = self.assert_rejected(
            self.trace(self.data, self.a, empty_path, self.eof)
        )
        self.assertEqual(coverage["missing_entries"], ["b.whl"])
        coverage = self.assert_rejected(
            self.trace(
                self.data,
                self.a,
                self.b,
                "close(8</stage/wheels/b.whl (deleted)>) = 0",
                self.eof,
            )
        )
        self.assertIn(
            "deleted catalog descriptor",
            [item["reason"] for item in coverage["contradictions"]],
        )

    def test_directory_enumeration_must_succeed_on_the_exact_directory(self) -> None:
        for data in (
            self.data.replace("= 128", "= -1 EIO (Input/output error)"),
            self.data.replace("4</stage/wheels>", "4</stage/wheels/a.whl>"),
            self.data.replace("4</stage/wheels>", "4</stage/wheels-old>"),
        ):
            with self.subTest(data=data):
                coverage = self.assert_rejected(
                    self.trace(data, self.a, self.b, self.eof)
                )
                self.assertEqual(coverage["enumeration_data_calls"], 0)

    def test_directory_eof_must_follow_data_on_the_same_process_and_descriptor(
        self,
    ) -> None:
        cases = (
            self.trace(self.eof, self.data, self.a, self.b),
            self.trace(self.data, self.a, self.b, self.eof.replace("4<", "5<")),
            self.trace(self.data, self.a, self.b)
            + self.trace(self.eof).replace("10 178913", "11 178913"),
            self.trace(self.data, self.a, self.b, self.eof, self.data),
            self.trace(
                self.data,
                self.a,
                self.b,
                "close(4</stage/wheels>) = 0",
                self.eof,
            ),
        )
        for text in cases:
            with self.subTest(text=text):
                coverage = self.assert_rejected(text)
                self.assertEqual(coverage["missing_entries"], [])

    def test_reused_directory_descriptor_cannot_finish_an_earlier_enumeration(
        self,
    ) -> None:
        for replacement in (
            'openat(AT_FDCWD, "/stage/wheels", O_RDONLY|O_DIRECTORY) = 4</stage/wheels>',
            self.data.replace("4</stage/wheels>", "4</elsewhere>"),
        ):
            with self.subTest(replacement=replacement):
                coverage = self.assert_rejected(
                    self.trace(self.data, self.a, self.b, replacement, self.eof)
                )
                self.assertEqual(coverage["enumeration_complete"], [])
                self.assertTrue(coverage["contradictions"])

    def test_opened_wheel_descriptor_must_match_its_frozen_path(self) -> None:
        opened = (
            'openat(AT_FDCWD, "/stage/wheels/a.whl", O_RDONLY) = 8</stage/wheels/b.whl>'
        )
        coverage = self.assert_rejected(
            self.trace(self.data, self.a, self.b, opened, self.eof)
        )
        self.assertEqual(coverage["unexpected_paths"], [])
        self.assertIn(
            "opened catalog path differs",
            [item["reason"] for item in coverage["contradictions"]],
        )

    def test_unexpected_directory_fd_is_not_hidden_by_a_normalized_target(self) -> None:
        lookup = self.a.replace(
            '4</stage/wheels>, "a.whl"', '7</stage/wheels/nested>, "../a.whl"'
        )
        coverage = self.assert_rejected(
            self.trace(self.data, self.a, self.b, lookup, self.eof)
        )
        self.assertEqual(coverage["unexpected_paths"], ["/stage/wheels/nested"])

    def test_statx_enosys_may_fall_back_to_a_successful_lookup(self) -> None:
        unsupported = self.b.replace("= 0", "= -1 ENOSYS (Function not implemented)")
        fallback = self.a.replace('"a.whl"', '"b.whl"').replace(
            "st_size=3", "st_size=5"
        )
        attribution = self.evidence(
            self.trace(self.data, self.a, unsupported, fallback, self.eof)
        )
        self.require_gate(attribution)
        self.assertEqual(attribution["catalog_coverage"]["contradictions"], [])

    def assert_termination_rejected(self, text: str) -> dict:
        attribution = self.evidence(text, complete_process=False)
        self.assertTrue(attribution["complete"], attribution["unparsed_lines"])
        self.assertTrue(attribution["catalog_coverage"]["accepted"])
        with self.assertRaisesRegex(ValueError, "incomplete tracee termination"):
            self.require_gate(attribution)
        return attribution["tracee_termination"]

    def test_after_eof_truncation_requires_the_checked_tracee_exit(self) -> None:
        text = self.initial_exec + self.trace(self.data, self.a, self.b, self.eof)
        termination = self.assert_termination_rejected(text)
        self.assertEqual(termination["execve"]["process"], "10")
        self.assertIsNone(termination["terminal"])

    def test_wrong_or_failed_initial_execve_cannot_bind_the_tracee(self) -> None:
        body = self.trace(self.data, self.a, self.b, self.eof)
        for initial in (
            "",
            self.initial_exec.replace('execve("/checked/uv"', 'execve("/other/uv"'),
            self.initial_exec.replace(
                "= 0 <", "= -1 ENOENT (No such file or directory) <"
            ),
            self.initial_exec.removeprefix("10 "),
        ):
            with self.subTest(initial=initial):
                self.assert_termination_rejected(initial + body + self.terminal_exit)

    def test_tracee_exit_must_match_pid_and_zero_status(self) -> None:
        text = self.initial_exec + self.trace(self.data, self.a, self.b, self.eof)
        for terminal in (
            self.terminal_exit.replace("10 ", "11 ", 1),
            self.terminal_exit.replace("exited with 0", "exited with 7"),
            self.terminal_exit.replace("exited with 0", "killed by SIGKILL"),
            self.terminal_exit.replace(
                "exited with 0", "killed by SIGSEGV (core dumped)"
            ),
            "10 1789130001.000000 --- SIGCHLD {si_status=0} ---\n",
        ):
            with self.subTest(terminal=terminal):
                self.assert_termination_rejected(text + terminal)

    def test_ambiguous_or_replaced_tracee_image_is_rejected(self) -> None:
        body = self.trace(self.data, self.a, self.b, self.eof)
        for later_exec in (
            self.initial_exec,
            self.initial_exec.replace("10 ", "11 ", 1),
            self.initial_exec.replace(
                'execve("/checked/uv"', 'execve("/other/program"'
            ),
        ):
            with self.subTest(later_exec=later_exec):
                self.assert_termination_rejected(
                    self.initial_exec + body + later_exec + self.terminal_exit
                )

    def test_bound_tracee_activity_after_termination_is_rejected(self) -> None:
        text = self.initial_exec + self.trace(self.data, self.a, self.b, self.eof)
        for tail in (self.terminal_exit, self.trace("close(8</elsewhere>) = 0")):
            with self.subTest(tail=tail):
                self.assert_termination_rejected(text + self.terminal_exit + tail)

    def test_resumed_initial_exec_and_later_child_exit_are_accepted(self) -> None:
        initial = self.initial_exec.replace(
            ") = 0 <0.000010>\n",
            " <unfinished ...>\n10 1789130000.000000 <... execve resumed>) = 0 <0.000010>\n",
        )
        child = self.initial_exec.replace("10 ", "11 ", 1).replace(
            "/checked/uv", "/python"
        )
        text = (
            initial
            + self.trace(self.data, self.a, self.b, self.eof)
            + child
            + self.terminal_exit
            + self.terminal_exit.replace("10 ", "11 ", 1)
        )
        attribution = self.evidence(text, complete_process=False)
        self.require_gate(attribution)
        termination = attribution["tracee_termination"]
        self.assertEqual(termination["expected_executable"], "/checked/uv")
        self.assertEqual(termination["terminal"]["process"], "10")
        self.assertEqual(termination["terminal"]["exit_code"], 0)
        self.assertNotIn("execve", attribution["process_calls"])
        self.assertNotIn("process_exit", attribution["process_calls"])

    def test_unknown_supersession_event_keeps_the_parser_incomplete(self) -> None:
        text = (
            self.initial_exec
            + self.trace(self.data, self.a, self.b, self.eof)
            + "10 1789130000.999999 +++ superseded by execve in pid 11 +++\n"
            + self.terminal_exit
        )
        attribution = self.evidence(text, complete_process=False)
        self.assertFalse(attribution["complete"])
        with self.assertRaisesRegex(ValueError, "incomplete pathname attribution"):
            self.require_gate(attribution)


if __name__ == "__main__":
    unittest.main()
