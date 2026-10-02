"""Check full-wheel upgrade references and their conservative network bounds."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location(
    "tool_upgrade_oracle", HERE / "tool_upgrade_oracle.py"
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class ToolUpgradeOracleTests(unittest.TestCase):
    def test_distinct_indexes_and_shared_wheels(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            driver = [
                "uv",
                "--no-config",
                "run",
                "--no-project",
                "--offline",
                "--python",
                sys.executable,
                "python",
                "-S",
            ]
            subprocess.run(
                [
                    *driver,
                    str(HERE / "make_tool_upgrade_fixtures.py"),
                    "--directory",
                    str(directory),
                    "--packages",
                    "3",
                    "--shared-dependencies",
                    "1",
                    "--index-groups",
                    "2",
                ],
                capture_output=True,
                check=True,
            )
            profiles = json.loads(
                (directory / "tool-upgrade-profiles.json").read_text()
            )
            scenarios = json.loads(
                (directory / "tool-upgrade-descriptor.json").read_text()
            )["scenarios"]
            fixtures = oracle.bench.Fixtures(
                directory / "tool-upgrade-fixtures.json", directory, True
            )
            tasks = oracle.requests(fixtures, profiles["fast"], scenarios["upgrade"])
            pages = [task for task in tasks if not task["parents"]]
            wheels = [task for task in tasks if task["parents"]]
            self.assertEqual((len(pages), len(wheels)), (5, 4))
            self.assertEqual(len({task["path"] for task in tasks}), 9)
            self.assertTrue(all(len(task["parents"]) == 1 for task in wheels))
            self.assertTrue(
                all(next(iter(task["parents"])) < len(pages) for task in wheels)
            )
            required = sum(len(task["expected"]) for task in pages) + sum(
                (directory / filename).stat().st_size
                for filename in scenarios["upgrade"]["selected_wheels"]
            )
            bound = oracle.bounds(
                tasks, profiles["slow"], 2, netem={"rtt_ms": 200, "rate_mbit": 10}
            )
            self.assertEqual(bound["required_bytes"], required)
            self.assertEqual(bound["required_waves"], 5)
            self.assertEqual(bound["required_latency_ms"], 750)
            self.assertEqual(
                bound["optimistic_network_floor_seconds"], max(required / 1250000, 1.75)
            )
            current = oracle.requests(fixtures, profiles["fast"], scenarios["current"])
            self.assertEqual(current, pages)
            for mode in ("pinned", "local", "empty"):
                self.assertEqual(
                    oracle.requests(fixtures, profiles["fast"], scenarios[mode]), []
                )
            self.assertEqual(
                oracle.bounds([], profiles["slow"], 2, netem={})[
                    "optimistic_network_floor_seconds"
                ],
                0,
            )
            output = directory / "result.json"
            subprocess.run(
                [
                    *driver,
                    str(HERE / "tool_upgrade_oracle.py"),
                    "--directory",
                    str(directory),
                    "--profile",
                    "fast",
                    "--scenario",
                    "upgrade",
                    "--concurrency",
                    "2",
                    "--output",
                    str(output),
                ],
                capture_output=True,
                check=True,
            )
            result = json.loads(output.read_text())
            self.assertEqual(result["requests"], 9)
            self.assertEqual(result["origin_connections"], 2)
            self.assertEqual(result["required_bytes"], required)
            self.assertEqual(result["actual_bytes"], required)
            self.assertEqual(result["required_waves"], 5)
            self.assertTrue(all(event["status"] == 200 for event in result["events"]))

    def test_request_capacity_bound_allows_pipeline_overlap(self) -> None:
        tasks = [
            {"path": f"/page/{number}", "expected": b"a", "parents": set()}
            for number in range(3)
        ] + [
            {"path": f"/wheel/{number}", "expected": b"b", "parents": {number}}
            for number in range(3)
        ]
        bound = oracle.bounds(tasks, {"latency_ms": 100}, 2, netem={})
        self.assertEqual(bound["required_waves"], 3)
        self.assertEqual(bound["optimistic_network_floor_seconds"], 0.3)
        with self.assertRaises(ValueError):
            oracle.bounds(tasks, {}, 0)

    def test_selected_metadata_latency_is_part_of_the_bound(self) -> None:
        tasks = [
            {"path": "/page/root", "expected": b"a", "parents": set()},
            {"path": "/page/shared", "expected": b"a", "parents": set()},
            {"path": "/wheel/shared", "expected": b"b", "parents": {1}},
        ]
        bound = oracle.bounds(
            tasks,
            {"path_latency_ms": {"/page/shared": 150}, "jitter_ms": 10},
            50,
            netem={},
        )
        self.assertEqual(bound["required_waves"], 2)
        self.assertEqual(bound["required_latency_ms"], 140)
        self.assertEqual(bound["optimistic_network_floor_seconds"], 0.14)

    def test_rtt_and_weighted_capacity_bounds_are_not_added(self) -> None:
        tasks = [
            {"path": f"/page/{number}", "expected": b"a", "parents": set()}
            for number in range(9)
        ] + [
            {"path": f"/wheel/{number}", "expected": b"b", "parents": {number}}
            for number in range(9)
        ]
        bound = oracle.bounds(
            tasks,
            {"path_latency_ms": {"/page/0": 1000}},
            2,
            netem={"rtt_ms": 100},
        )
        self.assertEqual(bound["required_waves"], 9)
        self.assertEqual(bound["required_latency_ms"], 500)
        self.assertEqual(bound["optimistic_latency_bound_ms"], 1400)
        self.assertEqual(bound["optimistic_network_floor_seconds"], 1.4)

    def test_reference_requires_dependency_order(self) -> None:
        with self.assertRaisesRegex(ValueError, "dependency order"):
            oracle.bounds(
                [{"path": "/wheel", "expected": b"a", "parents": {0}}],
                {},
                1,
                netem={},
            )


if __name__ == "__main__":
    unittest.main()
