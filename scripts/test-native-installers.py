# /// script
# requires-python = ">=3.12"
# dependencies = ["chevron-blue==0.3.0"]
# ///
"""Exercise native installer rendering, target selection, and asset integrity."""

import hashlib
import importlib.util
import io
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "native_installers", ROOT / "scripts/generate-native-installers.py"
)
assert SPEC and SPEC.loader
INSTALLERS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLERS)


def executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


class NativeInstallers(unittest.TestCase):
    def test_checksum_inventory_rendering(self) -> None:
        checksums = {
            "x86_64-unknown-linux-gnu": "a" * 64,
            "x86_64-pc-windows-msvc": "b" * 64,
            "aarch64-pc-windows-msvc": "c" * 64,
        }
        rendered = INSTALLERS.render_installers("1.2.3", "astral-sh/uv", checksums)
        self.assertEqual(
            re.findall(
                r"^        (\S+)\) checksum='([a-f0-9]{64})' ;;$",
                rendered["uv-installer.sh"],
                re.MULTILINE,
            ),
            sorted(checksums.items()),
        )
        self.assertEqual(
            re.findall(
                r"^    '(\S+)' = '([a-f0-9]{64})'$",
                rendered["uv-installer.ps1"],
                re.MULTILINE,
            ),
            sorted(
                (target, checksum)
                for target, checksum in checksums.items()
                if target.endswith("-pc-windows-msvc")
            ),
        )

    def test_missing_template_variable(self) -> None:
        read_text = Path.read_text

        def with_missing_variable(path: Path, *args, **kwargs) -> str:
            contents = read_text(path, *args, **kwargs)
            if path.name == "uv-installer.sh.in":
                return contents + "\n{{missing_variable}}\n"
            return contents

        with (
            patch.object(Path, "read_text", with_missing_variable),
            self.assertRaisesRegex(KeyError, "missing_variable"),
        ):
            INSTALLERS.render_installers("1.2.3", "astral-sh/uv", {})

    def test_rejects_script_injection(self) -> None:
        for tag, repository, checksums in (
            ("1.2.3'", "astral-sh/uv", {}),
            ("1.2.3", "astral-sh/uv;echo bad", {}),
            ("1.2.3", "astral-sh/uv", {"$(bad)": "a" * 64}),
            ("1.2.3", "astral-sh/uv", {"x86_64-unknown-linux-gnu": "oops"}),
        ):
            with self.assertRaises(ValueError):
                INSTALLERS.render_installers(tag, repository, checksums)

    @unittest.skipIf(os.name == "nt", "requires POSIX sh")
    def test_target_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "tools"
            tools.mkdir()
            executable(
                tools / "uname",
                '#!/bin/sh\ncase "$1" in -s) echo "$TEST_OS" ;; -m) echo "$TEST_ARCH" ;; esac\n',
            )
            executable(
                tools / "getconf",
                '#!/bin/sh\ncase "$1" in LONG_BIT) echo "${TEST_BITS:-64}" ;; GNU_LIBC_VERSION) [ -z "$TEST_GLIBC" ] || echo "glibc $TEST_GLIBC" ;; esac\n',
            )
            executable(tools / "sysctl", '#!/bin/sh\necho "${TEST_ARM64:-0}"\n')
            installer = root / "install.sh"
            installer.write_text(
                INSTALLERS.render_installers("1.2.3", "astral-sh/uv", {})[
                    "uv-installer.sh"
                ],
                encoding="utf-8",
            )
            for system, arch, glibc, extra, expected in (
                ("Darwin", "arm64", "", {}, "aarch64-apple-darwin"),
                ("Darwin", "x86_64", "", {"TEST_ARM64": "1"}, "aarch64-apple-darwin"),
                ("Linux", "x86_64", "2.17", {}, "x86_64-unknown-linux-gnu"),
                ("Linux", "x86_64", "2.16", {}, "x86_64-unknown-linux-musl"),
                ("Linux", "aarch64", "2.27", {}, "aarch64-unknown-linux-musl"),
                ("Linux", "aarch64", "2.28", {}, "aarch64-unknown-linux-gnu"),
                ("Linux", "riscv64", "2.30", {}, "riscv64gc-unknown-linux-musl"),
                ("Linux", "riscv64", "2.31", {}, "riscv64gc-unknown-linux-gnu"),
                ("Linux", "armv7l", "", {}, "armv7-unknown-linux-musleabihf"),
                (
                    "Linux",
                    "armv6l",
                    "2.31",
                    {"TEST_BITS": "32"},
                    "arm-unknown-linux-musleabihf",
                ),
                (
                    "Linux",
                    "x86_64",
                    "2.17",
                    {"TEST_BITS": "32"},
                    "i686-unknown-linux-gnu",
                ),
                (
                    "MINGW64_NT",
                    "x86_64",
                    "",
                    {"PROCESSOR_ARCHITECTURE": "ARM64"},
                    "aarch64-pc-windows-msvc",
                ),
            ):
                with self.subTest(system=system, arch=arch, glibc=glibc, extra=extra):
                    env = {
                        **os.environ,
                        "PATH": f"{tools}{os.pathsep}{os.defpath}",
                        "TEST_OS": system,
                        "TEST_ARCH": arch,
                        "TEST_GLIBC": glibc,
                        "TEST_BITS": "64",
                        "TEST_ARM64": "0",
                        "PROCESSOR_ARCHITECTURE": "",
                        "PROCESSOR_ARCHITEW6432": "",
                        **extra,
                    }
                    output = subprocess.check_output(
                        ["sh", str(installer), "--print-target"], env=env, text=True
                    )
                    self.assertEqual(output.strip(), expected)

    @unittest.skipIf(os.name == "nt", "requires POSIX sh")
    def test_bootstrap_delegates_and_rejects_corruption(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = "x86_64-unknown-linux-gnu"
            archive = root / "distribution.tar.gz"
            program = b'#!/bin/sh\nprintf "%s\\n" "$@" > "$TEST_ARGUMENTS"\n'
            with tarfile.open(archive, "w:gz") as tar:
                member = tarfile.TarInfo(f"uv-{target}/uv")
                member.mode = 0o755
                member.size = len(program)
                tar.addfile(member, io.BytesIO(program))
            checksums = {target: hashlib.sha256(archive.read_bytes()).hexdigest()}
            installer = root / "install.sh"
            installer.write_text(
                INSTALLERS.render_installers("1.2.3", "example/uv", checksums)[
                    "uv-installer.sh"
                ],
                encoding="utf-8",
            )
            tools = root / "tools"
            tools.mkdir()
            executable(
                tools / "uname",
                '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; esac\n',
            )
            executable(
                tools / "getconf",
                '#!/bin/sh\ncase "$1" in LONG_BIT) echo 64 ;; GNU_LIBC_VERSION) echo "glibc 2.17" ;; esac\n',
            )
            executable(
                tools / "curl",
                '#!/bin/sh\nwhile [ "$1" != -o ]; do shift; done\ncp "$TEST_ARCHIVE" "$2"\n',
            )
            arguments = root / "arguments"
            env = {
                **os.environ,
                "PATH": f"{tools}{os.pathsep}{os.defpath}",
                "TMPDIR": str(root),
                "TEST_ARCHIVE": str(archive),
                "TEST_ARGUMENTS": str(arguments),
                "UV_DOWNLOAD_URL": "https://example.test/releases/1.2.3",
            }
            subprocess.run(
                ["sh", str(installer), "--no-modify-path"], env=env, check=True
            )
            self.assertEqual(
                arguments.read_text().splitlines(),
                [
                    "self",
                    "install",
                    "--preview-features",
                    "self-management",
                    "--source-repository",
                    "example/uv",
                    "--no-modify-path",
                ],
            )
            arguments.unlink()
            archive.write_bytes(b"corrupt")
            failed = subprocess.run(
                ["sh", str(installer)],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn("checksum mismatch", failed.stderr)
            self.assertFalse(arguments.exists())
            self.assertFalse(list(root.glob("uv-installer.*")))

    def test_powershell_parses(self) -> None:
        shell = shutil.which("pwsh") or shutil.which("powershell")
        if not shell:
            self.skipTest("PowerShell is not installed")
        with tempfile.TemporaryDirectory() as temporary:
            installer = Path(temporary) / "install.ps1"
            installer.write_text(
                INSTALLERS.render_installers(
                    "1.2.3", "astral-sh/uv", {"x86_64-pc-windows-msvc": "a" * 64}
                )["uv-installer.ps1"],
                encoding="utf-8",
            )
            subprocess.run(
                [
                    shell,
                    "-NoProfile",
                    "-Command",
                    "$tokens = $null; $errors = $null; [void][System.Management.Automation.Language.Parser]::ParseFile($env:UV_TEST_INSTALLER, [ref]$tokens, [ref]$errors); if ($errors.Count) { throw ($errors | Out-String) }",
                ],
                env={**os.environ, "UV_TEST_INSTALLER": str(installer)},
                check=True,
            )

    def test_archive_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = "uv-aarch64-apple-darwin.tar.gz"
            (root / name).write_bytes(b"archive")
            checksum = hashlib.sha256(b"archive").hexdigest()
            (root / f"{name}.sha256").write_text(f"{checksum}  {name}\n")
            plan = {
                "releases": [{"app_name": "uv", "artifacts": [name]}],
                "artifacts": {
                    name: {
                        "kind": "executable-zip",
                        "target_triples": ["aarch64-apple-darwin"],
                        "checksum": f"{name}.sha256",
                    }
                },
            }
            self.assertEqual(
                INSTALLERS.release_archive_checksums(plan, root),
                ({"aarch64-apple-darwin": checksum}, {name: checksum}),
            )
            (root / name).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum differs"):
                INSTALLERS.release_archive_checksums(plan, root)

    def test_global_artifacts_retain_manifest_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "tracked").write_text("source\n")
            subprocess.run(["git", "init", "--quiet", str(source)], check=True)
            subprocess.run(["git", "-C", str(source), "add", "tracked"], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(source),
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.test",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "Fixture",
                ],
                check=True,
            )
            artifacts = {
                "source.tar.gz": {
                    "kind": "source-tarball",
                    "checksum": "source.tar.gz.sha256",
                },
                "source.tar.gz.sha256": {"kind": "checksum"},
                "uv-installer.sh": {"kind": "installer"},
                "uv-installer.ps1": {"kind": "installer"},
                "sha256.sum": {"kind": "unified-checksum"},
            }
            for target, extension in (
                ("aarch64-apple-darwin", "tar.gz"),
                ("x86_64-pc-windows-msvc", "zip"),
            ):
                name = f"uv-{target}.{extension}"
                (root / name).write_bytes(target.encode())
                checksum = hashlib.sha256(target.encode()).hexdigest()
                (root / f"{name}.sha256").write_text(f"{checksum}  {name}\n")
                artifacts[name] = {
                    "kind": "executable-zip",
                    "target_triples": [target],
                    "checksum": f"{name}.sha256",
                }
                artifacts[f"{name}.sha256"] = {"kind": "checksum"}
            plan = {
                "announcement_tag": "1.2.3",
                "releases": [
                    {
                        "app_name": "uv",
                        "app_version": "1.2.3",
                        "artifacts": list(artifacts),
                        "hosting": {"github": {"owner": "astral-sh", "repo": "uv"}},
                    }
                ],
                "artifacts": artifacts,
            }
            generated = INSTALLERS.generate(plan, root, source)
            self.assertEqual(generated["releases"], plan["releases"])
            self.assertEqual(
                {Path(path).name for path in generated["upload_files"]},
                set(INSTALLERS.GLOBALS),
            )
            with tarfile.open(root / "source.tar.gz") as archive:
                self.assertIn("uv-1.2.3/tracked", archive.getnames())
            self.assertIn("source.tar.gz", (root / "sha256.sum").read_text())
            self.assertNotIn("{{", (root / "uv-installer.sh").read_text())


if __name__ == "__main__":
    unittest.main()
