"""Keep every built benchmark in exactly one non-empty walltime shard."""

import copy
import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "walltime_shards",
    Path(__file__).resolve().parents[1] / "benchmark/walltime-shards.py",
)
assert SPEC is not None and SPEC.loader is not None
shards = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shards)


class WalltimeShards(unittest.TestCase):
    def test_small_and_large_suites_are_complete(self):
        for count in (1, 2, 8, 9, 61):
            with self.subTest(count=count):
                names = [f"suite_{index}" for index in range(count)]
                plan = shards.partition(names)
                self.assertEqual(len(plan), min(count, shards.MAX_SHARDS))
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

    def test_invalid_sets_are_rejected(self):
        for names in ([], ["uv", "uv"], ["--all"], ["../uv"]):
            with self.subTest(names=names), self.assertRaises(ValueError):
                shards.partition(names)


class WalltimePlan(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifacts = {}
        for index in range(9):
            name = f"suite_{index}"
            path = self.root / name
            path.write_bytes(name.encode())
            self.artifacts[name] = path
        self.source = {"commit": "a" * 40, "tree": "b" * 40}
        self.plan = shards.prepare_plan(self.source, {}, self.artifacts)

    def test_selected_artifacts_match_the_producer(self):
        selected = shards.verify_plan(self.plan, self.source, self.artifacts, 1)
        self.assertEqual(selected["benches"], ["suite_0", "suite_8"])
        self.artifacts["suite_8"].write_bytes(b"different binary")
        with self.assertRaisesRegex(ValueError, "artifact does not match.*suite_8"):
            shards.verify_plan(self.plan, self.source, self.artifacts, 1)

    def test_each_shard_checks_only_the_artifacts_it_executes(self):
        self.artifacts["suite_1"].write_bytes(b"different binary")
        shards.verify_plan(self.plan, self.source, self.artifacts, 1)
        with self.assertRaisesRegex(ValueError, "artifact does not match.*suite_1"):
            shards.verify_plan(self.plan, self.source, self.artifacts, 2)

    def test_source_inventory_and_plan_changes_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "different tracked source"):
            shards.verify_plan(
                self.plan, dict(self.source, commit="c" * 40), self.artifacts, 1
            )

        for mutate in (
            lambda plan: plan.update(version=1),
            lambda plan: plan["shards"].reverse(),
            lambda plan: plan["shards"][0]["benches"].pop(),
            lambda plan: plan["artifacts"].pop("suite_8"),
            lambda plan: plan["artifacts"].update(unexpected={}),
        ):
            plan = copy.deepcopy(self.plan)
            mutate(plan)
            with self.subTest(plan=plan), self.assertRaises(ValueError):
                shards.verify_plan(plan, self.source, self.artifacts, 1)

        for index in (0, 9):
            error = self.assertRaisesRegex(ValueError, "Shard index")
            with self.subTest(index=index), error:
                shards.verify_plan(self.plan, self.source, self.artifacts, index)

    def test_source_identity_tracks_real_committed_and_modified_files(self):
        source = self.root / "source"
        source.mkdir()
        (source / "Cargo.lock").write_text("version = 4\n")
        (source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "stable"\n')
        subprocess.run(["git", "init", "--quiet", str(source)], check=True)
        subprocess.run(
            ["git", "add", "Cargo.lock", "rust-toolchain.toml"], cwd=source, check=True
        )
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                f"core.hooksPath={os.devnull}",
                "commit",
                "--quiet",
                "-m",
                "Fixture",
            ],
            cwd=source,
            check=True,
        )
        original = shards.source_identity(source)
        self.assertFalse(original["tracked_working_tree_dirty"])
        (source / "generated-input").write_text("generated\n")
        self.assertEqual(shards.source_identity(source), original)
        (source / "Cargo.lock").write_text("version = 3\n")
        modified = shards.source_identity(source)
        self.assertTrue(modified["tracked_working_tree_dirty"])
        self.assertEqual(modified["commit"], original["commit"])
        self.assertNotEqual(
            modified["tracked_diff_sha256"], original["tracked_diff_sha256"]
        )
        self.assertNotEqual(
            modified["cargo_lock_sha256"], original["cargo_lock_sha256"]
        )


if __name__ == "__main__":
    unittest.main()
