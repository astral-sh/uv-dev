"""Differential tests against the pinned Rust implementation, using only stdlib."""

import argparse
import hashlib
import importlib.util
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
import uv_build_slim as slim
from uv_build_slim import _generated as g
from uv_build_slim._runtime import _NONE, RustError, _method, _Some

SCRATCH = Path.home() / "code/tmp/uv-build-slim-poc"
REFERENCE = SCRATCH / "target/debug/uv-build"
ORACLE = SCRATCH / "target/debug/uv-build-slim-oracle"
FRONTEND = SCRATCH / "target/debug/uv-build-slim-codegen"
RANDOM = random.Random(20261013)
COUNTS = {"normalization": 0, "metadata": 0, "unicode": 0, "wheels": 0}


@contextmanager
def cwd(path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)


def native_requests(requests):
    data = "".join(
        json.dumps(request, ensure_ascii=True) + "\n" for request in requests
    )
    result = subprocess.run(
        [str(ORACLE)], input=data, text=True, capture_output=True, check=True
    )
    responses = [json.loads(line) for line in result.stdout.split("\n") if line]
    if len(responses) != len(requests):
        raise AssertionError("Native oracle response count mismatch")
    return responses


def python_metadata(fields):
    converted = dict(fields)
    defaults = g._Metadata23()
    for name, value in converted.items():
        if name == "keywords":
            converted[name] = _NONE if value is None else _Some(g._Keywords(value))
        elif name == "project_urls":
            converted[name] = g._ProjectUrls(dict(sorted(value.items())))
        elif getattr(defaults, name) is _NONE:
            converted[name] = _NONE if value is None else _Some(value)
    return str(g._rs_Metadata23__core_metadata_format(g._Metadata23(**converted)))


class GeneratedParity(unittest.TestCase):
    def test_name_normalization_against_rust(self):
        alphabet = "abcXYZ09._-é☃\n\r "
        values = ["", "Foo.BAR__baz", "a", "a--b", "a-", "0", "Éclair", "a\0b"]
        values += [
            "".join(RANDOM.choices(alphabet, k=RANDOM.randrange(0, 70)))
            for _ in range(1500)
        ]
        responses = native_requests(
            [{"operation": "normalize", "value": value} for value in values]
        )
        for value, expected in zip(values, responses):
            with self.subTest(name=value):
                try:
                    normalized = str(g._rs_validate_and_normalize_ref(value))
                    dist_info = str(
                        g._rs_PackageName__as_dist_info_name(
                            SimpleNamespace(_0=normalized)
                        )
                    )
                    actual = {"ok": {"normalized": normalized, "dist_info": dist_info}}
                except RustError as error:
                    actual = {"error": str(error)}
                self.assertEqual(actual, expected)
        COUNTS["normalization"] = len(values)

    def test_metadata_serializer_against_rust(self):
        values = [
            None,
            "",
            "plain",
            "\n",
            "\r",
            "a\r\nb\n",
            "a\nb\n\n",
            "雪\u2028x",
            "a\vb\fc\x85d",
            "a\rb",
        ]
        cases = []
        for value in values:
            cases.append(
                {
                    "metadata_version": "2.3",
                    "name": "Example",
                    "version": "1.0",
                    "summary": value,
                    "license": value,
                    "description": value,
                }
            )
        optional = [
            "summary",
            "home_page",
            "download_url",
            "author",
            "author_email",
            "maintainer",
            "maintainer_email",
            "license",
            "license_expression",
            "requires_python",
            "description",
            "description_content_type",
        ]
        arrays = [
            "platforms",
            "supported_platforms",
            "classifiers",
            "requires_dist",
            "provides_dist",
            "obsoletes_dist",
            "requires_external",
            "provides_extra",
            "license_files",
            "import_names",
            "import_namespaces",
            "dynamic",
        ]
        for _ in range(250):
            fields = {"metadata_version": "2.5", "name": "Test", "version": "1.0"}
            fields.update({name: RANDOM.choice(values) for name in optional})
            fields.update(
                {
                    name: [
                        RANDOM.choice(values[1:]) for _ in range(RANDOM.randrange(4))
                    ]
                    for name in arrays
                }
            )
            fields["keywords"] = RANDOM.choice(
                [None, [], [""], ["a", "", "b,c"], ["雪", "a\r\nb"]]
            )
            fields["project_urls"] = {
                "Zebra": "https://example.org/z",
                "Alpha": RANDOM.choice(values[1:]),
            }
            cases.append(fields)
        expected = native_requests(
            [{"operation": "metadata", "value": fields} for fields in cases]
        )
        for index, (fields, response) in enumerate(zip(cases, expected)):
            with self.subTest(case=index):
                self.assertEqual({"ok": python_metadata(fields)}, response)
        COUNTS["metadata"] = len(cases)

    def test_unicode_table_boundaries_against_rust(self):
        from uv_build_slim._runtime import _ALPHANUMERIC

        codes = {0, 0x10FFFF}
        for start, end in _ALPHANUMERIC:
            codes.update((start - 1, start, end, end + 1))
        codes.update(RANDOM.randrange(0x110000) for _ in range(1000))
        codes = sorted(
            code
            for code in codes
            if 0 <= code <= 0x10FFFF and not 0xD800 <= code <= 0xDFFF
        )
        expected = native_requests(
            [{"operation": "alphanumeric", "value": code} for code in codes]
        )
        for code, response in zip(codes, expected):
            self.assertEqual(
                {"ok": _method(chr(code), "is_alphanumeric")}, response, hex(code)
            )
        COUNTS["unicode"] = len(codes)


class ArtifactParity(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(
            prefix="parity-", dir=SCRATCH / "tmp"
        )
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.rust_output = self.root / "rust"
        self.python_output = self.root / "python"
        self.rust_output.mkdir()
        self.python_output.mkdir()
        self.write_project()

    def write_project(
        self, extra="", settings="", name="Demo_Pkg", module="src/demo_pkg/__init__.py"
    ):
        text = "[project]\nname = " + json.dumps(name) + '\nversion = "1.2.3"\n' + extra
        text += '\n[build-system]\nrequires = ["uv_build>=0.13,<0.14"]\nbuild-backend = "uv_build"\n'
        text += settings
        (self.project / "pyproject.toml").write_text(text)
        path = self.project / module
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'VALUE = "hello"\n')

    def native(self, command, output=None, metadata=None, check=True):
        args = [str(REFERENCE), command, str(output or self.rust_output)]
        if metadata is not None:
            args.append(str(metadata))
        result = subprocess.run(
            args, cwd=self.project, capture_output=True, text=True, check=False
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def compare_wheel(self, editable=False, metadata=None):
        command = "build-editable" if editable else "build-wheel"
        expected = self.native(command, metadata=metadata).stdout.strip()
        with cwd(self.project):
            hook = slim.build_editable if editable else slim.build_wheel
            actual = hook(
                str(self.python_output),
                metadata_directory=str(metadata) if metadata else None,
            )
        self.assertEqual(actual, expected)
        with (
            zipfile.ZipFile(self.rust_output / expected) as rust,
            zipfile.ZipFile(self.python_output / actual) as python,
        ):
            self.assertEqual(python.namelist(), rust.namelist())
            for name in rust.namelist():
                with self.subTest(member=name):
                    self.assertEqual(python.read(name), rust.read(name))
                    left, right = python.getinfo(name), rust.getinfo(name)
                    self.assertEqual(
                        left.external_attr >> 16, right.external_attr >> 16
                    )
                    self.assertEqual(left.compress_type, right.compress_type)
        COUNTS["wheels"] += 1
        return self.python_output / actual

    def test_default_src_layout_files_record_and_permissions(self):
        module = self.project / "src/demo_pkg"
        (module / "nested").mkdir()
        (module / "nested/data.bin").write_bytes(bytes(range(256)))
        (module / "nested/script.py").write_text("print('run')\n")
        (module / "nested/script.py").chmod(0o755)
        for name in (
            "comma,name",
            'quote"name',
            "line\nname",
            "carriage\rname",
            "雪.txt",
        ):
            (module / name).write_bytes(b"payload\r\n")
        (module / "__pycache__").mkdir()
        (module / "__pycache__/hidden").write_text("excluded")
        (module / "ignored.pyc").write_text("excluded")
        (module / "ignored.pyo").write_text("excluded")
        self.compare_wheel()

    def test_rich_metadata_and_entry_points(self):
        self.write_project("""description = "Example ☃"
requires-python = ">=3.11"
readme = "README.md"
license = {text = "line one\\r\\nline two\\n"}
keywords = ["python", "", "comma,keyword"]
authors = [{name="Alice"}, {email="bob@example.org"}, {name="Chloé", email="c@example.org"}]
maintainers = [{name="Maintainer", email="m@example.org"}]
[project.urls]
Zebra = "https://example.org/z"
Alpha = "https://example.org/a"
[project.scripts]
demo = "demo_pkg:main"
[project.gui-scripts]
window = "demo_pkg:window"
[project.entry-points."demo.plugins"]
"non compliant" = "demo_pkg:plugin"
""")
        (self.project / "README.md").write_bytes("# README\r\n雪\r\n".encode())
        self.compare_wheel()

    def test_empty_option_values(self):
        self.write_project("""description = ""
readme = {text = "", content-type = "text/plain"}
keywords = []
license = {text = ""}
authors = []
[project.scripts]
[project.gui-scripts]
[project.entry-points]
""")
        self.compare_wheel()

    def test_flat_layout_explicit_module(self):
        self.write_project(
            settings='\n[tool.uv.build-backend]\nmodule-root = ""\nmodule-name = "Custom"\n',
            module="Custom/__init__.py",
        )
        self.compare_wheel()

    def test_dotted_namespace(self):
        self.write_project(
            settings='\n[tool.uv.build-backend]\nmodule-name = "cloud.demo"\n',
            module="src/cloud/demo/__init__.py",
        )
        self.compare_wheel()

    def test_stub_package(self):
        self.write_project(
            name="Demo.Pkg-stubs", module="src/demo_pkg-stubs/__init__.pyi"
        )
        self.compare_wheel()

    def test_disable_default_excludes(self):
        self.write_project(
            settings="\n[tool.uv.build-backend]\ndefault-excludes = false\n"
        )
        (self.project / "src/demo_pkg/retained.pyc").write_bytes(b"compiled")
        self.compare_wheel()

    def test_editable_wheel(self):
        self.compare_wheel(editable=True)

    def test_backend_builds_itself(self):
        self.project = ROOT
        self.compare_wheel()

    def test_prepare_metadata_then_build(self):
        self.write_project('[project.scripts]\ndemo = "demo_pkg:main"\n')
        rust_metadata = self.root / "rust-metadata"
        python_metadata = self.root / "python-metadata"
        rust_metadata.mkdir()
        python_metadata.mkdir()
        expected = self.native(
            "prepare-metadata-for-build-wheel", rust_metadata
        ).stdout.strip()
        with cwd(self.project):
            actual = slim.prepare_metadata_for_build_wheel(str(python_metadata))
        self.assertEqual(actual, expected)
        self.assertEqual(
            sorted(path.name for path in (rust_metadata / expected).iterdir()),
            sorted(path.name for path in (python_metadata / actual).iterdir()),
        )
        for path in (rust_metadata / expected).iterdir():
            self.assertEqual(
                (python_metadata / actual / path.name).read_bytes(), path.read_bytes()
            )
        self.compare_wheel(metadata=python_metadata / actual)
        self.write_project(
            'description = "Changed"\n[project.scripts]\ndemo = "demo_pkg:main"\n'
        )
        result = self.native(
            "build-wheel", metadata=python_metadata / actual, check=False
        )
        self.assertNotEqual(result.returncode, 0)
        with (
            cwd(self.project),
            self.assertRaisesRegex(RustError, "Inconsistent metadata"),
        ):
            slim.build_wheel(
                str(self.python_output),
                metadata_directory=str(python_metadata / actual),
            )

    def test_entry_point_rejections_match_rust(self):
        cases = [
            '[project.scripts]\n""="x:main"\n',
            '[project.scripts]\n"..."="x:main"\n',
            '[project.scripts]\n"with space"="x:main"\n',
            '[project.entry-points."bad-group"]\nx="x:main"\n',
            '[project.entry-points.console_scripts]\nx="x:main"\n',
            '[project.entry-points.gui_scripts]\nx="x:main"\n',
        ]
        for case in cases:
            with self.subTest(case=case):
                self.write_project(case)
                result = self.native("build-wheel", check=False)
                self.assertNotEqual(result.returncode, 0)
                with cwd(self.project), self.assertRaises(RustError) as raised:
                    slim.build_wheel(str(self.python_output))
                self.assertIn(str(raised.exception), result.stderr)

    def test_unsupported_inputs_fail_without_an_artifact(self):
        cases = [
            'dependencies=["requests>=2"]\n',
            'version_override="unknown"\n',
            'license="MIT"\n',
            'requires-python=">=3.10,<4"\n',
            'classifiers=["Programming Language :: Python"]\n',
        ]
        for case in cases:
            with self.subTest(case=case):
                self.write_project(case)
                with cwd(self.project), self.assertRaises(slim.UnsupportedOperation):
                    slim.build_wheel(str(self.python_output))
                self.assertEqual(list(self.python_output.iterdir()), [])
        with self.assertRaises(slim.UnsupportedOperation):
            slim.build_sdist(str(self.python_output))

    def test_symlink_fails_without_an_artifact(self):
        (self.project / "src/demo_pkg/link.py").symlink_to("__init__.py")
        with cwd(self.project), self.assertRaises(slim.UnsupportedOperation):
            slim.build_wheel(str(self.python_output))
        self.assertEqual(list(self.python_output.iterdir()), [])


class CodeGeneration(unittest.TestCase):
    def test_untranslated_helper_is_rejected_before_writing(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH / "tmp") as temporary:
            vendor = Path(temporary) / "vendor"
            shutil.copytree(ROOT / "vendor", vendor)
            source = vendor / "normalize.rs"
            source.write_text(
                source.read_text().replace(
                    "fn normalize(name: &str) -> Result<String, InvalidNameError> {",
                    "fn normalize(name: &str) -> Result<String, InvalidNameError> { let ignored = untranslated_helper();",
                )
            )
            manifest = json.loads((vendor / "manifest.json").read_text())
            for entry in manifest["files"]:
                if entry["path"] == source.name:
                    entry["sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (vendor / "manifest.json").write_text(json.dumps(manifest))
            generated_ast = subprocess.check_output([str(FRONTEND), str(vendor)])
            output = Path(temporary) / "rejected.py"
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "codegen/generate.py"),
                    "--ast",
                    "/dev/stdin",
                    "--vendor",
                    str(vendor),
                    "--output",
                    str(output),
                ],
                input=generated_ast,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                b"Untranslated Rust symbol: untranslated_helper", result.stderr
            )
            self.assertFalse(output.exists())

    def test_regeneration_is_reproducible(self):
        generated_ast = subprocess.check_output([str(FRONTEND), str(ROOT / "vendor")])
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "codegen/generate.py"),
                "--ast",
                "/dev/stdin",
                "--check",
            ],
            input=generated_ast,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_rust_source_change_changes_generated_behavior(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH / "tmp") as temporary:
            vendor = Path(temporary) / "vendor"
            shutil.copytree(ROOT / "vendor", vendor)
            source = vendor / "metadata23.rs"
            original = source.read_text()
            updated = original.replace(
                '"Metadata-Version", &self.metadata_version',
                '"Changed-Header", &self.metadata_version',
            )
            self.assertNotEqual(updated, original)
            source.write_text(updated)
            manifest = json.loads((vendor / "manifest.json").read_text())
            for entry in manifest["files"]:
                if entry["path"] == source.name:
                    entry["sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (vendor / "manifest.json").write_text(json.dumps(manifest))
            generated_ast = subprocess.check_output([str(FRONTEND), str(vendor)])
            output = Path(temporary) / "changed.py"
            subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "codegen/generate.py"),
                    "--ast",
                    "/dev/stdin",
                    "--vendor",
                    str(vendor),
                    "--output",
                    str(output),
                ],
                input=generated_ast,
                capture_output=True,
                check=True,
            )
            spec = importlib.util.spec_from_file_location(
                "uv_build_slim._changed", output
            )
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            self.assertTrue(
                str(
                    module._rs_Metadata23__core_metadata_format(
                        module._Metadata23(metadata_version="2.3")
                    )
                ).startswith("Changed-Header: 2.3\n")
            )

    def test_untranslated_rust_syntax_is_rejected(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH / "tmp") as temporary:
            vendor = Path(temporary) / "vendor"
            shutil.copytree(ROOT / "vendor", vendor)
            source = vendor / "normalize.rs"
            source.write_text(
                source.read_text().replace(
                    "fn normalize(name: &str) -> Result<String, InvalidNameError> {",
                    "fn normalize(name: &str) -> Result<String, InvalidNameError> { unsupported_new_macro!();",
                )
            )
            result = subprocess.run(
                [str(FRONTEND), str(vendor)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Unsupported Rust syntax", result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", type=Path, default=REFERENCE)
    parser.add_argument("--oracle", type=Path, default=ORACLE)
    parser.add_argument("--frontend", type=Path, default=FRONTEND)
    parser.add_argument("--report", type=Path)
    args, remaining = parser.parse_known_args()
    REFERENCE, ORACLE, FRONTEND = args.reference, args.oracle, args.frontend
    for executable in (REFERENCE, ORACLE, FRONTEND):
        if not executable.is_file():
            raise SystemExit(f"Missing native test executable: {executable}")
    (SCRATCH / "tmp").mkdir(parents=True, exist_ok=True)
    result = unittest.main(argv=[sys.argv[0], *remaining], exit=False)
    if args.report:
        args.report.write_text(
            json.dumps(
                {
                    "revision": g.UPSTREAM_REVISION,
                    "tests": result.result.testsRun,
                    "success": result.result.wasSuccessful(),
                    "cases": COUNTS,
                },
                indent=2,
            )
            + "\n"
        )
    raise SystemExit(not result.result.wasSuccessful())
