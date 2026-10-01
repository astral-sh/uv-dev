# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for the release artifact SBOM checker."""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tarfile
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from zipfile import ZipFile

SCRIPT = Path(__file__).with_name("check-release-artifact-sboms.sh")
STUB = r"""
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

root = Path(os.environ["SBOM_TEST_STATE"])
command = Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps([command, *args]) + "\n")

if command == "mktemp" and args == ["-d"]:
    print(tempfile.mkdtemp(prefix="sboms.", dir=os.environ["TMPDIR"]))
elif command == "gh" and args[:1] == ["api"]:
    assert args == ["api", "repos/{owner}/{repo}/actions/runs/456/artifacts", "--paginate", "--jq", ".artifacts[].name"]
    if os.environ.get("SBOM_TEST_FAIL") == "listing":
        print("artifact listing failed", file=sys.stderr)
        raise SystemExit(23)
    for artifact in sorted((root / "fixtures").iterdir()):
        print(artifact.name)
elif command == "gh" and args[:2] == ["run", "download"]:
    assert args[:3] == ["run", "download", "456"] and args[3] == "-n" and args[5] == "-D" and len(args) == 7
    if os.environ.get("SBOM_TEST_FAIL") == "download":
        print("artifact download failed", file=sys.stderr)
        raise SystemExit(24)
    shutil.copytree(root / "fixtures" / args[4], args[6])
elif command == "rust-audit-info":
    assert len(args) == 1
    if Path(args[0]).read_bytes() != b"auditable\n":
        raise SystemExit(17)
else:
    raise AssertionError(f"Unexpected command: {command} {args}")
"""


class ReleaseSbomTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.tmp = self.root / "tmp"
        self.tmp.mkdir()
        self.fixtures = self.root / "fixtures"
        self.fixtures.mkdir()
        for name in ("gh", "rust-audit-info", "mktemp"):
            executable = self.bin / name
            executable.write_text(f"#!{sys.executable}\n{STUB}")
            executable.chmod(0o755)
        self.environment = os.environ | {
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "TMPDIR": str(self.tmp),
            "SBOM_TEST_STATE": str(self.root),
        }

    def artifact(self, target, members, *, windows=False, filename=None):
        directory = self.fixtures / f"build-github-archives-{target}"
        directory.mkdir(exist_ok=True)
        name = filename or f"uv-{target}.{('zip' if windows else 'tar.gz')}"
        if windows:
            with ZipFile(directory / name, "w") as archive:
                for member, contents in members.items():
                    archive.writestr(member, contents)
        else:
            with tarfile.open(directory / name, "w:gz") as archive:
                for member, contents in members.items():
                    info = tarfile.TarInfo(f"uv-{target}/{member}")
                    info.size = len(contents)
                    info.mode = 0o755
                    archive.addfile(info, io.BytesIO(contents))
        return directory

    def run_script(self, *, failure=None):
        environment = self.environment
        if failure:
            environment = environment | {"SBOM_TEST_FAIL": failure}
        result = subprocess.run(
            ["bash", str(SCRIPT), "456"],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(list(self.tmp.iterdir()), [])
        return result

    def calls(self, command):
        return [
            row[1:]
            for line in (self.root / "calls.jsonl").read_text().splitlines()
            if (row := json.loads(line))[0] == command
        ]

    def test_no_matching_artifacts_fails(self):
        (self.fixtures / "unrelated-artifact").mkdir()
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("No GitHub release archive artifacts", result.stderr)
        self.assertEqual(self.calls("rust-audit-info"), [])

    def test_missing_archive_fails(self):
        directory = self.fixtures / "build-github-archives-linux"
        directory.mkdir()
        (directory / "uv").write_bytes(b"auditable\n")
        (directory / "uvx").write_bytes(b"auditable\n")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("expected one release archive", result.stdout)
        self.assertEqual(self.calls("rust-audit-info"), [])

    def test_multiple_archives_fail(self):
        self.artifact("linux", {"uv": b"auditable\n", "uvx": b"auditable\n"})
        self.artifact("linux", {}, filename="unexpected.tar.gz")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("expected one release archive", result.stdout)
        self.assertEqual(self.calls("rust-audit-info"), [])

    def test_empty_archive_fails(self):
        self.artifact("linux", {})
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing uv", result.stdout)
        self.assertIn("missing uvx", result.stdout)
        self.assertIn("PASS 0 / FAIL 2", result.stdout)

    def test_missing_uv_fails(self):
        self.artifact("linux", {"uvx": b"auditable\n"})
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing uv", result.stdout)
        self.assertIn("PASS 1 / FAIL 1", result.stdout)

    def test_missing_uvx_fails(self):
        self.artifact("linux", {"uv": b"auditable\n"})
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing uvx", result.stdout)
        self.assertIn("PASS 1 / FAIL 1", result.stdout)

    def test_complete_unix_archive_passes(self):
        self.artifact("linux", {"uv": b"auditable\n", "uvx": b"auditable\n"})
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS 2 / FAIL 0", result.stdout)
        self.assertEqual(len(self.calls("rust-audit-info")), 2)

    def test_complete_windows_archive_passes(self):
        self.artifact(
            "windows",
            {
                "uv.exe": b"auditable\n",
                "uvx.exe": b"auditable\n",
                "uvw.exe": b"auditable\n",
            },
            windows=True,
        )
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS 2 / FAIL 0", result.stdout)
        self.assertEqual(len(self.calls("rust-audit-info")), 2)

    def test_invalid_sbom_fails(self):
        self.artifact("linux", {"uv": b"auditable\n", "uvx": b"not auditable\n"})
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("PASS 1 / FAIL 1", result.stdout)

    def test_unrelated_artifact_is_ignored(self):
        (self.fixtures / "unrelated-artifact").mkdir()
        self.artifact("linux", {"uv": b"auditable\n", "uvx": b"auditable\n"})
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("rust-audit-info")), 2)

    def test_all_target_artifacts_are_checked(self):
        self.artifact("linux", {"uv": b"auditable\n", "uvx": b"auditable\n"})
        self.artifact("windows", {"uv.exe": b"auditable\n"}, windows=True)
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing uvx", result.stdout)
        self.assertIn("PASS 3 / FAIL 1", result.stdout)
        self.assertEqual(len(self.calls("rust-audit-info")), 3)

    def test_listing_failure_is_reported(self):
        result = self.run_script(failure="listing")
        self.assertEqual(result.returncode, 23)
        self.assertIn("artifact listing failed", result.stderr)

    def test_download_failure_is_reported(self):
        self.artifact("linux", {})
        result = self.run_script(failure="download")
        self.assertEqual(result.returncode, 24)
        self.assertIn("artifact download failed", result.stderr)
        self.assertEqual(self.calls("rust-audit-info"), [])


if __name__ == "__main__":
    unittest.main()
