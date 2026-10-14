"""Public-hook parity at filesystem and validation boundaries."""

import contextlib
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
import uv_build_slim_wasm as backend

NATIVE = Path(
    os.environ.get(
        "UV_SLIM_REFERENCE",
        Path.home() / "code/tmp/uv-build-slim-poc/target/debug/uv-build",
    )
)
SCRATCH = Path(os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp"))
if os.environ.get("UV_SLIM_PROGRAM"):
    backend._PROGRAM = backend.Program(Path(os.environ["UV_SLIM_PROGRAM"]))


class Parity(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=str(SCRATCH))
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        (self.source / "src/demo").mkdir(parents=True)
        (self.source / "src/demo/__init__.py").write_bytes(b"VALUE = 1\n")
        self.project()
        self.before = Path.cwd()
        os.chdir(self.source)
        self.addCleanup(os.chdir, self.before)
        self.native = self.root / "native"
        self.native.mkdir()
        self.python = self.root / "python"
        self.python.mkdir()

    def project(self, extra="", settings=""):
        (self.source / "pyproject.toml").write_text(
            '[project]\nname="demo"\nversion="1.0"\n'
            + extra
            + '\n[build-system]\nrequires=["uv_build>=0.13,<0.14"]\nbuild-backend="uv_build"\n'
            + settings
        )

    def check(self, command="build-wheel", success=True):
        expected = subprocess.run(
            [str(NATIVE), command, str(self.native)],
            text=True,
            capture_output=True,
            check=False,
        )
        error = io.StringIO()
        result = None
        status = 0
        with contextlib.redirect_stderr(error):
            try:
                result = backend.call([command, str(self.python)])
            except SystemExit as stopped:
                status = stopped.code
        self.assertEqual(status, expected.returncode)
        self.assertEqual(status == 0, success, expected.stderr)
        self.assertEqual(
            error.getvalue().replace(str(self.python), str(self.native)),
            expected.stderr,
        )
        if success:
            self.assertEqual(result, expected.stdout.strip())
            self.assertEqual(
                (self.python / result).read_bytes(), (self.native / result).read_bytes()
            )

    def test_unicode_filenames(self):
        (self.source / "src/demo/雪☃.txt").write_text("text")
        self.check()
        self.check("build-sdist")

    def test_symlink_file_and_unicode_target(self):
        (self.source / "src/demo/target-雪.txt").write_text("target")
        link = self.source / "src/demo/link.txt"
        link.symlink_to("target-雪.txt")
        if not link.is_symlink():
            self.skipTest("The filesystem did not create the requested symbolic link")
        self.check()
        self.check("build-sdist")

    def test_dangling_symlink(self):
        link = self.source / "src/demo/missing-link"
        link.symlink_to("absent")
        if not link.is_symlink():
            self.skipTest("The filesystem did not create the requested symbolic link")
        self.check(success=False)

    def test_missing_readme(self):
        self.project('readme="missing.md"\n')
        self.check(success=False)

    def test_readme_is_directory(self):
        self.project('readme={file="directory",content-type="text/plain"}\n')
        (self.source / "directory").mkdir()
        self.check(success=False)

    def test_invalid_utf8_readme(self):
        self.project('readme="README.md"\n')
        (self.source / "README.md").write_bytes(b"\xff\xfe")
        self.check(success=False)

    def test_spdx_dependencies_and_import_names(self):
        self.project("""license="MIT OR Apache-2.0"
dependencies=["Some_Package[Foo.BAR]>=1.2; python_version < '3.14'", "demo-url @ https://example.org/a.whl"]
import-names=["demo; private"]
import-namespaces=["namespace.demo"]
[project.optional-dependencies]
Test_Extra=["pytest>=8", "coverage; sys_platform == 'linux'"]
""")
        self.check()
        self.check("build-sdist")

    def test_invalid_spdx(self):
        self.project('license="Not-A-License"\n')
        self.check(success=False)

    def test_invalid_glob_hint(self):
        self.project(settings='\n[tool.uv.build-backend]\nsource-include=["**/@bad"]\n')
        self.check("build-sdist", success=False)

    @unittest.skipIf(os.name == "nt", "Windows filesystem paths use UTF-16")
    def test_non_utf8_filename(self):
        path = os.fsencode(self.source / "src/demo") + b"/bad-\xff.txt"
        with open(path, "wb") as stream:
            stream.write(b"contents")
        self.check()

    @unittest.skipIf(os.name == "nt", "Windows filesystem paths use UTF-16")
    def test_non_utf8_working_directory(self):
        previous = self.source
        renamed = previous.with_name("source-\udcff")
        os.chdir(self.root)
        previous.rename(renamed)
        self.source = renamed
        os.chdir(renamed)
        self.check()

    def test_unicode_working_directory(self):
        renamed = self.source.with_name("source-雪☃")
        os.chdir(self.root)
        self.source.rename(renamed)
        self.source = renamed
        os.chdir(renamed)
        self.check()
        self.check("build-sdist")

    def test_unicode_output_directory(self):
        self.native = self.root / "native-雪☃"
        self.python = self.root / "python-雪☃"
        self.native.mkdir()
        self.python.mkdir()
        self.check()

    def test_pathlike_output_directory(self):
        expected = subprocess.check_output(
            [str(NATIVE), "build-wheel", self.native], text=True
        ).strip()
        actual = backend.build_wheel(self.python)
        self.assertEqual(actual, expected)
        self.assertEqual(
            (self.python / actual).read_bytes(), (self.native / expected).read_bytes()
        )

    def test_null_argument(self):
        message = "embedded null character" if os.name == "nt" else "embedded null byte"
        with self.assertRaisesRegex(ValueError, message):
            subprocess.run([str(NATIVE), "build-wheel", "bad\0path"], check=False)
        with self.assertRaisesRegex(ValueError, message):
            backend.build_wheel("bad\0path")


if __name__ == "__main__":
    unittest.main()
