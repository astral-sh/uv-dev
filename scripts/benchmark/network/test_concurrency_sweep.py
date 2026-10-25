"""Check source identity, pairing, and equivalence in configuration calibration."""

from __future__ import annotations

import argparse
import copy
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
            "frontend_protocols": None,
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

    def verified_data(self, *, warmups=0):
        data = self.data([1, 16, 50, 100], warmups=warmups)
        data.update(
            kind="download-concurrency-calibration",
            revision="a" * 40,
            binary={"sha256": "b" * 64, "version": "uv test (aaaaaaaaa)"},
            profile={"latency_ms": 100},
            netem={"rtt_ms": 200, "rate_mbit": 1},
            selected=[
                {"index_bytes": 50, "metadata_bytes": 250, "wheel_bytes": 1000000}
            ],
            workload="resolve",
            http2_proxy=None,
        )
        for limit, result in data["results"].items():
            result["strategy_floor"] = sweep.strategy_floor(
                data["profile"], data["netem"], data["selected"], "resolve", int(limit)
            )
        self.assertEqual(data["results"]["1"]["strategy_floor"]["seconds"], 0.6)
        with patch.object(sweep.bench, "run_one", return_value=self.observation(1)):
            sweep.run_pairs(
                self.binary, None, {}, self.trial, data, self.root / "result.json"
            )
        data["complete"] = True
        return data

    def test_verifier_uses_recorded_network_and_rejects_drift(self) -> None:
        data = self.verified_data()
        sweep.verify_calibration(data)
        data["results"]["1"]["pairs"][0]["candidate"]["seconds"] = 2
        with self.assertRaisesRegex(ValueError, "measurements differ"):
            sweep.verify_calibration(data)

    def test_verifier_checks_shuffled_pair_order_after_warmups(self) -> None:
        for warmups in (0, 1, 3):
            with self.subTest(warmups=warmups):
                data = self.verified_data(warmups=warmups)
                sweep.verify_calibration(data)
                pair = data["results"]["16"]["pairs"][1]
                data["results"]["16"]["pairs"][1] = dict(reversed(pair.items()))
                with self.assertRaisesRegex(ValueError, "run order differs"):
                    sweep.verify_calibration(data)

    def test_verifier_requires_valid_order_parameters(self) -> None:
        original = self.verified_data()
        for key, value in (
            ("warmups", None),
            ("warmups", -1),
            ("warmups", True),
            ("pairs_per_limit", True),
            ("order_seed", "42"),
        ):
            with self.subTest(key=key, value=value):
                data = copy.deepcopy(original)
                data[key] = value
                with self.assertRaisesRegex(ValueError, "identities or limits differ"):
                    sweep.verify_calibration(data)


if __name__ == "__main__":
    unittest.main()
