"""Keep every built benchmark in exactly one non-empty walltime shard."""

import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "walltime_shards",
    Path(__file__).resolve().parents[1] / "benchmark/walltime-shards.py",
)
assert SPEC is not None and SPEC.loader is not None
shards = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shards)

# A representative Linux walltime build, in workflow target order.
SUITES = (
    "uv",
    "resolver_errors",
    "workspace_discovery",
    "real_workspace",
    "workspace_scripts",
    "lockfile",
    "wheel_extract",
    "wheel_install",
    "sdist_extract",
    "package_build",
    "build_frontend",
    "native_source_install",
    "source_cache_keys",
    "source_metadata",
    "source_git_cache_keys",
    "source_build_reuse",
    "source_requirements",
    "requirements_includes",
    "project_inspection",
    "installed_environment",
    "project_noop",
    "project_lock",
    "incremental_cutoff_lock",
    "member_index_lock",
    "direct_url_lock",
    "environment_reinstall",
    "concurrent_environments",
    "installed_bytecode",
    "publish_preparation",
    "python_discovery",
    "python_install",
    "python_uninstall",
    "python_concurrent_install",
    "python_bin_links",
    "cache_management",
    "virtualenv_creation",
    "concurrent_downloads",
    "local_wheel_concurrency",
    "local_wheel_cache",
    "download_hashing",
    "multi_index",
    "outdated_tree",
    "s3_signing",
    "osv_audit",
    "tls_handshake",
    "entrypoint_startup",
    "tool_list",
    "cached_tool_run",
    "git_fetch",
    "github_metadata",
    "github_revisions",
)


class WalltimeShards(unittest.TestCase):
    def assert_complete(self, names):
        plan = shards.partition(names)
        self.assertEqual(len(plan), min(len(names), shards.MAX_SHARDS))
        self.assertEqual(
            sorted(name for item in plan for name in item["benches"]),
            sorted(names),
        )
        self.assertTrue(all(item["benches"] for item in plan))
        self.assertEqual(plan, shards.partition(list(reversed(names))))
        self.assertEqual(
            [item["index"] for item in plan], list(range(1, len(plan) + 1))
        )
        self.assertTrue(all(item["total"] == len(plan) for item in plan))
        self.assertTrue(
            all(set(item) == {"index", "total", "benches"} for item in plan)
        )
        self.assertTrue(
            all(item["benches"] == sorted(item["benches"]) for item in plan)
        )
        return plan

    def test_small_and_large_suites_are_complete(self):
        for count in (1, 2, 8, 9, 61):
            with self.subTest(count=count):
                names = [f"suite_{index}" for index in range(count)]
                self.assert_complete(names)

    def test_real_suite_sets_are_complete(self):
        for count in range(1, len(SUITES) + 1):
            with self.subTest(count=count):
                self.assert_complete(list(SUITES[:count]))

    def test_equal_weights_use_name_and_shard_index(self):
        names = [f"suite_{index:02}" for index in range(19)]
        plan = self.assert_complete(list(reversed(names)))
        self.assertEqual(
            [item["benches"] for item in plan],
            [names[index :: shards.MAX_SHARDS] for index in range(shards.MAX_SHARDS)],
        )

    def test_unmeasured_suites_use_one_positive_unit(self):
        self.assertEqual(shards.DEFAULT_WEIGHT, 1)
        self.assertTrue(
            all(
                type(weight) is int and weight > 0
                for weight in shards.SUITE_WEIGHTS.values()
            )
        )

    def test_long_suites_are_placed_first(self):
        names = ["b", "a", "long", "short", "c", "d"]
        with (
            patch.object(shards, "MAX_SHARDS", 3),
            patch.dict(shards.SUITE_WEIGHTS, {"long": 4, "short": 2}, clear=True),
        ):
            plan = self.assert_complete(names)
        self.assertEqual(
            [item["benches"] for item in plan],
            [["long"], ["c", "short"], ["a", "b", "d"]],
        )

    def test_observed_long_builds_do_not_share_a_shard(self):
        names = [
            name
            for name in SUITES
            if name
            not in {
                "resolver_errors",
                "project_lock",
                "incremental_cutoff_lock",
                "member_index_lock",
                "direct_url_lock",
                "python_install",
                "python_uninstall",
                "python_concurrent_install",
                "python_bin_links",
            }
        ]
        plan = self.assert_complete(names)
        self.assertIn(["source_build_reuse"], [item["benches"] for item in plan])
        self.assertTrue(
            all(
                "source_build_reuse" not in item["benches"]
                or "source_metadata" not in item["benches"]
                for item in plan
            )
        )

    def test_invalid_sets_are_rejected(self):
        for names in ([], ["uv", "uv"], ["--all"], ["../uv"]):
            with self.subTest(names=names), self.assertRaises(ValueError):
                shards.partition(names)


if __name__ == "__main__":
    unittest.main()
