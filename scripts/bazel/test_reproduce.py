# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run with `uv run --offline scripts/bazel/test_reproduce.py`. No network required."""

import argparse
import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

RUNNER = Path(__file__).with_name("reproduce.py")
SPEC = importlib.util.spec_from_file_location("reproduce", RUNNER)
reproduce = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reproduce)

FAKE_KEY = "fake-buildbuddy-test-key"
FAKE_REVISION = "a" * 40
FAKE_BINARY = b"fake Linux uv binary\n"
REMOTE_ACTIONS = 5


class ReproduceTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(
            self.enterContext(tempfile.TemporaryDirectory())
        ).resolve()
        self.root = self.directory / "checkout"
        self.root.mkdir()
        (self.root / ".bazelrc").write_text(
            "build:remote-linux --config=ci\ntry-import %workspace%/user.bazelrc\n"
        )
        (self.root / ".bazelversion").write_text("9.0.0\n")
        self.enterContext(mock.patch.object(reproduce, "ROOT", self.root))
        self.args = argparse.Namespace(
            bazel="fake-bazel",
            host="remote.buildbuddy.io",
            instance="uv-dev-experiment/offline-test",
            key_file=self.directory / "unread-fake.key",
            run_dir=self.directory / "results",
            dry_run=False,
        )

    def write_bep(self, events, path=None):
        path = path or self.directory / "events.bep.json"
        path.write_text("\n".join(json.dumps(event) for event in events) + "\n")
        return path

    def events(self, runners=(), *, test=False):
        events = [
            {
                "started": {
                    "startTime": "2026-08-26T10:00:00Z",
                    "buildToolVersion": "9.0.0",
                    "command": "this raw command must not appear in summary.json",
                }
            },
            {
                "buildMetrics": {
                    "actionSummary": {
                        "runnerCount": [
                            {"name": name, "count": str(count)}
                            for name, count in runners
                        ],
                        "remoteCacheHits": 9999,
                    }
                }
            },
            {
                "finished": {
                    "finishTime": "2026-08-26T10:00:01.250Z",
                    "overallSuccess": True,
                    "exitCode": {"name": "SUCCESS"},
                }
            },
        ]
        if test:
            events.append(
                {
                    "testResult": {
                        "status": "PASSED",
                        "executionInfo": {"strategy": "remote"},
                    }
                }
            )
        return events

    def good_results(self):
        results = {
            phase: reproduce.summarize_bep(
                self.write_bep(
                    self.events(
                        [
                            (
                                "remote cache hit" if phase == "replay" else "remote",
                                REMOTE_ACTIONS,
                            )
                        ],
                        test=phase == "tests",
                    )
                )
            )
            for phase in ("seed", "replay", "tests")
        }
        results["seed"]["binary_sha256"] = results["replay"]["binary_sha256"] = "a" * 64
        results["tests"]["rust_tests_passed"] = 67
        return results

    def test_commands_isolate_rc_output_bases_and_local_caches(self):
        for phase, action in (
            ("fetch", "fetch"),
            ("seed", "build"),
            ("replay", "build"),
            ("tests", "test"),
        ):
            with self.subTest(phase=phase):
                arguments = reproduce.command(self.args, self.args.run_dir, phase)
                startup = arguments[: arguments.index(action)]
                for flag in ("--nosystem_rc", "--nohome_rc", "--noworkspace_rc"):
                    self.assertIn(flag, startup)
                self.assertEqual(
                    [
                        argument
                        for argument in startup
                        if argument.startswith("--bazelrc=")
                    ],
                    [
                        f"--bazelrc={self.args.run_dir / 'experiment.bazelrc'}",
                        "--bazelrc=/dev/null",
                    ],
                )
                output_base = self.args.run_dir / (
                    "seed" if phase == "tests" else phase
                )
                for argument in (
                    f"--output_base={output_base}",
                    f"--output_user_root={self.args.run_dir / 'bazel-root'}",
                    "--disk_cache=",
                    f"--repository_cache={self.args.run_dir / 'repository-cache'}",
                    f"--repo_contents_cache={self.args.run_dir / 'repo-contents-cache'}",
                    "--bes_backend=",
                    "--bes_results_url=",
                    f"--remote_instance_name={self.args.instance}",
                ):
                    self.assertIn(argument, arguments)
                self.assertEqual(
                    "--nocache_test_results" in arguments, phase == "tests"
                )

    def test_sanitized_rc_excludes_user_rc_and_rejects_other_imports(self):
        self.assertEqual(reproduce.sanitized_rc(), "build:remote-linux --config=ci\n")
        for import_line in (
            "import extra.bazelrc",
            "  try-import %workspace%/other.bazelrc",
        ):
            with self.subTest(import_line=import_line):
                (self.root / ".bazelrc").write_text(import_line + "\n")
                with self.assertRaisesRegex(ValueError, "Unexpected Bazel rc import"):
                    reproduce.sanitized_rc()

    def test_dry_run_creates_nothing_and_does_not_read_keys_or_start_processes(self):
        self.args.dry_run = True
        before = set(self.directory.rglob("*"))
        opened = []
        original_open = Path.open

        def tracked_open(path, *arguments, **keywords):
            opened.append(path)
            return original_open(path, *arguments, **keywords)

        with (
            mock.patch.object(Path, "open", autospec=True, side_effect=tracked_open),
            mock.patch.object(reproduce.subprocess, "run") as run,
            mock.patch.object(reproduce.subprocess, "check_output") as check_output,
            mock.patch.object(reproduce.shutil, "which") as which,
            contextlib.redirect_stdout(io.StringIO()) as output,
        ):
            reproduce.run(self.args)
        self.assertEqual(set(self.directory.rglob("*")), before)
        self.assertEqual(opened, [self.root / ".bazelrc"])
        run.assert_not_called()
        check_output.assert_not_called()
        which.assert_not_called()
        self.assertEqual(
            [line.split(":", 1)[0] for line in output.getvalue().splitlines()],
            ["fetch", "seed", "replay", "tests"],
        )

    def test_default_and_explicit_instances_stay_in_experiment_namespace(self):
        with (
            mock.patch.object(
                sys,
                "argv",
                [
                    str(RUNNER),
                    "--host",
                    self.args.host,
                    "--key-file",
                    str(self.args.key_file),
                    "--dry-run",
                ],
            ),
            mock.patch.object(reproduce, "run") as run,
        ):
            reproduce.main()
        self.assertRegex(
            run.call_args.args[0].instance, r"^uv-dev-experiment/repro-[a-f0-9]{32}$"
        )
        self.args.dry_run = True
        for instance in (
            "",
            "default",
            "uv-dev-experiment",
            "uv-dev-experiment/../production",
            "uv-dev-experiment/team?key=value",
        ):
            with self.subTest(instance=instance):
                self.args.instance = instance
                with self.assertRaisesRegex(ValueError, "fresh namespace"):
                    reproduce.run(self.args)

    def test_run_directory_cannot_reuse_checkout_parent_or_existing_state(self):
        existing = self.directory / "existing"
        existing.mkdir()
        for run_dir in (self.root, self.root / "run", self.directory, existing):
            with self.subTest(run_dir=run_dir):
                self.args.run_dir = run_dir
                self.args.dry_run = True
                with self.assertRaises(ValueError):
                    reproduce.run(self.args)

    def test_key_file_must_stay_outside_checkout(self):
        self.args.key_file = self.root / "buildbuddy.key"
        self.args.dry_run = True
        with self.assertRaisesRegex(ValueError, "credential file outside the checkout"):
            reproduce.run(self.args)

    @unittest.skipUnless(os.name == "posix", "Unix symlink")
    def test_key_file_parser_preserves_symlink_for_helper_validation(self):
        symlink = self.directory / "symlink.key"
        symlink.symlink_to(self.args.key_file)
        with (
            mock.patch.object(
                sys,
                "argv",
                [
                    str(RUNNER),
                    "--host",
                    self.args.host,
                    "--key-file",
                    str(symlink),
                    "--dry-run",
                ],
            ),
            mock.patch.object(reproduce, "run") as run,
        ):
            reproduce.main()
        self.assertEqual(run.call_args.args[0].key_file, symlink)

    def test_bep_uses_runner_counts_and_supports_both_timestamp_formats(self):
        for legacy_timestamps in (False, True):
            with self.subTest(legacy_timestamps=legacy_timestamps):
                events = self.events([("remote cache hit", REMOTE_ACTIONS)])
                if legacy_timestamps:
                    del events[0]["started"]["startTime"]
                    events[0]["started"]["startTimeMillis"] = "1000"
                    del events[2]["finished"]["finishTime"]
                    events[2]["finished"]["finishTimeMillis"] = "2250"
                self.assertEqual(
                    reproduce.summarize_bep(self.write_bep(events)),
                    {
                        "runners": {"remote-cache-hit": REMOTE_ACTIONS},
                        "tests": [],
                        "bazel_version": "9.0.0",
                        "success": True,
                        "bazel_elapsed_seconds": 1.25,
                    },
                )

    def test_incomplete_or_failed_bep_is_not_successful(self):
        for events in ([], self.events()[:-1], self.events()[1:]):
            with self.subTest(events=events):
                with self.assertRaisesRegex(ValueError, "Incomplete build events"):
                    reproduce.summarize_bep(self.write_bep(events))
        for failure in ({"exitCode": {"code": 1}}, {"overallSuccess": False}):
            with self.subTest(failure=failure):
                events = self.events()
                events[2]["finished"].update(failure)
                self.assertFalse(
                    reproduce.summarize_bep(self.write_bep(events))["success"]
                )

    def test_bep_detects_both_remote_cache_markers(self):
        for marker in ("result", "execution"):
            with self.subTest(marker=marker):
                events = self.events(test=True)
                result = events[-1]["testResult"]
                (result if marker == "result" else result["executionInfo"])[
                    "cachedRemotely"
                ] = True
                self.assertTrue(
                    reproduce.summarize_bep(self.write_bep(events))["tests"][0][
                        "cached_remotely"
                    ]
                )

    def test_binary_discovery_handles_file_and_bytestream_events(self):
        output_base = self.directory / "seed output"
        binary = output_base / "execroot/_main/bazel-out/k8-fastbuild/bin/crates/uv/uv"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(FAKE_BINARY)
        for item in (
            {"uri": binary.as_uri()},
            {
                "uri": "bytestream://remote.buildbuddy.io/blobs/fake/21",
                "name": "crates/uv/uv",
                "pathPrefix": ["bazel-out", "k8-fastbuild", "bin"],
            },
        ):
            with self.subTest(item=item):
                path = self.write_bep([{"namedSetOfFiles": {"files": [item]}}])
                self.assertEqual(reproduce.binary_from_bep(path, output_base), binary)

    def test_binary_discovery_rejects_missing_or_outside_outputs(self):
        output_base = self.directory / "seed"
        outside = self.directory / "outside/bin/crates/uv/uv"
        outside.parent.mkdir(parents=True)
        outside.write_bytes(FAKE_BINARY)
        for item in (
            {"uri": outside.as_uri()},
            {"uri": (output_base / "missing/bin/crates/uv/uv").as_uri()},
            {"uri": "https://remote.buildbuddy.io/bin/crates/uv/uv"},
            {
                "uri": "bytestream://remote.buildbuddy.io/blobs/fake/21",
                "name": "crates/uv/uv",
                "pathPrefix": [str(outside.parents[2])],
            },
        ):
            with self.subTest(item=item):
                path = self.write_bep([{"namedSetOfFiles": {"files": [item]}}])
                with self.assertRaisesRegex(ValueError, "No downloaded uv binary"):
                    reproduce.binary_from_bep(path, output_base)

    def test_verify_results_rejects_cache_permission_failures_and_digest_mismatch(self):
        results = self.good_results()
        self.assertEqual(reproduce.verify_results(results), [])
        results["replay"]["runners"] = {"remote": REMOTE_ACTIONS}
        self.assertEqual(
            reproduce.verify_results(results),
            [
                "Replay executed remote actions; check Action Cache write permission.",
                "Replay cache hits do not match the seed's remote action count.",
            ],
        )
        results = self.good_results()
        results["replay"]["binary_sha256"] = "b" * 64
        self.assertEqual(
            reproduce.verify_results(results),
            ["Seed and replay binary hashes differ or are missing."],
        )
        results = self.good_results()
        results["seed"]["runners"]["remote-cache-hit"] = 1
        self.assertEqual(
            reproduce.verify_results(results),
            ["Seed had cache hits; this is not a cold action-cache measurement."],
        )
        results = self.good_results()
        results["seed"]["runners"] = results["replay"]["runners"] = {}
        self.assertEqual(
            reproduce.verify_results(results),
            ["Seed executed no remote actions; use a new experiment namespace."],
        )

    def test_verify_results_requires_one_successful_uncached_remote_test(self):
        original = self.good_results()
        for change in (
            {"status": "FAILED"},
            {"strategy": "local"},
            {"cached_locally": True},
            {"cached_remotely": True},
            None,
        ):
            with self.subTest(change=change):
                results = copy.deepcopy(original)
                if change is None:
                    results["tests"]["tests"] = []
                else:
                    results["tests"]["tests"][0].update(change)
                self.assertEqual(
                    reproduce.verify_results(results),
                    [
                        "The PEP 440 test must pass once on a remote worker without cached results."
                    ],
                )
        results = copy.deepcopy(original)
        results["tests"]["rust_tests_passed"] = 66
        self.assertEqual(
            reproduce.verify_results(results),
            ["Expected 67 passing Rust tests in the test console log."],
        )

    def fake_processes(
        self, *, replay_miss=False, failed_seed=False, incomplete_seed=False
    ):
        commands = []

        def fake_run(arguments, **keywords):
            commands.append(arguments)
            self.assertNotIn(FAKE_KEY, " ".join(arguments))
            for variable in ("BUILDBUDDY_API_KEY", "GITHUB_TOKEN", "BAZELISK_BASE_URL"):
                self.assertNotIn(variable, keywords["env"])
            self.assertEqual(
                keywords["env"]["UV_BAZEL_KEY_FILE"], str(self.args.key_file)
            )
            self.assertEqual(
                keywords["env"]["UV_BAZEL_CREDENTIAL_HOST"], self.args.host
            )
            if arguments[0] == sys.executable:
                self.assertEqual(keywords["stdout"], subprocess.DEVNULL)
                return subprocess.CompletedProcess(arguments, 0)
            self.assertEqual(keywords["cwd"], self.root)
            if arguments[-1] == "shutdown":
                self.assertEqual(keywords["stdout"], subprocess.DEVNULL)
                return subprocess.CompletedProcess(arguments, 0)
            bep = Path(
                next(
                    argument.split("=", 1)[1]
                    for argument in arguments
                    if argument.startswith("--build_event_json_file=")
                )
            )
            output_base = Path(
                next(
                    argument.split("=", 1)[1]
                    for argument in arguments
                    if argument.startswith("--output_base=")
                )
            )
            output_base.mkdir(parents=True, exist_ok=True)
            phase = bep.name.split(".", 1)[0]
            keywords["stdout"].write(FAKE_KEY + "\n")
            if phase == "seed" and failed_seed:
                return subprocess.CompletedProcess(arguments, 1)
            runner = (
                "remote cache hit"
                if phase == "replay" and not replay_miss
                else "remote"
            )
            events = self.events([(runner, REMOTE_ACTIONS)], test=phase == "tests")
            if phase in ("seed", "replay"):
                binary = (
                    output_base
                    / "execroot/_main/bazel-out/k8-fastbuild/bin/crates/uv/uv"
                )
                binary.parent.mkdir(parents=True)
                binary.write_bytes(FAKE_BINARY)
                events.append(
                    {"namedSetOfFiles": {"files": [{"uri": binary.as_uri()}]}}
                )
            if phase == "tests":
                keywords["stdout"].write(
                    "test result: ok. 67 passed; 0 failed; 0 ignored;\n"
                )
            if phase == "seed" and incomplete_seed:
                events = [event for event in events if "finished" not in event]
            self.write_bep(events, bep)
            return subprocess.CompletedProcess(arguments, 0)

        self.enterContext(
            mock.patch.dict(
                os.environ,
                {
                    "BUILDBUDDY_API_KEY": FAKE_KEY,
                    "GITHUB_TOKEN": FAKE_KEY,
                    "BAZELISK_BASE_URL": "https://fake.invalid",
                },
                clear=True,
            )
        )
        self.enterContext(
            mock.patch.object(reproduce.shutil, "which", return_value="fake-bazel")
        )
        self.enterContext(
            mock.patch.object(
                reproduce.subprocess,
                "check_output",
                side_effect=[FAKE_REVISION + "\n", ""],
            )
        )
        self.enterContext(
            mock.patch.object(reproduce.subprocess, "run", side_effect=fake_run)
        )
        return commands

    def test_offline_end_to_end_run_writes_verified_selected_metrics(self):
        commands = self.fake_processes()
        with contextlib.redirect_stdout(io.StringIO()) as output:
            reproduce.run(self.args)
        summary_text = (self.args.run_dir / "summary.json").read_text()
        summary = json.loads(summary_text)
        self.assertTrue(summary["verified"])
        self.assertEqual(summary["source_revision"], FAKE_REVISION)
        self.assertEqual(
            summary["seed"]["binary_sha256"], hashlib.sha256(FAKE_BINARY).hexdigest()
        )
        self.assertEqual(
            summary["replay"]["runners"], {"remote-cache-hit": REMOTE_ACTIONS}
        )
        self.assertEqual(summary["tests"]["rust_tests_passed"], 67)
        self.assertEqual(
            len([arguments for arguments in commands if arguments[-1] != "shutdown"]), 5
        )
        self.assertEqual(
            [
                next(
                    argument
                    for argument in arguments
                    if argument.startswith("--output_base=")
                )
                for arguments in commands
                if arguments[-1] == "shutdown"
            ],
            [
                f"--output_base={self.args.run_dir / phase}"
                for phase in ("fetch", "seed", "replay")
            ],
        )
        self.assertNotIn(FAKE_KEY, summary_text + output.getvalue())
        self.assertNotIn("this raw command", summary_text)
        self.assertNotIn(str(self.root), summary_text)
        self.assertEqual(
            (self.args.run_dir / "experiment.bazelrc").read_text(),
            "build:remote-linux --config=ci\n",
        )
        if os.name == "posix":
            self.assertEqual(self.args.run_dir.stat().st_mode & 0o077, 0)

    def test_offline_replay_miss_writes_unverified_summary(self):
        self.fake_processes(replay_miss=True)
        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "Action Cache write permission"):
                reproduce.run(self.args)
        summary = json.loads((self.args.run_dir / "summary.json").read_text())
        self.assertFalse(summary["verified"])
        self.assertEqual(len(summary["validation_errors"]), 2)

    def test_wrong_bazel_version_aborts_before_seed(self):
        commands = self.fake_processes()
        (self.root / ".bazelversion").write_text("8.0.0\n")
        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "does not match .bazelversion"):
                reproduce.run(self.args)
        summary = json.loads((self.args.run_dir / "summary.json").read_text())
        self.assertFalse(summary["verified"])
        self.assertNotIn("seed", summary)
        self.assertFalse(any("build" in arguments for arguments in commands))

    def test_failed_or_incomplete_phase_preserves_partial_summary(self):
        for failure in ("failed_seed", "incomplete_seed"):
            with self.subTest(failure=failure):
                self.args.run_dir = self.directory / failure
                self.fake_processes(**{failure: True})
                with contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaises(ValueError):
                        reproduce.run(self.args)
                summary = json.loads((self.args.run_dir / "summary.json").read_text())
                self.assertIn("fetch", summary)
                self.assertNotIn("seed", summary)
                self.assertFalse(summary.get("verified", False))


if __name__ == "__main__":
    unittest.main()
