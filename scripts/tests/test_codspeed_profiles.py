"""Exercise source-run discovery and independent platform import identities."""

import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "codspeed_profiles", Path(__file__).resolve().parents[1] / "codspeed-profiles.py"
)
assert SPEC is not None and SPEC.loader is not None
profiles = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(profiles)

SHA = "a" * 40
RUN = {
    "id": 23,
    "head_sha": SHA,
    "head_branch": "main",
    "event": "push",
    "path": ".github/workflows/ci.yml",
    "repository": {"full_name": "astral-sh/uv"},
    "head_repository": {"full_name": "astral-sh/uv"},
}


class ProfileImports(unittest.TestCase):
    def source_run(self, modes: dict[str, str | None], artifacts: set[str]):
        def items(path, key):
            if key == "jobs":
                return [
                    {"name": profiles.SOURCE_JOBS[mode], "conclusion": conclusion}
                    for mode, conclusion in modes.items()
                ]
            self.assertEqual(key, "artifacts")
            return [
                {"name": profiles.ARTIFACTS[mode], "expired": False}
                for mode in artifacts
            ]

        with (
            patch.object(profiles, "github_object", return_value=RUN),
            patch.object(profiles, "github_items", side_effect=items),
        ):
            return profiles.find_source_run(SHA, "23")

    def test_older_two_mode_run(self):
        modes = {mode: "success" for mode in profiles.REQUIRED_MODES}
        self.assertEqual(self.source_run(modes, set(modes)), "23")

    def test_platform_run_must_finish_and_upload(self):
        modes = {mode: "success" for mode in profiles.REQUIRED_MODES}
        modes["walltime-macos"] = None
        self.assertIsNone(self.source_run(modes, profiles.REQUIRED_MODES))
        modes["walltime-macos"] = "success"
        self.assertIsNone(self.source_run(modes, profiles.REQUIRED_MODES))
        self.assertEqual(self.source_run(modes, set(modes)), "23")

    def test_platform_artifact_needs_its_source_job(self):
        modes = {mode: "success" for mode in profiles.REQUIRED_MODES}
        self.assertIsNone(self.source_run(modes, {*modes, "walltime-macos"}))

    def test_walltime_parts_do_not_collide(self):
        source = {"version": 11, "runner": {"executor": "walltime"}}
        environment = {
            "GITHUB_REPOSITORY": "astral-sh/uv-dev",
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_JOB": "import",
            "GITHUB_RUN_ID": "42",
            "GITHUB_ACTOR_ID": "1",
            "GITHUB_ACTOR": "actor",
        }
        linux = profiles.destination_metadata(source, environment)
        macos = profiles.destination_metadata(
            source, environment, part="walltime-macos"
        )
        self.assertEqual(linux["runPart"]["runPartId"], "import-walltime")
        self.assertEqual(macos["runPart"]["runPartId"], "import-walltime-macos")
        self.assertEqual(source, {"version": 11, "runner": {"executor": "walltime"}})


if __name__ == "__main__":
    unittest.main()
