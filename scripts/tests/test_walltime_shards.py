"""Keep every built benchmark in exactly one non-empty walltime shard."""

import importlib.util
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


if __name__ == "__main__":
    unittest.main()
