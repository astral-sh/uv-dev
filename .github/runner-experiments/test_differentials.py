import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import differentials


class DifferentialTests(unittest.TestCase):
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
