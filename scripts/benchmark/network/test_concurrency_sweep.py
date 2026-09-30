"""Check source identity, pairing, and equivalence in configuration calibration."""

from __future__ import annotations

import argparse
import importlib.util
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "network_concurrency_sweep", Path(__file__).with_name("concurrency_sweep.py")
)
assert spec is not None and spec.loader is not None
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


class ConcurrencySweepTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp")
        self.root = Path(self.temporary.name)
        self.binary = self.root / "uv"
        self.trial = argparse.Namespace(env={"unchanged": "value"})

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def data(self, limits, *, pairs=2, warmups=0):
        return {
            "limits": limits,
            "reference_limit": 50,
            "order_seed": 42,
            "pairs_per_limit": pairs,
            "warmups": warmups,
            "results": {str(limit): {"pairs": []} for limit in limits},
        }

    @staticmethod
    def observation(seconds, output="same"):
        return {
            "seconds": seconds,
            "stdout_sha256": output,
            "verified_tree": {"sha256": "installed", "entries": 10},
            "verified_files": {},
        }

    def test_limits_are_paired_with_the_same_binary(self) -> None:
        calls = []

        def run(binary, fixtures, profile, args):
            self.assertEqual(binary, self.binary)
            limit = int(args.env["UV_CONCURRENT_DOWNLOADS"])
            self.assertEqual(set(args.env), {"UV_CONCURRENT_DOWNLOADS"})
            calls.append(limit)
            return self.observation(1 + 1 / limit)

        data = self.data([1, 2, 50], warmups=1)
        with patch.object(sweep.bench, "run_one", side_effect=run):
            sweep.run_pairs(
                self.binary, None, {}, self.trial, data, self.root / "result.json"
            )
        self.assertEqual(len(calls), 18)
        for offset in range(0, len(calls), 2):
            self.assertIn(50, calls[offset : offset + 2])
        self.assertEqual(self.trial.env, {"unchanged": "value"})
        for limit in [1, 2, 50]:
            result = data["results"][str(limit)]
            self.assertEqual(len(result["pairs"]), 2)
            self.assertEqual(result["summary"]["pairs"], 2)
            self.assertAlmostEqual(
                result["summary"]["median_paired_ratio"], (1 + 1 / limit) / 1.02
            )
            self.assertNotIn("qualifies_5_percent", result["summary"])

    def test_equivalence_is_required_between_pairs(self) -> None:
        observations = [self.observation(1, value) for value in ["a", "a", "b", "b"]]
        with (
            patch.object(sweep.bench, "run_one", side_effect=observations),
            self.assertRaisesRegex(ValueError, "different stdout_sha256"),
        ):
            sweep.run_pairs(
                self.binary,
                None,
                {},
                self.trial,
                self.data([1]),
                self.root / "result.json",
            )

    def test_manifest_requires_one_wheel_per_package(self) -> None:
        first = self.root / "example-1.0-py3-none-any.whl"
        first.write_bytes(b"first")
        second = self.root / "example-2.0-py3-none-any.whl"
        second.write_bytes(b"second")
        fixtures = SimpleNamespace(
            files={first.name: first},
            metadata={first.name + ".metadata": b"Name: Example\nVersion: 1.0\n"},
            simple={"example": b"index"},
        )
        selected = sweep.selections(fixtures)
        self.assertEqual(selected[0]["requirement"], "example==1.0")
        self.assertEqual(selected[0]["wheel_bytes"], 5)
        self.assertEqual(selected[0]["index_bytes"], 5)
        fixtures.files[second.name] = second
        fixtures.metadata[second.name + ".metadata"] = b"Name: example\nVersion: 2.0\n"
        with self.assertRaisesRegex(ValueError, "More than one wheel"):
            sweep.selections(fixtures)


if __name__ == "__main__":
    unittest.main()
