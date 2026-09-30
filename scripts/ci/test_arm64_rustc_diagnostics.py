"""Exercise the compiler observer with real exit and signal statuses."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("arm64-rustc-diagnostics.py")
WRAPPER = SCRIPT.with_name("arm64-rustc-wrapper.sh")


class CompilerDiagnosticsTest(unittest.TestCase):
    def setUp(self):
        temporary_root = Path.home() / "code" / "tmp"
        temporary_root.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="uv-rustc-diagnostic-test-", dir=temporary_root
        )
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = dict(os.environ) | {
            "UV_DIAGNOSTIC_REAL_RUSTC": sys.executable,
            "UV_DIAGNOSTIC_RESULTS": str(self.root),
            "UV_DIAGNOSTIC_PYTHON": sys.executable,
            "UV_DIAGNOSTIC_UV": shutil.which("uv") or "uv",
            "UV_DIAGNOSTIC_SCRIPT": str(SCRIPT),
            "CARGO_AUDITABLE_ORIG_ARGS": "[]",
        }
        self.environment.pop("CARGO_PRIMARY_PACKAGE", None)

    def observe(self, source, prefix=None):
        command = prefix or [sys.executable, str(SCRIPT), "rustc", "--"]
        return subprocess.run(
            [*command, "-c", source],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def record(self):
        paths = list((self.root / "rustc").glob("*.json"))
        self.assertEqual(len(paths), 1)
        return json.loads(paths[0].read_text())

    def test_preserves_stdout_and_exit_status(self):
        result = self.observe("print('compiler output'); raise SystemExit(42)")
        self.assertEqual(result.returncode, 42)
        self.assertEqual(result.stdout, "compiler output\n")
        record = self.record()
        self.assertEqual(record["returncode"], 42)
        self.assertNotIn("signal", record)
        self.assertGreater(record["max_rss_bytes"], 0)

    def test_reports_sigkill(self):
        result = self.observe("import os, signal; os.kill(os.getpid(), signal.SIGKILL)")
        self.assertEqual(result.returncode, 137)
        record = self.record()
        self.assertEqual(record["returncode"], -9)
        self.assertEqual(record["signal"], 9)
        self.assertEqual(record["signal_name"], "SIGKILL")
        self.assertIn("RUSTC_DIAGNOSTIC", result.stderr)

    def test_build_failure_keeps_memory_evidence(self):
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "build",
                "--label",
                "test",
                "--",
                sys.executable,
                "-c",
                "raise SystemExit(3)",
            ],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 3)
        self.assertEqual(
            json.loads((self.root / "test-result.json").read_text())["returncode"], 3
        )
        self.assertTrue((self.root / "test-memory.jsonl").read_text())
        self.assertIn("COMPILER_REPORT", result.stdout)

    @unittest.skipUnless(
        os.environ.get("AUDITABLE_TEST_BINARY"),
        "set AUDITABLE_TEST_BINARY to test cargo-auditable",
    )
    def test_cargo_auditable_reports_original_signal(self):
        result = self.observe(
            "import os, signal; os.kill(os.getpid(), signal.SIGTERM)",
            [os.environ["AUDITABLE_TEST_BINARY"], str(WRAPPER)],
        )
        self.assertEqual(result.returncode, 143)
        self.assertEqual(self.record()["signal_name"], "SIGTERM")
        self.assertNotIn("deadly signal", result.stderr)


if __name__ == "__main__":
    unittest.main()
