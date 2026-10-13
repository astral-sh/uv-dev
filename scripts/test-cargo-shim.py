#!/usr/bin/env python3
"""Exercise the Cargo shim against dependency-free projects on Unix and Windows."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHIM = ROOT / "scripts" / ("cargo.cmd" if os.name == "nt" else "cargo.sh")


class CargoShimTests(unittest.TestCase):
    toolchain: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.toolchain = subprocess.check_output(
            ["rustup", "show", "active-toolchain"], cwd=ROOT, text=True
        ).split()[0]

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="cargo shim ")
        self.addCleanup(self.directory.cleanup)
        self.project = Path(self.directory.name)
        self.manifest = self.project / "Cargo.toml"
        self.manifest.write_text(
            '[package]\nname = "shim-test"\nversion = "0.1.0"\nedition = "2021"\n',
            encoding="utf-8",
        )
        (self.project / "src").mkdir()
        (self.project / "src/main.rs").write_text(
            'fn main() { println!("{:?}", std::env::args().skip(1).collect::<Vec<_>>()); }\n',
            encoding="utf-8",
        )
        self.lockfile = self.project / "Cargo.lock"
        self.environment = {
            **os.environ,
            "CARGO_AUDITABLE": "0",
            "CARGO_LOCKED": "1",
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(self.project / "target"),
            "CARGO_TERM_COLOR": "never",
            "RUSTUP_TOOLCHAIN": self.toolchain,
        }

    def cargo(
        self, *arguments: str, locked: str = "1", auditable: str = "0"
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(SHIM), *arguments],
            cwd=self.project,
            env={
                **self.environment,
                "CARGO_LOCKED": locked,
                "CARGO_AUDITABLE": auditable,
            },
            capture_output=True,
            text=True,
            check=False,
        )

    def generate_lockfile(self) -> None:
        result = self.cargo("generate-lockfile", locked="0")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_lockfile(self) -> None:
        result = self.cargo("check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertFalse(self.lockfile.exists())

    @unittest.skipUnless(os.environ.get("CARGO"), "requires setup-cargo")
    def test_path_invocation(self) -> None:
        result = subprocess.run(
            "cargo check",
            shell=True,
            cwd=self.project,
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertFalse(self.lockfile.exists())

    def test_stale_lockfile(self) -> None:
        self.generate_lockfile()
        locked = self.lockfile.read_bytes()
        self.manifest.write_text(
            self.manifest.read_text(encoding="utf-8").replace("0.1.0", "0.2.0"),
            encoding="utf-8",
        )
        result = self.cargo("metadata", "--format-version", "1", auditable="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertEqual(self.lockfile.read_bytes(), locked)

    def test_explicit_locked(self) -> None:
        self.generate_lockfile()
        result = self.cargo(
            "--verbose", "--locked", "metadata", "--format-version", "1"
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_explicit_frozen(self) -> None:
        self.generate_lockfile()
        result = self.cargo("--frozen", "metadata", "--format-version", "1")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_unlocked_update(self) -> None:
        self.generate_lockfile()
        self.manifest.write_text(
            self.manifest.read_text(encoding="utf-8").replace("0.1.0", "0.2.0"),
            encoding="utf-8",
        )
        result = self.cargo("check", locked="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "0.2.0"', self.lockfile.read_text(encoding="utf-8"))

    def test_run_arguments_do_not_disable_locking(self) -> None:
        result = self.cargo("run", "--", "--locked")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertFalse(self.lockfile.exists())

    def test_alias_and_run_arguments(self) -> None:
        self.generate_lockfile()
        (self.project / ".cargo").mkdir()
        (self.project / ".cargo/config.toml").write_text(
            '[alias]\ndev = "run --quiet --"\n', encoding="utf-8"
        )
        result = self.cargo("dev", "two words", "--locked")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '["two words", "--locked"]\n')

    def test_alias_arguments_do_not_disable_locking(self) -> None:
        (self.project / ".cargo").mkdir()
        (self.project / ".cargo/config.toml").write_text(
            '[alias]\ndev = "run --quiet --"\n', encoding="utf-8"
        )
        result = self.cargo("dev", "--locked")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertFalse(self.lockfile.exists())

    @unittest.skipIf(
        os.name == "nt", "Windows selects toolchains with RUSTUP_TOOLCHAIN"
    )
    def test_toolchain_override(self) -> None:
        result = self.cargo(f"+{self.toolchain}", "check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--locked", result.stderr)
        self.assertFalse(self.lockfile.exists())


if __name__ == "__main__":
    unittest.main()
