"""Keep every built benchmark in exactly one non-empty walltime shard."""

import copy
import errno
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "benchmark/walltime-shards.py"
SPEC = importlib.util.spec_from_file_location("walltime_shards", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
shards = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shards)


class CargoCodspeedVersion(unittest.TestCase):
    def probe(self, returncode, stdout, stderr):
        root = Path.cwd()
        environment = {"PATH": "fixture-tools"}
        command = ["/fixture/cargo", "codspeed", "--version"]
        result = subprocess.CompletedProcess(command, returncode, stdout, stderr)
        with mock.patch.object(shards.subprocess, "run", return_value=result) as run:
            try:
                return shards.cargo_codspeed_version(
                    root, cargo=command[0], environment=environment
                )
            finally:
                run.assert_called_once_with(
                    command,
                    cwd=root,
                    env=environment,
                    capture_output=True,
                    text=True,
                    timeout=15,
                    check=False,
                )

    def test_successful_version_response(self):
        for version in ("5.0.1", "5.0.2", "6.0.0-alpha.1+build.2"):
            with self.subTest(version=version):
                self.assertEqual(
                    self.probe(0, f"cargo-codspeed {version}\n", ""),
                    f"cargo-codspeed {version}",
                )

    def test_known_display_version_exit(self):
        self.assertEqual(
            self.probe(1, "", "cargo-codspeed 5.0.1\n\n"),
            "cargo-codspeed 5.0.1",
        )

    def test_ambiguous_or_malformed_success_is_rejected(self):
        for stdout, stderr in (
            ("", ""),
            ("5.0.1\n", ""),
            ("cargo-codspeed 5.0.1\nextra output\n", ""),
            ("cargo-codspeed 5.0.1\n", "unexpected warning\n"),
        ):
            error = self.assertRaisesRegex(ValueError, "version response")
            with self.subTest(stdout=stdout, stderr=stderr), error:
                self.probe(0, stdout, stderr)

    def test_other_failures_remain_errors(self):
        for returncode, stdout, stderr in (
            (1, "cargo-codspeed 5.0.1\n", "cargo-codspeed 5.0.1\n\n"),
            (1, " ", "cargo-codspeed 5.0.1\n\n"),
            (1, "", "cargo-codspeed 5.0.2\n\n"),
            (1, "", " cargo-codspeed 5.0.1\n\n"),
            (1, "", "cargo-codspeed 5.0.1\nfailed to start\n"),
            (2, "", "cargo-codspeed 5.0.1\n\n"),
            (1, "", "failed to start\n"),
        ):
            error = self.assertRaises(subprocess.CalledProcessError)
            with self.subTest(returncode=returncode, stdout=stdout, stderr=stderr):
                with error:
                    self.probe(returncode, stdout, stderr)
                self.assertEqual(error.exception.returncode, returncode)
                self.assertEqual(error.exception.stdout, stdout)
                self.assertEqual(error.exception.stderr, stderr)

    def test_other_metadata_commands_remain_strict(self):
        root = Path.cwd()
        for command in (("cargo", "--version"), ("rustc", "-Vv")):
            failure = subprocess.CalledProcessError(1, command)
            patched = mock.patch.object(
                shards.subprocess, "check_output", side_effect=failure
            )
            error = self.assertRaises(subprocess.CalledProcessError)
            with self.subTest(command=command), patched, error:
                shards.output(root, *command)


def commit_source(source, message):
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
            message,
        ],
        cwd=source,
        check=True,
    )


def committed_source(source):
    source.mkdir()
    (source / "Cargo.lock").write_text("version = 4\n")
    (source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "stable"\n')
    subprocess.run(["git", "init", "--quiet", str(source)], check=True)
    subprocess.run(
        ["git", "add", "Cargo.lock", "rust-toolchain.toml"], cwd=source, check=True
    )
    commit_source(source, "Fixture")
    return source


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

    def test_launch_recheck_keeps_the_inventory_but_hashes_one_suite(self):
        self.artifacts["suite_8"].write_bytes(b"different binary")
        shards.verify_plan(self.plan, self.source, self.artifacts, 1, bench="suite_0")
        with self.assertRaisesRegex(ValueError, "artifact does not match.*suite_8"):
            shards.verify_plan(
                self.plan, self.source, self.artifacts, 1, bench="suite_8"
            )
        with self.assertRaisesRegex(ValueError, "does not belong"):
            shards.verify_plan(
                self.plan, self.source, self.artifacts, 1, bench="suite_1"
            )

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
        source = committed_source(self.root / "source")
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

    def test_suite_commands_keep_the_selected_order_and_artifact_identity(self):
        selected = shards.verify_plan(self.plan, self.source, self.artifacts, 1)
        cargo = str(Path(sys.executable).resolve())
        commands = shards.suite_commands(self.plan, selected, cargo=cargo)
        self.assertEqual([item["name"] for item in commands], selected["benches"])
        for item in commands:
            self.assertEqual(
                item["command"], shards.suite_command(item["name"], cargo=cargo)
            )
            self.assertEqual(item["command"][0], cargo)
            self.assertEqual(item["command"].count("--bench"), 1)
            self.assertEqual(item["artifact"], self.plan["artifacts"][item["name"]])


class WalltimeRuns(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.metadata = {"source": {"commit": "a" * 40}, "plan_sha256": "b" * 64}

    @staticmethod
    def suite(name, program):
        return {"name": name, "command": [sys.executable, "-c", program]}

    def run_case(self, name, suites, **limits):
        directory = self.root / name
        exit_code = shards.run_suites(
            self.root, directory, self.metadata, suites, **limits
        )
        return exit_code, json.loads((directory / "result.json").read_text())

    def test_real_commands_complete_in_order(self):
        commands = [
            self.suite(
                "first", "from pathlib import Path; Path('order').write_text('1')"
            ),
            self.suite(
                "second",
                "from pathlib import Path; p = Path('order'); p.write_text(p.read_text() + '2')",
            ),
        ]
        exit_code, state = self.run_case("success", commands)
        self.assertEqual(exit_code, 0)
        self.assertEqual((self.root / "order").read_text(), "12")
        self.assertEqual(state["source"], self.metadata["source"])
        self.assertEqual(state["status"], "success")
        self.assertEqual(state["successful_suites"], 2)
        self.assertEqual(
            [item["command"] for item in state["suites"]],
            [item["command"] for item in commands],
        )
        for item in state["suites"]:
            self.assertEqual(item["status"], "success")
            self.assertEqual(item["returncode"], 0)
            self.assertGreater(item["pid"], 0)
            self.assertIsNotNone(item["started_at"])
            self.assertIsNotNone(item["finished_at"])
        original = (self.root / "success/result.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.run_case("success", commands)
        self.assertEqual((self.root / "success/result.json").read_bytes(), original)

    def test_real_failure_stops_before_the_next_command(self):
        commands = [
            self.suite("failure", "raise SystemExit(7)"),
            self.suite(
                "pending", "from pathlib import Path; Path('unexpected').touch()"
            ),
        ]
        exit_code, state = self.run_case("failure", commands)
        self.assertEqual(exit_code, 7)
        self.assertEqual(state["status"], "failed")
        self.assertEqual(state["successful_suites"], 0)
        self.assertEqual(state["suites"][0]["returncode"], 7)
        self.assertEqual(state["suites"][1]["status"], "pending")
        self.assertNotIn("pid", state["suites"][1])
        self.assertFalse((self.root / "unexpected").exists())

    def test_failed_process_start_is_not_a_completed_command(self):
        exit_code, state = self.run_case(
            "missing",
            [{"name": "missing", "command": [str(self.root / "missing-executable")]}],
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(state["status"], "failed")
        self.assertEqual(state["suites"][0]["error"]["kind"], "FileNotFoundError")
        self.assertNotIn("pid", state["suites"][0])
        self.assertNotIn("started_at", state["suites"][0])

    def test_launch_verification_rejects_changed_real_inputs(self):
        source = committed_source(self.root / "source")
        first_commit = shards.output(source, "git", "rev-parse", "HEAD")
        (source / "Cargo.lock").write_text("version = 3\n")
        subprocess.run(["git", "add", "Cargo.lock"], cwd=source, check=True)
        commit_source(source, "Second fixture")
        second_commit = shards.output(source, "git", "rev-parse", "HEAD")
        subprocess.run(
            ["git", "checkout", "--quiet", "--detach", first_commit],
            cwd=source,
            check=True,
        )
        artifact_directory = source / "artifacts"
        artifact_directory.mkdir()
        artifact = artifact_directory / "only"
        artifact.write_bytes(b"prepared benchmark")
        tools = source / "tools"
        tools.mkdir()
        suffix = ".exe" if os.name == "nt" else ""
        consumer = {"codspeed": None}
        for name, key in (("cargo", "cargo"), ("cargo-codspeed", "cargo_codspeed")):
            path = tools / f"{name}{suffix}"
            shutil.copyfile(Path(sys.executable).resolve(), path)
            path.chmod(0o755)
            consumer[key] = {
                "invocation_executable": str(path),
                "resolved_executable": str(path.resolve()),
                "sha256": shards.digest(path),
            }
        environment = dict(os.environ)
        environment["PATH"] = str(tools) + os.pathsep + environment.get("PATH", "")
        identity = shards.source_identity(source, environment=environment)
        plan = shards.prepare_plan(
            identity, {}, shards.artifact_paths(artifact_directory)
        )
        selected = shards.verify_plan(
            plan, identity, shards.artifact_paths(artifact_directory), 1
        )

        def verify(suite):
            shards.verify_suite_launch(
                source,
                plan,
                selected,
                consumer,
                environment,
                artifact_directory,
                suite,
            )

        mutations = {
            "unchanged": lambda: None,
            "source": lambda: (source / "Cargo.lock").write_text("version = 3\n"),
            "artifact": lambda: artifact.write_bytes(b"replaced benchmark"),
            "inventory": lambda: (artifact_directory / "unexpected").write_bytes(
                b"extra"
            ),
            "tool": lambda: (tools / f"cargo-codspeed{suffix}").write_bytes(
                b"replaced tool"
            ),
            "checkout": lambda: subprocess.run(
                ["git", "checkout", "--quiet", "--detach", second_commit],
                cwd=source,
                check=True,
            ),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                (source / "Cargo.lock").write_text("version = 4\n")
                artifact.write_bytes(b"prepared benchmark")
                (artifact_directory / "unexpected").unlink(missing_ok=True)
                shutil.copyfile(
                    Path(sys.executable).resolve(), tools / f"cargo-codspeed{suffix}"
                )
                mutate()
                marker = f"launched-{name}"
                directory = self.root / f"verified-{name}"
                exit_code = shards.run_suites(
                    source,
                    directory,
                    {"source": identity},
                    [
                        self.suite(
                            "only",
                            f"from pathlib import Path; Path({marker!r}).touch()",
                        )
                    ],
                    environment=environment,
                    before_launch=verify,
                )
                state = json.loads((directory / "result.json").read_text())
                if name == "unchanged":
                    self.assertEqual(exit_code, 0)
                    self.assertTrue((source / marker).exists())
                else:
                    self.assertEqual(exit_code, 1)
                    self.assertEqual(state["status"], "failed")
                    self.assertEqual(
                        state["suites"][0]["error"]["phase"], "verification"
                    )
                    self.assertNotIn("pid", state["suites"][0])
                    self.assertFalse((source / marker).exists())

    def test_child_environment_is_frozen_before_launch(self):
        environment = dict(os.environ)
        environment["UV_WALLTIME_FIXTURE"] = "observed"
        environment["PATH"] = str(self.root / "observed-path")

        def replace_environment(_suite):
            environment["UV_WALLTIME_FIXTURE"] = "replaced"
            environment["PATH"] = str(self.root / "replaced-path")

        exit_code, state = self.run_case(
            "frozen-environment",
            [
                self.suite(
                    "first",
                    "import json, os; from pathlib import Path; "
                    "Path('environment.json').write_text(json.dumps([os.environ['UV_WALLTIME_FIXTURE'], os.environ['PATH']]))",
                )
            ],
            environment=environment,
            before_launch=replace_environment,
        )
        self.assertEqual(exit_code, 0)
        self.assertEqual(state["suites"][0]["command"][0], sys.executable)
        self.assertEqual(
            json.loads((self.root / "environment.json").read_text()),
            ["observed", str(self.root / "observed-path")],
        )

    def test_deadline_during_verification_starts_no_command(self):
        exit_code, state = self.run_case(
            "verification-deadline",
            [self.suite("pending", "raise SystemExit(99)")],
            timeout_seconds=0.01,
            before_launch=lambda _suite: time.sleep(0.02),
        )
        self.assertEqual(exit_code, 124)
        self.assertEqual(state["status"], "timed_out")
        self.assertEqual(state["suites"][0]["status"], "pending")
        self.assertNotIn("pid", state["suites"][0])

    def test_real_timeout_retains_the_started_command(self):
        commands = [
            self.suite("slow", "import time; time.sleep(30)"),
            self.suite("pending", "raise SystemExit(99)"),
        ]
        exit_code, state = self.run_case(
            "timeout", commands, timeout_seconds=0.25, grace_seconds=0.1
        )
        self.assertEqual(exit_code, 124)
        self.assertEqual(state["status"], "timed_out")
        self.assertEqual(state["suites"][0]["status"], "timed_out")
        self.assertTrue(state["suites"][0]["termination_complete"])
        self.assertGreater(state["suites"][0]["pid"], 0)
        self.assertEqual(state["suites"][1]["status"], "pending")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_timeout_stops_a_child_that_ignores_sigterm(self):
        descendant = """
import signal
import time
from pathlib import Path
signal.signal(signal.SIGTERM, signal.SIG_IGN)
for index in range(1000):
    Path("descendant-count").write_text(str(index))
    time.sleep(0.01)
"""
        program = f"""
import signal
import subprocess
import sys
import time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
subprocess.Popen([sys.executable, "-c", {descendant!r}])
time.sleep(30)
"""
        exit_code, state = self.run_case(
            "descendant",
            [self.suite("slow", program)],
            timeout_seconds=1,
            grace_seconds=0.1,
        )
        try:
            self.assertEqual(exit_code, 124)
            self.assertEqual(state["suites"][0]["returncode"], -signal.SIGKILL)
            self.assertTrue(state["suites"][0]["termination_complete"])
            time.sleep(0.05)
            stopped = (self.root / "descendant-count").read_text()
            time.sleep(0.15)
            self.assertEqual((self.root / "descendant-count").read_text(), stopped)
        finally:
            try:
                os.killpg(state["suites"][0]["pid"], signal.SIGKILL)
            except ProcessLookupError:
                pass

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_timeout_stops_the_owned_group_after_its_leader_exits(self):
        descendant = """
import signal
import time
from pathlib import Path
signal.signal(signal.SIGTERM, signal.SIG_IGN)
for index in range(1000):
    Path("owned-count").write_text(str(index))
    time.sleep(0.01)
"""
        program = f"""
import os
import subprocess
import sys
import time
from pathlib import Path
Path("owned-group").write_text(str(os.getpgrp()))
subprocess.Popen([sys.executable, "-c", {descendant!r}])
time.sleep(30)
"""
        unrelated = subprocess.Popen(
            [
                sys.executable,
                "-c",
                (
                    "import time; from pathlib import Path; "
                    "[(Path('unrelated-count').write_text(str(index)), time.sleep(0.01)) "
                    "for index in range(1000)]"
                ),
            ],
            cwd=self.root,
            start_new_session=True,
        )
        owned_group = None
        try:
            exit_code, state = self.run_case(
                "leader-exits",
                [self.suite("slow", program)],
                timeout_seconds=1,
                grace_seconds=0.25,
            )
            owned_group = state["suites"][0]["pid"]
            self.assertEqual(exit_code, 124)
            self.assertEqual(state["suites"][0]["returncode"], -signal.SIGTERM)
            self.assertTrue(state["suites"][0]["termination_complete"])
            self.assertEqual(int((self.root / "owned-group").read_text()), owned_group)
            self.assertNotEqual(owned_group, os.getpgrp())
            self.assertNotEqual(owned_group, os.getpgid(unrelated.pid))
            with self.assertRaises(ProcessLookupError):
                os.killpg(owned_group, 0)
            self.assertIsNone(unrelated.poll())
            before = (self.root / "unrelated-count").read_text()
            time.sleep(0.1)
            self.assertNotEqual((self.root / "unrelated-count").read_text(), before)
        finally:
            if owned_group is not None:
                try:
                    os.killpg(owned_group, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            try:
                os.killpg(unrelated.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            unrelated.wait(timeout=5)

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_cleanup_rejects_a_child_in_the_drivers_process_group(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"]
        )
        try:
            self.assertEqual(os.getpgid(process.pid), os.getpgrp())
            with self.assertRaisesRegex(ValueError, "lead its process group"):
                shards.stop_process(process, 0.1)
            self.assertIsNone(process.poll())
        finally:
            process.kill()
            process.wait(timeout=5)

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_group_absence_requires_esrch_after_a_denied_probe(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "raise SystemExit(0)"], start_new_session=True
        )
        self.assertEqual(process.wait(timeout=5), 0)
        events = []
        with mock.patch.object(
            shards.os,
            "killpg",
            side_effect=(
                PermissionError(errno.EPERM, "probe denied"),
                ProcessLookupError(errno.ESRCH, "group absent"),
            ),
        ) as probe:
            result = shards.wait_for_termination(
                process, 0.25, process_group=process.pid, events=events
            )
        self.assertEqual(result, (0, True))
        self.assertEqual(
            probe.call_args_list,
            [mock.call(process.pid, 0), mock.call(process.pid, 0)],
        )
        self.assertEqual(
            [(item["event"]["result"], item["event"]["errno"]) for item in events],
            [("denied", errno.EPERM), ("absent", errno.ESRCH)],
        )

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_persistent_group_probe_denial_is_not_absence(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "raise SystemExit(0)"], start_new_session=True
        )
        self.assertEqual(process.wait(timeout=5), 0)
        events = []
        with mock.patch.object(
            shards.os,
            "killpg",
            side_effect=PermissionError(errno.EPERM, "probe denied"),
        ) as probe:
            result = shards.wait_for_termination(
                process, 0.05, process_group=process.pid, events=events
            )
        self.assertEqual(result, (0, False))
        self.assertGreaterEqual(probe.call_count, 1)
        self.assertEqual(
            probe.call_args_list, [mock.call(process.pid, 0)] * probe.call_count
        )
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["count"], probe.call_count)
        self.assertEqual(events[0]["event"]["result"], "denied")
        self.assertEqual(events[0]["event"]["errno"], errno.EPERM)

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_denied_termination_signals_remain_incomplete(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
            start_new_session=True,
        )
        killpg = os.killpg
        events = []
        attempted_signals = []

        def deny_termination(process_group, signum):
            self.assertEqual(process_group, process.pid)
            if signum:
                attempted_signals.append(signum)
                raise PermissionError(errno.EPERM, "termination denied")
            return killpg(process_group, signum)

        try:
            with mock.patch.object(shards.os, "killpg", deny_termination):
                result = shards.stop_process(
                    process, 0.025, process_group=process.pid, events=events
                )
            self.assertEqual(result, (None, False))
            self.assertIsNone(process.poll())
            self.assertEqual(attempted_signals, [signal.SIGTERM, signal.SIGKILL])
            self.assertEqual(
                [
                    (item["event"]["signal"], item["event"]["errno"])
                    for item in events
                    if item["event"]["result"] == "denied"
                ],
                [(signal.SIGTERM, errno.EPERM), (signal.SIGKILL, errno.EPERM)],
            )
            self.assertFalse(
                any(item["event"]["result"] == "absent" for item in events)
            )
        finally:
            try:
                killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_signal_denial_is_retained_when_the_group_later_exits(self):
        for disposition in ("timed_out", "failed"):
            with self.subTest(disposition=disposition):
                child = f"""
import time
from pathlib import Path
Path("denied-ready-{disposition}").touch()
while not Path("denied-release-{disposition}").exists():
    time.sleep(0.01)
"""
                program = (
                    child
                    if disposition == "timed_out"
                    else f"""
import subprocess
import sys
import time
from pathlib import Path
subprocess.Popen([sys.executable, "-c", {child!r}])
while not Path("denied-ready-{disposition}").exists():
    time.sleep(0.01)
raise SystemExit(7)
"""
                )
                name = f"denied-term-{disposition}"
                directory = self.root / name
                killpg = os.killpg
                owned_group = None

                def deny_term(
                    process_group,
                    signum,
                    *,
                    directory=directory,
                    disposition=disposition,
                    killpg=killpg,
                ):
                    nonlocal owned_group
                    running = json.loads((directory / "result.json").read_text())
                    owned_group = running["suites"][0]["process_group"]
                    self.assertEqual(process_group, owned_group)
                    if signum == signal.SIGTERM:
                        (self.root / f"denied-release-{disposition}").touch()
                        raise PermissionError(errno.EPERM, "termination denied")
                    return killpg(process_group, signum)

                try:
                    with mock.patch.object(shards.os, "killpg", deny_term):
                        exit_code, state = self.run_case(
                            name,
                            [self.suite("only", program)],
                            timeout_seconds=1 if disposition == "timed_out" else None,
                            grace_seconds=0.25,
                        )
                    self.assertEqual(
                        exit_code, 124 if disposition == "timed_out" else 7
                    )
                    self.assertEqual(state["status"], disposition)
                    self.assertEqual(state["suites"][0]["status"], disposition)
                    self.assertEqual(
                        state["suites"][0]["returncode"],
                        0 if disposition == "timed_out" else 7,
                    )
                    self.assertTrue(state["suites"][0]["termination_complete"])
                    events = state["suites"][0]["cleanup_events"]
                    self.assertEqual(
                        [
                            item["event"]["signal"]
                            for item in events
                            if item["event"]["result"] == "denied"
                        ],
                        [signal.SIGTERM],
                    )
                    self.assertEqual(events[-1]["event"]["signal"], 0)
                    self.assertEqual(events[-1]["event"]["result"], "absent")
                    self.assertEqual(events[-1]["event"]["errno"], errno.ESRCH)
                finally:
                    if owned_group is not None:
                        try:
                            killpg(owned_group, signal.SIGKILL)
                        except ProcessLookupError:
                            pass

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_completed_leaders_do_not_leave_owned_descendants(self):
        for returncode in (0, 7):
            with self.subTest(returncode=returncode):
                descendant = f"""
import signal
import time
from pathlib import Path
signal.signal(signal.SIGTERM, signal.SIG_IGN)
Path("ready-{returncode}").touch()
time.sleep(30)
"""
                program = f"""
import subprocess
import sys
import time
from pathlib import Path
subprocess.Popen([sys.executable, "-c", {descendant!r}])
for _ in range(500):
    if Path("ready-{returncode}").exists():
        break
    time.sleep(0.01)
else:
    raise RuntimeError("descendant did not start")
raise SystemExit({returncode})
"""
                commands = [
                    self.suite("leader", program),
                    self.suite("next", "raise SystemExit(0)"),
                ]
                exit_code, state = self.run_case(
                    f"completed-{returncode}", commands, grace_seconds=0.25
                )
                owned_group = state["suites"][0]["pid"]
                try:
                    self.assertEqual(exit_code, returncode, json.dumps(state))
                    self.assertEqual(state["suites"][0]["returncode"], returncode)
                    self.assertEqual(
                        state["suites"][0]["status"],
                        "success" if returncode == 0 else "failed",
                    )
                    with self.assertRaises(ProcessLookupError):
                        os.killpg(owned_group, 0)
                    self.assertTrue(
                        state["suites"][0]["termination_complete"], json.dumps(state)
                    )
                    self.assertEqual(
                        state["suites"][1]["status"],
                        "success" if returncode == 0 else "pending",
                    )
                finally:
                    try:
                        os.killpg(owned_group, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_expired_deadline_starts_no_command(self):
        exit_code, state = self.run_case(
            "expired",
            [self.suite("pending", "raise SystemExit(99)")],
            deadline=time.time() - 1,
        )
        self.assertEqual(exit_code, 124)
        self.assertEqual(state["status"], "timed_out")
        self.assertEqual(state["timeout_before_suite"], "pending")
        self.assertEqual(state["suites"][0]["status"], "pending")
        self.assertNotIn("pid", state["suites"][0])

    def test_consumer_tool_identity_uses_a_real_executable(self):
        identity = shards.tool_identity(self.root, sys.executable, "--version")
        self.assertIsNotNone(identity)
        self.assertEqual(identity["invocation_executable"], str(Path(sys.executable)))
        self.assertEqual(
            identity["resolved_executable"], str(Path(sys.executable).resolve())
        )
        self.assertEqual(identity["sha256"], shards.digest(Path(sys.executable)))
        self.assertTrue(identity["version"].startswith("Python "))
        self.assertIsNone(
            shards.tool_identity(
                self.root,
                "uv-missing-walltime-tool",
                "--version",
            )
        )

    def test_consumer_uses_the_cargo_codspeed_version_probe(self):
        executable = Path(sys.executable).resolve()
        environment = {"PATH": os.environ.get("PATH", os.defpath)}

        def invocation(name, _environment):
            return None if name == "codspeed" else executable

        resolve = mock.patch.object(
            shards, "invocation_executable", side_effect=invocation
        )
        probe = mock.patch.object(
            shards, "cargo_codspeed_version", return_value="cargo-codspeed 5.0.1"
        )
        with resolve, probe as version_probe:
            consumer = shards.consumer_metadata(self.root, environment)
        version_probe.assert_called_once_with(
            self.root, cargo=str(executable), environment=environment
        )
        self.assertEqual(consumer["cargo_codspeed"]["version"], "cargo-codspeed 5.0.1")
        self.assertEqual(
            consumer["cargo_codspeed"]["version_command"],
            [str(executable), "codspeed", "--version"],
        )
        self.assertEqual(
            consumer["cargo_codspeed"]["sha256"], shards.digest(executable)
        )

    def test_consumer_rejects_cargo_lookup_changes_before_the_version_probe(self):
        first = self.root / "first-cargo"
        second = self.root / "second-cargo"
        for executable in (first, second):
            shutil.copyfile(Path(sys.executable).resolve(), executable)
            executable.chmod(0o755)
        lookups = 0

        def which(name, *, path=None):
            nonlocal lookups
            if name == "cargo":
                lookups += 1
                return str(first if lookups == 1 else second)
            if name == "cargo-codspeed":
                return str(second)
            if name == "codspeed":
                return None
            raise AssertionError(f"Unexpected lookup: {name}")

        check_output = subprocess.check_output

        def version_output(command, *arguments, **kwargs):
            if tuple(command) == (str(first), "--version"):
                return "cargo fixture\n"
            return check_output(command, *arguments, **kwargs)

        lookup = mock.patch.object(shards.shutil, "which", side_effect=which)
        output = mock.patch.object(
            shards.subprocess, "check_output", side_effect=version_output
        )
        probe = mock.patch.object(
            shards, "cargo_codspeed_version", return_value="cargo-codspeed 5.0.1"
        )
        error = self.assertRaisesRegex(ValueError, "identity was recorded")
        with lookup, output as version_command, probe as version_probe, error:
            shards.consumer_metadata(self.root, {})
        version_command.assert_not_called()
        version_probe.assert_not_called()

    def test_cargo_codspeed_probe_is_bound_to_the_observed_cargo(self):
        first = self.root / "first-cargo"
        second = self.root / "second-cargo"
        codspeed = self.root / "cargo-codspeed"
        for executable in (first, second, codspeed):
            shutil.copyfile(Path(sys.executable).resolve(), executable)
            executable.chmod(0o755)
        check_output = subprocess.check_output

        def version_output(command, *arguments, **kwargs):
            if tuple(command) == (str(first), "--version"):
                return "cargo fixture\n"
            return check_output(command, *arguments, **kwargs)

        for phase in ("before", "after"):
            with self.subTest(phase=phase):
                changed = False

                def which(name, *, path=None, phase=phase):
                    nonlocal changed
                    if name == "cargo":
                        return str(second if changed else first)
                    if name == "cargo-codspeed":
                        if phase == "before":
                            changed = True
                        return str(codspeed)
                    if name == "codspeed":
                        return None
                    raise AssertionError(f"Unexpected lookup: {name}")

                def probe(_root, *, cargo, environment):
                    nonlocal changed
                    self.assertEqual(cargo, str(first))
                    self.assertEqual(environment, {})
                    changed = True
                    return "cargo-codspeed 5.0.1"

                lookup = mock.patch.object(shards.shutil, "which", side_effect=which)
                output = mock.patch.object(
                    shards.subprocess, "check_output", side_effect=version_output
                )
                version = mock.patch.object(
                    shards, "cargo_codspeed_version", side_effect=probe
                )
                error = self.assertRaisesRegex(ValueError, "identity was recorded")
                with lookup, output as version_command, version as version_probe, error:
                    shards.consumer_metadata(self.root, {})
                version_command.assert_called_once_with(
                    (str(first), "--version"),
                    cwd=self.root,
                    env={},
                    text=True,
                    timeout=15,
                )
                if phase == "before":
                    version_probe.assert_not_called()
                else:
                    version_probe.assert_called_once_with(
                        self.root, cargo=str(first), environment={}
                    )

    @unittest.skipUnless(os.name == "posix", "requires POSIX executable symlinks")
    def test_real_rustup_proxy_keeps_the_cargo_invocation_name(self):
        rustup = shutil.which("rustup")
        if rustup is None:
            self.skipTest("requires Rustup")
        repository = SCRIPT.parents[2]
        available = subprocess.run(
            [rustup, "which", "cargo"],
            cwd=repository,
            capture_output=True,
            text=True,
            check=False,
        )
        if available.returncode != 0:
            self.skipTest("requires the repository's installed Rust toolchain")

        resolved = Path(rustup).resolve()
        cargo = self.root / "cargo"
        cargo.symlink_to(resolved)
        environment = dict(os.environ)
        environment["PATH"] = str(self.root) + os.pathsep + environment.get("PATH", "")
        invocation = shards.invocation_executable("cargo", environment)
        self.assertEqual(invocation, cargo)
        identity = shards.tool_identity(
            repository, "cargo", "--version", environment=environment
        )
        self.assertEqual(identity["invocation_executable"], str(cargo))
        self.assertEqual(identity["resolved_executable"], str(resolved))
        self.assertEqual(identity["sha256"], shards.digest(resolved))
        self.assertTrue(identity["version"].startswith("cargo "))

        arguments = ["metadata", "--locked", "--no-deps", "--format-version", "1"]
        metadata = json.loads(
            subprocess.check_output(
                [str(invocation), *arguments],
                cwd=repository,
                env=environment,
                text=True,
            )
        )
        self.assertIn("uv-bench", [package["name"] for package in metadata["packages"]])
        wrong_name = subprocess.run(
            [str(resolved), *arguments],
            cwd=repository,
            env=environment,
            capture_output=True,
            check=False,
        )
        self.assertNotEqual(wrong_name.returncode, 0)

    @unittest.skipUnless(os.name == "posix", "requires POSIX executable symlinks")
    def test_tool_checks_reject_changed_lookup_target_and_bytes(self):
        source = committed_source(self.root / "source")
        artifact_directory = source / "artifacts"
        artifact_directory.mkdir()
        (artifact_directory / "only").write_bytes(b"prepared benchmark")
        first_target = self.root / "first-target"
        second_target = self.root / "second-target"
        original_bytes = Path(sys.executable).resolve().read_bytes()
        for target in (first_target, second_target):
            target.write_bytes(original_bytes)
            target.chmod(0o755)
        first_directory = self.root / "first-tools"
        second_directory = self.root / "second-tools"
        first_directory.mkdir()
        second_directory.mkdir()
        first = first_directory / "cargo"
        second = second_directory / "cargo"
        first.symlink_to(first_target)
        second.symlink_to(first_target)
        environment = dict(os.environ)
        environment["PATH"] = os.pathsep.join(
            (str(first_directory), str(second_directory), environment.get("PATH", ""))
        )
        identity = shards.executable_identity("cargo", environment)
        consumer = {"cargo": identity, "cargo_codspeed": None, "codspeed": None}
        source_identity = shards.source_identity(source, environment=environment)
        plan = shards.prepare_plan(
            source_identity, {}, shards.artifact_paths(artifact_directory)
        )
        selected = shards.verify_plan(
            plan, source_identity, shards.artifact_paths(artifact_directory), 1
        )

        def reset():
            first.unlink(missing_ok=True)
            first_target.write_bytes(original_bytes)
            first.symlink_to(first_target)

        def change_target():
            first.unlink()
            first.symlink_to(second_target)

        def verify():
            shards.verify_suite_launch(
                source,
                plan,
                selected,
                consumer,
                environment,
                artifact_directory,
                {"name": "only"},
            )

        verify()
        for name, mutate in (
            ("lookup", first.unlink),
            ("target", change_target),
            ("bytes", lambda: first_target.write_bytes(b"replaced executable")),
        ):
            with self.subTest(name=name):
                reset()
                mutate()
                with self.assertRaisesRegex(ValueError, "walltime tool changed: cargo"):
                    verify()
                reset()

                def version_probe(_invocation, mutate=mutate):
                    mutate()
                    return "fixture version"

                with self.assertRaisesRegex(ValueError, "identity was recorded"):
                    shards.tool_identity(
                        source,
                        "cargo",
                        "--version",
                        environment=environment,
                        version_probe=version_probe,
                    )
                reset()

    def test_other_consumer_tool_failures_remain_errors(self):
        with self.assertRaises(subprocess.CalledProcessError):
            shards.tool_identity(self.root, sys.executable, "-c", "raise SystemExit(1)")

    def start_driver(
        self,
        directory,
        *,
        command_program="import time; time.sleep(30)",
        grace_seconds=0.1,
    ):
        program = f"""
import importlib.util
import sys
from pathlib import Path
spec = importlib.util.spec_from_file_location("walltime_shards", {str(SCRIPT)!r})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
commands = [
    {{"name": "slow", "command": [sys.executable, "-c", {command_program!r}]}},
    {{"name": "pending", "command": [sys.executable, "-c", "raise SystemExit(99)"]}},
]
sys.exit(module.run_suites(Path({str(self.root)!r}), Path({str(directory)!r}), {self.metadata!r}, commands, grace_seconds={grace_seconds!r}))
"""
        process = subprocess.Popen(
            [sys.executable, "-c", program],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        child_pid = None

        def cleanup():
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
            if child_pid is not None:
                try:
                    os.killpg(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if process.stderr is not None:
                process.stderr.close()

        self.addCleanup(cleanup)
        expires = time.monotonic() + 5
        while time.monotonic() < expires:
            if (directory / "result.json").exists():
                state = json.loads((directory / "result.json").read_text())
                if state["suites"][0]["status"] == "running":
                    child_pid = state["suites"][0]["pid"]
                    return process, state
            if process.poll() is not None:
                stderr = process.stderr.read() if process.stderr is not None else b""
                self.fail(f"Walltime fixture exited early: {stderr!r}")
            time.sleep(0.01)
        self.fail("Walltime fixture did not start its command")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_outer_kill_leaves_a_readable_partial_run(self):
        directory = self.root / "partial"
        process, started = self.start_driver(directory)
        process.kill()
        self.assertEqual(process.wait(timeout=5), -signal.SIGKILL)
        state = json.loads((directory / "result.json").read_text())
        self.assertEqual(state, started)
        self.assertEqual(state["status"], "running")
        self.assertIsNone(state["finished_at"])
        self.assertEqual(state["source"], self.metadata["source"])
        self.assertEqual(state["suites"][0]["status"], "running")
        self.assertEqual(state["suites"][1]["status"], "pending")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_sigterm_is_recorded_as_an_interruption(self):
        directory = self.root / "interrupted"
        process, _ = self.start_driver(directory)
        process.terminate()
        self.assertEqual(process.wait(timeout=5), 128 + signal.SIGTERM)
        state = json.loads((directory / "result.json").read_text())
        self.assertEqual(state["status"], "interrupted")
        self.assertEqual(state["suites"][0]["status"], "interrupted")
        self.assertEqual(state["suites"][0]["signal"], signal.SIGTERM)
        self.assertTrue(state["suites"][0]["termination_complete"])
        self.assertEqual(state["suites"][1]["status"], "pending")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_repeated_signals_do_not_interrupt_owned_group_cleanup(self):
        directory = self.root / "repeated-interruption"
        program = """
import signal
import time
from pathlib import Path
signal.signal(signal.SIGTERM, lambda *_: Path("child-terminated").touch())
Path("child-ready").touch()
time.sleep(30)
"""
        process, started = self.start_driver(
            directory,
            command_program=program,
            grace_seconds=1,
        )
        expires = time.monotonic() + 5
        while not (self.root / "child-ready").exists():
            self.assertLess(time.monotonic(), expires, "child did not become ready")
            time.sleep(0.01)
        process.terminate()
        while not (self.root / "child-terminated").exists():
            self.assertLess(time.monotonic(), expires, "cleanup did not start")
            time.sleep(0.01)
        process.send_signal(signal.SIGINT)
        process.send_signal(signal.SIGTERM)
        self.assertEqual(process.wait(timeout=5), 128 + signal.SIGTERM)
        state = json.loads((directory / "result.json").read_text())
        self.assertEqual(state["status"], "interrupted")
        self.assertEqual(state["suites"][0]["signal"], signal.SIGTERM)
        self.assertEqual(state["suites"][0]["returncode"], -signal.SIGKILL)
        self.assertTrue(state["suites"][0]["termination_complete"])
        self.assertEqual(state["suites"][1]["status"], "pending")
        with self.assertRaises(ProcessLookupError):
            os.killpg(started["suites"][0]["pid"], 0)

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_cancellation_during_completed_leader_cleanup_keeps_its_result(self):
        for returncode in (0, 7):
            with self.subTest(returncode=returncode):
                directory = self.root / f"cleanup-interruption-{returncode}"
                descendant = f"""
import signal
import time
from pathlib import Path
signal.signal(signal.SIGTERM, lambda *_: Path("cleanup-started-{returncode}").touch())
Path("cleanup-ready-{returncode}").touch()
time.sleep(30)
"""
                program = f"""
import subprocess
import sys
import time
from pathlib import Path
subprocess.Popen([sys.executable, "-c", {descendant!r}])
while not Path("cleanup-ready-{returncode}").exists():
    time.sleep(0.01)
raise SystemExit({returncode})
"""
                process, started = self.start_driver(
                    directory, command_program=program, grace_seconds=1
                )
                expires = time.monotonic() + 5
                while not (self.root / f"cleanup-started-{returncode}").exists():
                    self.assertLess(time.monotonic(), expires, "cleanup did not start")
                    time.sleep(0.01)
                process.send_signal(signal.SIGTERM)
                expected_exit = returncode if returncode else 128 + signal.SIGTERM
                actual_exit = process.wait(timeout=5)
                state = json.loads((directory / "result.json").read_text())
                self.assertEqual(actual_exit, expected_exit, json.dumps(state))
                self.assertEqual(state["received_signal"], signal.SIGTERM)
                self.assertEqual(state["suites"][0]["returncode"], returncode)
                self.assertEqual(
                    state["suites"][0]["status"],
                    "success" if returncode == 0 else "failed",
                )
                self.assertTrue(state["suites"][0]["termination_complete"])
                self.assertEqual(state["suites"][1]["status"], "pending")
                with self.assertRaises(ProcessLookupError):
                    os.killpg(started["suites"][0]["pid"], 0)


if __name__ == "__main__":
    unittest.main()
