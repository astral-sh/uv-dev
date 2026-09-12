"""Require complete walltime shard sets when importing public benchmark profiles."""

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


class CodspeedShards(unittest.TestCase):
    def test_legacy_jobs(self):
        self.assertEqual(
            profiles.expected_source_jobs(set(profiles.SOURCE_JOBS.values())),
            profiles.SOURCE_JOBS,
        )

    def test_complete_shards(self):
        for total in (1, 2, 8):
            with self.subTest(total=total):
                jobs = {
                    f"bench / walltime on aarch64 linux ({index}/{total})"
                    for index in range(1, total + 1)
                }
                expected = profiles.expected_source_jobs(jobs | {"bench / simulated"})
                self.assertEqual(
                    set(expected),
                    {
                        "simulation",
                        *(f"walltime-{index}" for index in range(1, total + 1)),
                    },
                )

    def test_partial_or_mixed_shards_are_rejected(self):
        for jobs in (
            set(),
            {"bench / walltime on aarch64 linux (1/2)"},
            {
                "bench / walltime on aarch64 linux (1/2)",
                "bench / walltime on aarch64 linux (2/3)",
            },
            {"bench / walltime on aarch64 linux (0/1)"},
            {
                f"bench / walltime on aarch64 linux ({index}/9)"
                for index in range(1, 10)
            },
        ):
            with self.subTest(jobs=jobs):
                self.assertIsNone(
                    profiles.expected_source_jobs(jobs | {"bench / simulated"})
                )

    def test_find_requires_every_shard_and_artifact(self):
        sha = "a" * 40
        run = {
            "id": 23,
            "head_sha": sha,
            "head_branch": "main",
            "event": "push",
            "path": ".github/workflows/ci.yml",
            "repository": {"full_name": "astral-sh/uv"},
            "head_repository": {"full_name": "astral-sh/uv"},
        }
        jobs = [
            {"name": "bench / simulated", "conclusion": "success"},
            *(
                {
                    "name": f"bench / walltime on aarch64 linux ({index}/2)",
                    "conclusion": "success",
                }
                for index in (1, 2)
            ),
        ]
        artifacts = [
            {"name": profiles.ARTIFACTS[mode], "expired": False}
            for mode in ("simulation", "walltime-1", "walltime-2")
        ]

        def items(path, key):
            return jobs if key == "jobs" else artifacts

        with (
            patch.object(profiles, "github_object", return_value=run),
            patch.object(profiles, "github_items", side_effect=items),
        ):
            self.assertEqual(profiles.find_source_run(sha, "23"), "23")
            artifacts.pop()
            self.assertIsNone(profiles.find_source_run(sha, "23"))
            jobs.pop()
            self.assertIsNone(profiles.find_source_run(sha, "23"))


if __name__ == "__main__":
    unittest.main()
