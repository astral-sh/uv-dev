"""Check walltime result retention with small local commands."""

import copy
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "benchmark/check-walltime-aggregation.py"
SPEC = importlib.util.spec_from_file_location("walltime_aggregation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
aggregation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(aggregation)


def result_value(pid, name):
    return {
        "creator": {"name": "codspeed-rust", "version": "5.0.1", "pid": pid},
        "instrument": {"type": "walltime"},
        "benchmarks": [
            {
                "name": name,
                "uri": f"test.rs::{name}",
                "config": {},
                "stats": {
                    "min_ns": 1.0,
                    "max_ns": 1.0,
                    "mean_ns": 1.0,
                    "stdev_ns": 0.0,
                    "q1_ns": 1.0,
                    "median_ns": 1.0,
                    "q3_ns": 1.0,
                    "rounds": 10,
                    "total_time": 0.00000001,
                    "iqr_outlier_rounds": 0,
                    "stdev_outlier_rounds": 0,
                    "iter_per_round": 1,
                    "warmup_iters": 0,
                },
            }
        ],
    }


def contents(pid, name):
    return (json.dumps(result_value(pid, name)) + "\n").encode()


class WalltimeAggregationMetadata(unittest.TestCase):
    def test_exact_result_creator_and_filename(self):
        data = contents(101, "hash_sha256")
        result = aggregation.describe_result("101.json", data)
        self.assertEqual(result["size"], len(data))
        self.assertEqual(result["creator"]["pid"], 101)
        self.assertEqual(result["benchmarks"][0]["name"], "hash_sha256")
        for filename in ("0.json", "0101.json", "../101.json", "result.json"):
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                aggregation.describe_result(filename, data)
        for malformed in (b"null", b"[]", b"false", b"{not json}"):
            with self.subTest(malformed=malformed), self.assertRaises(ValueError):
                aggregation.describe_result("101.json", malformed)
        for mutate in (
            lambda value: value["creator"].update(name="other"),
            lambda value: value["creator"].update(version="5.0.2"),
            lambda value: value["creator"].update(pid=102),
            lambda value: value["creator"].update(pid=True),
            lambda value: value.update(instrument={"type": "simulation"}),
            lambda value: value.update(benchmarks=[]),
            lambda value: value["benchmarks"][0].update(uri=None),
        ):
            value = result_value(101, "hash_sha256")
            mutate(value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                aggregation.describe_result("101.json", json.dumps(value).encode())

    def test_transition_retains_prior_results_and_exact_selection(self):
        first = aggregation.describe_result("101.json", contents(101, "hash_sha256"))
        second = aggregation.describe_result(
            "202.json", contents(202, "discover_workspace_from_all_members")
        )
        previous = {"101.json": first}
        current = {**previous, "202.json": second}
        self.assertEqual(
            aggregation.verify_transition(
                previous, current, "discover_workspace_from_all_members"
            ),
            "202.json",
        )
        for invalid in (
            previous,
            {"202.json": second},
            {"101.json": second, "202.json": second},
            {**current, "303.json": second},
            {**previous, "202.json": first},
        ):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                aggregation.verify_transition(
                    previous, invalid, "discover_workspace_from_all_members"
                )

    def test_nonpublishing_exact_tool_context(self):
        source = {"commit": "a" * 40}
        environment = {
            "BENCHMARK_SOURCE_SHA": source["commit"],
            "CODSPEED_SKIP_UPLOAD": "true",
        }
        consumer = {
            "inside_codspeed_runner": True,
            "codspeed_runner_mode": "walltime",
            "cargo_codspeed": {"version": "cargo-codspeed 5.0.1"},
            "codspeed": {"version": "codspeed-runner 5.0.2"},
        }
        aggregation.verify_context(source, consumer, environment)
        for mutate in (
            lambda value: value.update(inside_codspeed_runner=False),
            lambda value: value.update(codspeed_runner_mode="simulation"),
            lambda value: value["cargo_codspeed"].update(
                version="cargo-codspeed 5.0.2"
            ),
            lambda value: value["codspeed"].update(version="codspeed-runner 5.0.3"),
            lambda value: value.update(codspeed=None),
        ):
            changed = copy.deepcopy(consumer)
            mutate(changed)
            with self.subTest(consumer=changed), self.assertRaises(ValueError):
                aggregation.verify_context(source, changed, environment)
        for changed in (
            dict(environment, CODSPEED_SKIP_UPLOAD="false"),
            dict(environment, BENCHMARK_SOURCE_SHA="b" * 40),
            dict(environment, ACTIONS_ID_TOKEN_REQUEST_TOKEN=""),
            dict(environment, ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid"),
        ):
            with self.subTest(environment=changed), self.assertRaises(ValueError):
                aggregation.verify_context(source, consumer, changed)

    def test_filters_do_not_change_the_prepared_artifacts(self):
        plan = {
            "artifacts": {
                name: {"sha256": name} for name in aggregation.EXPECTED_BENCHMARKS
            }
        }
        cargo = str(Path(sys.executable).resolve())
        commands = aggregation.filtered_commands(plan, cargo=cargo)
        self.assertEqual(
            [command["name"] for command in commands],
            list(aggregation.EXPECTED_BENCHMARKS),
        )
        for command in commands:
            self.assertEqual(command["command"][0], cargo)
            self.assertEqual(command["artifact"], plan["artifacts"][command["name"]])
            self.assertEqual(
                command["command"][9:12],
                [aggregation.EXPECTED_BENCHMARKS[command["name"]], "--", "--exact"],
            )

    def test_main_uses_the_observed_cargo_invocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script = root / "scripts/benchmark/check-walltime-aggregation.py"
            profile = root / "profile"
            profile.mkdir()
            artifacts_directory = root / "artifacts"
            artifacts_directory.mkdir()
            artifacts = {}
            for name in aggregation.EXPECTED_BENCHMARKS:
                path = artifacts_directory / name
                path.write_bytes(name.encode())
                artifacts[name] = path
            source = {"commit": "a" * 40}
            plan = aggregation.shards.prepare_plan(source, {}, artifacts)
            cache = root / ".cache"
            cache.mkdir()
            (cache / "bench-walltime-shards.json").write_text(json.dumps(plan))
            cargo = str(root / "tools/cargo")
            consumer = {
                "inside_codspeed_runner": True,
                "codspeed_runner_mode": "walltime",
                "cargo": {
                    "invocation_executable": cargo,
                    "resolved_executable": str(root / "tools/rustup"),
                },
                "cargo_codspeed": {"version": "cargo-codspeed 5.0.1"},
                "codspeed": {"version": "codspeed-runner 5.0.2"},
            }
            environment = {
                "BENCHMARK_SOURCE_SHA": source["commit"],
                "CODSPEED_SKIP_UPLOAD": "true",
                "CODSPEED_PROFILE_FOLDER": str(profile),
            }
            output = root / "output"
            with ExitStack() as stack:
                stack.enter_context(
                    mock.patch.object(aggregation, "__file__", str(script))
                )
                stack.enter_context(
                    mock.patch.dict(os.environ, environment, clear=True)
                )
                stack.enter_context(
                    mock.patch.object(
                        sys, "argv", [str(script), "--output", str(output)]
                    )
                )
                stack.enter_context(
                    mock.patch.object(
                        aggregation.shards, "consumer_metadata", return_value=consumer
                    )
                )
                stack.enter_context(
                    mock.patch.object(
                        aggregation.shards, "source_identity", return_value=source
                    )
                )
                built = stack.enter_context(
                    mock.patch.object(
                        aggregation.shards, "built_artifacts", return_value=artifacts
                    )
                )
                python = stack.enter_context(
                    mock.patch.object(
                        aggregation.shards,
                        "tool_identity",
                        return_value={"version": "Python fixture"},
                    )
                )
                run = stack.enter_context(
                    mock.patch.object(aggregation, "check_aggregation", return_value=0)
                )
                with self.assertRaises(SystemExit) as raised:
                    aggregation.main()
            self.assertEqual(raised.exception.code, 0)
            built.assert_called_once_with(root, cargo=cargo, environment=mock.ANY)
            python.assert_called_once_with(
                root, sys.executable, "--version", environment=mock.ANY
            )
            self.assertEqual(
                [command["command"][0] for command in run.call_args.args[4]],
                [cargo, cargo],
            )
            self.assertEqual(dict(run.call_args.kwargs["environment"]), environment)


class WalltimeAggregationFiles(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.profile = self.root / "profile"
        self.profile.mkdir()
        self.metadata = {"source": {"commit": "a" * 40}, "plan_sha256": "b" * 64}

    def suite(self, name, *, prefix="", suffix=""):
        benchmark = aggregation.EXPECTED_BENCHMARKS[name]
        program = f"""
import json
import os
import time
from pathlib import Path
directory = Path({str(self.profile)!r}) / 'results'
directory.mkdir(exist_ok=True)
{prefix}
pid = os.getpid()
value = {result_value(1, benchmark)!r}
value['creator']['pid'] = pid
(directory / f'{{pid}}.json').write_text(json.dumps(value) + '\\n')
{suffix}
"""
        return {"name": name, "command": [sys.executable, "-I", "-B", "-c", program]}

    def run_check(self, name, commands, **options):
        directory = self.root / name
        exit_code = aggregation.check_aggregation(
            self.root,
            directory,
            self.profile,
            self.metadata,
            commands,
            environment=dict(os.environ),
            grace_seconds=0.1,
            **options,
        )
        return (
            exit_code,
            json.loads((directory / "aggregation.json").read_text()),
            json.loads((directory / "commands/result.json").read_text()),
            directory,
        )

    def final_observation(self, state, directory):
        path = directory / state["final_observation"]["manifest"]
        self.assertEqual(
            aggregation.shards.digest(path), state["final_observation"]["sha256"]
        )
        receipt = json.loads(path.read_text())
        self.assertEqual(receipt["source"], self.metadata["source"])
        self.assertEqual(receipt["plan_sha256"], self.metadata["plan_sha256"])
        self.assertEqual(receipt["command_exit_code"], state["command_exit_code"])
        aggregates = path.parent / "aggregates"
        self.assertEqual(
            {item.name for item in aggregates.iterdir()}, set(receipt["inventory"])
        )
        for filename, entry in receipt["inventory"].items():
            self.assertEqual(
                aggregation.shards.digest(aggregates / filename), entry["sha256"]
            )
        return receipt, aggregates

    def test_real_sequential_commands_retain_both_aggregates(self):
        verified = []
        exit_code, state, commands, directory = self.run_check(
            "success",
            [self.suite("uv"), self.suite("workspace_discovery")],
            before_launch=lambda suite: verified.append(suite["name"]),
        )
        self.assertEqual(exit_code, 0)
        self.assertEqual(state["status"], "success")
        self.assertEqual(state["initial_inventory"], {})
        self.assertEqual(len(state["final_inventory"]), 2)
        self.assertEqual(len(state["observations"]), 2)
        self.assertEqual(verified, list(aggregation.EXPECTED_BENCHMARKS))
        self.assertEqual(commands["successful_suites"], 2)
        observed, _ = self.final_observation(state, directory)
        self.assertEqual(observed["inventory"], state["final_inventory"])
        self.assertEqual(observed["unexpected_results"], [])
        for filename, entry in state["final_inventory"].items():
            self.assertEqual(
                aggregation.shards.digest(directory / "aggregates" / filename),
                entry["sha256"],
            )

    def test_original_nonzero_exit_retains_partial_results(self):
        exit_code, state, commands, directory = self.run_check(
            "failure",
            [
                self.suite("uv", suffix="raise SystemExit(7)"),
                self.suite("workspace_discovery"),
            ],
        )
        self.assertEqual(exit_code, 7)
        self.assertEqual(state["command_exit_code"], 7)
        self.assertEqual(state["status"], "failed")
        self.assertEqual(len(state["final_inventory"]), 1)
        self.assertEqual(len(list((directory / "aggregates").glob("*.json"))), 1)
        self.assertEqual(commands["suites"][1]["status"], "pending")
        observed, _ = self.final_observation(state, directory)
        self.assertEqual(observed["inventory"], state["final_inventory"])

    def test_rewritten_prior_result_fails_without_rewriting_retained_bytes(self):
        rewrite = """
previous = next(directory.glob('*.json'))
value = json.loads(previous.read_text())
value['benchmarks'][0]['uri'] = 'changed.rs::hash_sha256'
previous.write_text(json.dumps(value) + '\\n')
"""
        exit_code, state, commands, directory = self.run_check(
            "rewritten",
            [self.suite("uv"), self.suite("workspace_discovery", prefix=rewrite)],
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(state["command_exit_code"], 0)
        self.assertEqual(commands["status"], "success")
        self.assertEqual(
            state["evidence_error"]["message"],
            "A completed walltime aggregate was removed or changed",
        )
        first = state["observations"][0]
        self.assertEqual(
            aggregation.shards.digest(directory / "aggregates" / first["added"]),
            first["inventory"][first["added"]]["sha256"],
        )
        observed, aggregates = self.final_observation(state, directory)
        self.assertEqual(observed["inventory"], state["final_inventory"])
        self.assertEqual(len(observed["inventory"]), 2)
        self.assertNotEqual(
            aggregation.shards.digest(aggregates / first["added"]),
            first["inventory"][first["added"]]["sha256"],
        )
        self.assertEqual(observed["unexpected_results"], [])

    def test_removed_prior_result_fails(self):
        exit_code, state, _, directory = self.run_check(
            "removed",
            [
                self.suite("uv"),
                self.suite(
                    "workspace_discovery",
                    prefix="next(directory.glob('*.json')).unlink()",
                ),
            ],
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(state["command_exit_code"], 0)
        self.assertEqual(len(state["final_inventory"]), 1)
        self.assertTrue(
            (directory / "aggregates" / state["observations"][0]["added"]).is_file()
        )
        observed, aggregates = self.final_observation(state, directory)
        self.assertEqual(observed["inventory"], state["final_inventory"])
        self.assertEqual(len(observed["inventory"]), 1)
        self.assertFalse((aggregates / state["observations"][0]["added"]).exists())
        self.assertEqual(observed["unexpected_results"], [])

    def test_unrelated_final_result_is_not_retained(self):
        unrelated = """
value['benchmarks'][0]['name'] = 'unrelated_benchmark'
(directory / f'{pid}.json').write_text(json.dumps(value) + '\\n')
"""
        exit_code, state, commands, directory = self.run_check(
            "unrelated",
            [self.suite("uv"), self.suite("workspace_discovery", suffix=unrelated)],
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(commands["status"], "success")
        self.assertEqual(
            state["evidence_error"]["message"],
            "Unexpected benchmarks in final walltime aggregates",
        )
        observed, aggregates = self.final_observation(state, directory)
        self.assertEqual(len(observed["inventory"]), 1)
        self.assertEqual(len(observed["unexpected_results"]), 1)
        self.assertFalse((aggregates / observed["unexpected_results"][0]).exists())
        self.assertEqual(observed["inventory"], state["observations"][0]["inventory"])

    def test_malformed_final_result_fails_closed(self):
        malformed = "(directory / f'{pid}.json').write_text('{malformed}')"
        exit_code, state, commands, directory = self.run_check(
            "malformed",
            [self.suite("uv"), self.suite("workspace_discovery", suffix=malformed)],
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(commands["status"], "success")
        self.assertEqual(state["evidence_error"]["phase"], "aggregation")
        self.assertNotIn("final_observation", state)
        self.assertFalse((directory / "final-observation").exists())
        self.assertTrue(
            (directory / "aggregates" / state["observations"][0]["added"]).is_file()
        )

    def test_missing_result_is_not_a_successful_probe(self):
        second = {
            "name": "workspace_discovery",
            "command": [sys.executable, "-I", "-B", "-c", "pass"],
        }
        exit_code, state, commands, _ = self.run_check(
            "missing", [self.suite("uv"), second]
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(state["command_exit_code"], 0)
        self.assertEqual(commands["status"], "success")
        self.assertEqual(
            state["evidence_error"]["message"],
            "Expected one new walltime aggregate for the selected suite",
        )

    def test_deadline_keeps_timeout_and_pending_suite(self):
        first = self.suite("uv", prefix="time.sleep(30)")
        exit_code, state, commands, _ = self.run_check(
            "timeout", [first, self.suite("workspace_discovery")], timeout_seconds=0.1
        )
        self.assertEqual(exit_code, 124)
        self.assertEqual(state["command_exit_code"], 124)
        self.assertEqual(commands["status"], "timed_out")
        self.assertEqual(commands["suites"][1]["status"], "pending")

    def test_preexisting_results_are_not_counted_as_new(self):
        results = self.profile / "results"
        results.mkdir()
        (results / "101.json").write_bytes(contents(101, "hash_sha256"))
        exit_code, state, commands, _ = self.run_check(
            "preexisting", [self.suite("uv"), self.suite("workspace_discovery")]
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(len(state["initial_inventory"]), 1)
        self.assertNotIn("pid", commands["suites"][0])
        self.assertEqual(commands["suites"][1]["status"], "pending")

    def test_result_symlinks_are_rejected(self):
        results = self.profile / "results"
        results.mkdir()
        target = self.root / "outside.json"
        target.write_bytes(contents(101, "hash_sha256"))
        (results / "101.json").symlink_to(target)
        with self.assertRaisesRegex(ValueError, "file type"):
            aggregation.read_results(self.profile)


if __name__ == "__main__":
    unittest.main()
