"""Regression tests for archived-study manifest recognition."""

import copy
import importlib.util
import json
import shutil
import subprocess
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
    def test_nonqualifying_studies_retain_all_other_evidence_checks(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as directory:
            root = Path(directory)
            repository = root / "repository"
            evidence = root / "evidence"
            repository.mkdir()
            evidence.mkdir()

            def git(*arguments):
                return subprocess.check_output(
                    [
                        "git",
                        "-c",
                        "user.name=Network benchmark test",
                        "-c",
                        "user.email=network-test@example.com",
                        "-c",
                        "commit.gpgsign=false",
                        "-c",
                        f"core.hooksPath={root / 'no-hooks'}",
                        *arguments,
                    ],
                    cwd=repository,
                )

            git("init", "-q")
            (repository / "source").write_text("parent\n")
            git("add", "source")
            git("commit", "-qm", "parent")
            parent = git("rev-parse", "HEAD").decode().strip()
            (repository / "source").write_text("head\n")
            git("commit", "-qam", "head")
            head = git("rev-parse", "HEAD").decode().strip()
            (evidence / "change.patch").write_bytes(
                git(
                    "diff",
                    "--no-ext-diff",
                    "--no-color",
                    "--abbrev=7",
                    parent,
                    head,
                    "--",
                )
            )
            (evidence / "source.txt").write_text(head + "\n")
            hashes = {"parent": "a" * 64, "head": "b" * 64}
            (evidence / "binaries.sha256").write_text(
                "".join(f"{value}  {side}\n" for side, value in hashes.items())
            )
            shutil.copyfile(Path(__file__).with_name("bench.py"), evidence / "bench.py")
            harness_spec = importlib.util.spec_from_file_location(
                "qualification_test_bench", evidence / "bench.py"
            )
            assert harness_spec is not None and harness_spec.loader is not None
            harness = importlib.util.module_from_spec(harness_spec)
            harness_spec.loader.exec_module(harness)
            pairs = [
                {
                    side: {
                        "seconds": seconds,
                        "stdout_sha256": "c" * 64,
                        "stderr_sha256": "d" * 64,
                        "verified_tree": {"sha256": "e" * 64, "entries": 1},
                        "verified_files": {},
                        "bytes": 0,
                        "requests": 0,
                        "events": [],
                    }
                    for side, seconds in (("parent", 1.0), ("head", 1.1))
                }
                for _ in range(20)
            ]
            result = {
                "parent_sha": parent,
                "head_sha": head,
                "binaries": {
                    side: {"sha256": hashes[side], "version": revision[:9]}
                    for side, revision in (("parent", parent), ("head", head))
                },
                "pairs": pairs,
                "summary": harness.summary(pairs),
                "compare_stderr": True,
                "profile": {},
                "lower_bound": {"required_bytes": 0, "required_waves": 0, "seconds": 0},
            }
            study = {
                "parent": parent,
                "head": head,
                "binary_sha256": hashes,
                "scope": "test",
                "result_globs": ["case-*.json"],
                "cases": [
                    {
                        "file": "case-slow.json",
                        "pairs": 20,
                        "role": "primary",
                        "qualifying": True,
                        "requires_tree": True,
                    },
                    {"file": "case-fast.json", "pairs": 20, "role": "fast"},
                ],
            }
            for case in study["cases"]:
                (evidence / case["file"]).write_text(json.dumps(result))
            with self.assertRaisesRegex(ValueError, "improvement below 5%"):
                verify.verify(evidence, repository, study)
            retained = verify.verify(
                evidence, repository, study, require_qualification=False
            )
            self.assertEqual(retained["verified_pairs"], 40)
            self.assertEqual(
                retained["qualification"],
                {
                    "passed": False,
                    "required_cases": ["case-slow.json"],
                    "failed_cases": ["case-slow.json"],
                },
            )

            invalid = copy.deepcopy(result)
            invalid["pairs"][0]["head"]["exit_code"] = 1
            (evidence / "case-slow.json").write_text(json.dumps(invalid))
            with self.assertRaisesRegex(ValueError, "exit status differs"):
                verify.verify(evidence, repository, study, require_qualification=False)
            (evidence / "case-slow.json").write_text(json.dumps(result))
            invalid["pairs"][0]["head"].pop("exit_code")
            invalid["pairs"][0]["head"]["stdout_sha256"] = "f" * 64
            (evidence / "case-fast.json").write_text(json.dumps(invalid))
            with self.assertRaisesRegex(ValueError, "stdout_sha256 differs"):
                verify.verify(evidence, repository, study, require_qualification=False)

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

    def test_retained_original_template_keeps_the_measurement_plan(self):
        original_name = "example-original-verification-template.json"
        study = {
            "parent": "a" * 40,
            "head": "b" * 40,
            "binary_sha256": {"parent": "c" * 64, "head": "d" * 64},
            "cases": [{"file": "example-slow.json", "pairs": 30}],
            "result_globs": ["example-*.json"],
            "support_files": [original_name, "resume.sh"],
            "logs": ["original.log", "resume.log"],
            "validation_profile": "dev",
        }
        template = copy.deepcopy(study)
        template["binary_sha256"]["head"] = None
        template["support_files"] = []
        template["logs"] = ["original.log"]
        template.pop("validation_profile")
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as directory:
            path = Path(directory) / original_name
            path.write_text(json.dumps(template))
            self.assertTrue(verify.is_study_spec(path, study))

            unlisted = copy.deepcopy(study)
            unlisted["support_files"].remove(original_name)
            self.assertFalse(verify.is_study_spec(path, unlisted))

            for key, value in (
                ("head", "e" * 40),
                ("binary_sha256", {"parent": "f" * 64, "head": None}),
                ("cases", [{"file": "example-slow.json", "pairs": 20}]),
                ("result_globs", ["example-slow.json"]),
            ):
                with self.subTest(key=key):
                    changed = copy.deepcopy(template)
                    changed[key] = value
                    path.write_text(json.dumps(changed))
                    self.assertFalse(verify.is_study_spec(path, study))

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
