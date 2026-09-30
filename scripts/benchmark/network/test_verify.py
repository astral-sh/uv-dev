"""Regression tests for archived-study manifest recognition."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_verify", Path(__file__).with_name("verify.py")
)
assert spec is not None and spec.loader is not None
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


class StudyManifestTest(unittest.TestCase):
    def test_prebuild_template_only_fills_unknown_hashes(self):
        study = {
            "parent": "a" * 40,
            "head": "b" * 40,
            "binary_sha256": {"parent": "c" * 64, "head": "d" * 64},
            "cases": [{"file": "example-slow.json", "pairs": 30}],
        }
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as directory:
            path = Path(directory) / "example-verification-template.json"
            template = json.loads(json.dumps(study))
            template["binary_sha256"]["head"] = None
            path.write_text(json.dumps(template))
            self.assertTrue(verify.is_study_spec(path, study))

            template["cases"][0]["pairs"] = 20
            path.write_text(json.dumps(template))
            self.assertFalse(verify.is_study_spec(path, study))

            template["cases"][0]["pairs"] = 30
            template["binary_sha256"]["parent"] = "e" * 64
            path.write_text(json.dumps(template))
            self.assertFalse(verify.is_study_spec(path, study))

            result = Path(directory) / "example-unlisted.json"
            result.write_text(json.dumps(study))
            self.assertFalse(verify.is_study_spec(result, study))

    def test_case_harness_identity(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as directory:
            root = Path(directory)
            original = root / "bench-original.py"
            original.write_text("version = 1\n")
            current = root / "bench.py"
            current.write_text("version = 2\n")
            case = {
                "harness_file": original.name,
                "harness_sha256": verify.sha256(original),
            }
            self.assertEqual(verify.case_harness(root, case, {}), original)
            self.assertEqual(
                verify.case_harness(
                    root, {}, {"harness_sha256": verify.sha256(current)}
                ),
                current,
            )
            with self.assertRaisesRegex(ValueError, "Recorded harness hash differs"):
                verify.case_harness(
                    root, case, {"harness_sha256": verify.sha256(current)}
                )
            with self.assertRaisesRegex(ValueError, "Missing recorded harness hash"):
                verify.case_harness(root, {**case, "requires_harness_hash": True}, {})
            with self.assertRaisesRegex(ValueError, "Invalid case harness filename"):
                verify.case_harness(root, {"harness_file": "../bench.py"}, {})
            original.write_text("version = 3\n")
            with self.assertRaisesRegex(ValueError, "Case harness hash differs"):
                verify.case_harness(root, case, {})


if __name__ == "__main__":
    unittest.main()
