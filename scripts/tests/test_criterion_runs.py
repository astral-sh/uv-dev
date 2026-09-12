"""Check independent Criterion sample aggregation."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "criterion_runs",
    Path(__file__).resolve().parents[1] / "benchmark/criterion-runs.py",
)
assert SPEC is not None and SPEC.loader is not None
runs = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runs)


class CriterionRuns(unittest.TestCase):
    def test_normalizes_iteration_counts_and_keeps_repeats(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for repetition, times in [(1, [10, 20]), (2, [12, 24])]:
                result = directory / f"run-{repetition}" / "case" / "new"
                result.mkdir(parents=True)
                (result / "benchmark.json").write_text(json.dumps({"full_id": "case"}))
                (result / "sample.json").write_text(
                    json.dumps({"times": times, "iters": [1, 2]})
                )
                (result / "estimates.json").write_text(
                    json.dumps(
                        {
                            "median": {
                                "confidence_interval": {
                                    "lower_bound": 9,
                                    "upper_bound": 13,
                                }
                            }
                        }
                    )
                )
            [summary] = runs.summarize(directory, 2)
            self.assertEqual(summary["median_ns"], 11)
            self.assertEqual(summary["maximum_within_run_relative_stddev"], 0)
            self.assertGreater(summary["between_run_relative_stddev"], 0)
            self.assertEqual(len(summary["runs"]), 2)

    def test_rejects_missing_measurements(self):
        with (
            tempfile.TemporaryDirectory() as temporary,
            self.assertRaisesRegex(ValueError, "empty"),
        ):
            runs.summarize(Path(temporary), 2)


if __name__ == "__main__":
    unittest.main()
