import json
import os
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import differentials
import kernel_profiles


class DifferentialTests(unittest.TestCase):
    def test_kernel_helper_stops_on_eof_and_rejects_startup_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "fake-perf"
            for fail in (False, True):
                with self.subTest(fail=fail):
                    executable.write_text(
                        f"#!{sys.executable}\n"
                        "import signal, sys, time\n"
                        f"if {fail!r}: sys.exit(7)\n"
                        "signal.signal(signal.SIGINT, lambda *_: sys.exit(0))\n"
                        "while True: time.sleep(0.01)\n"
                    )
                    executable.chmod(0o700)
                    environment = {
                        **os.environ,
                        "PYTHONPATH": str(Path(kernel_profiles.__file__).parent),
                    }
                    with subprocess.Popen(
                        [
                            sys.executable,
                            kernel_profiles.__file__,
                            "--capture",
                            str(executable),
                            str(root / "unused.data"),
                        ],
                        stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        text=True,
                        env=environment,
                    ) as process:
                        self.assertTrue(select.select([process.stdout], [], [], 5)[0])
                        ready = process.stdout.readline().strip()
                        process.stdin.close()
                        process.stdin = None
                        _, stderr = process.communicate(timeout=5)
                        if fail:
                            self.assertNotEqual(process.returncode, 0)
                            self.assertIn("exited during startup", stderr)
                        else:
                            self.assertEqual(ready, "ready")
                            self.assertEqual(process.returncode, 0, stderr)

    @unittest.skipUnless(os.environ.get("RCA_PERF"), "Installed Linux perf required")
    def test_live_kernel_profile_exports_reports_and_removes_raw_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(kernel_profiles, "RESULTS", root):
                with kernel_profiles.KernelProfile("smoke") as profile:
                    # Keep the probe active across multiple sampling intervals.
                    deadline = time.monotonic() + 0.5
                    while time.monotonic() < deadline:
                        os.stat("/proc/self/stat")
                capture = json.loads((root / "smoke.capture.json").read_text())
                self.assertEqual(capture["returncode"], 0)
                self.assertTrue((root / "smoke.symbols.txt").exists())
                self.assertFalse(profile.raw.exists())

    def test_log_summary_preserves_test_identity(self):
        summary = differentials.summarize_log(
            "\x1b[32mCompiling uv v1.0 (/work/uv)\x1b[0m\n"
            "PASS [ 0.001s] (1/2) uv-a test::a\n"
            "PASS [ 0.002s] (2/2) uv-b test::b\n"
            "Summary [ 0.100s] 2 tests run: 2 passed, 4 skipped\n"
        )
        self.assertEqual(summary["compiled_crates"], ["uv v1.0 (/work/uv)"])
        self.assertEqual(summary["passing_test_count"], 2)
        reordered = differentials.summarize_log(
            "PASS [ 1.000s] (1/2) uv-b test::b\nPASS [ 2.000s] (2/2) uv-a test::a\n"
        )
        self.assertEqual(
            summary["pass_identity_sha256"], reordered["pass_identity_sha256"]
        )

    @unittest.skipUnless(sys.platform == "linux", "GNU time is required")
    def test_failed_phase_keeps_its_evidence(self):
        # A nonzero process must not turn into a successful timing observation.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                patch.object(differentials, "RESULTS", root),
                self.assertRaisesRegex(RuntimeError, "exit code 7"),
            ):
                differentials.run_phase("failed", ["sh", "-c", "exit 7"])
            self.assertTrue((root / "failed.json").is_file())
            self.assertTrue((root / "failed.log").is_file())


if __name__ == "__main__":
    unittest.main()
