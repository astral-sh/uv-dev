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
        with tempfile.TemporaryDirectory() as directory:
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


if __name__ == "__main__":
    unittest.main()
