# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Exercise the manual cache-acceptance contract without Cargo or cache publication."""

from __future__ import annotations

import argparse
import base64
import copy
import errno
import hashlib
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid
from pathlib import Path
from typing import Any
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
ACTION = ROOT / ".github/actions/uv-rust-cache"
PROCESS_OWNER_REPOSITORY: Path | None = None
RETAIN_DIRECTORY: Path | None = None
sys.dont_write_bytecode = True


def load_script(name: str, path: Path) -> Any:
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError("Could not load a source-contract helper")
    module = importlib.util.module_from_spec(specification)
    sys.modules[name] = module
    specification.loader.exec_module(module)
    return module


acceptance = load_script("uv_ci_rust_cache_acceptance", ACTION / "acceptance.py")
cache = acceptance.cache
cache_tests = load_script(
    "uv_ci_rust_cache_tests", Path(__file__).with_name("test_ci_rust_cache.py")
)
plan_tests = load_script(
    "uv_ci_plan_tests", Path(__file__).with_name("test_ci_plan.py")
)


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def replace_json(path: Path, value: Any) -> None:
    path.write_bytes(cache.json_bytes(value))


def cargo_artifact(
    source: Path, target: Path, *, fresh: bool = True, explicit_target: bool = False
) -> dict[str, Any]:
    directory = target / cache.HOST if explicit_target else target
    executable = directory / cache.PROFILE / "deps/example-0123456789abcdef"
    return {
        "reason": "compiler-artifact",
        "package_id": "path+" + source.as_uri() + "#example@0.1.0",
        "manifest_path": str(source / "Cargo.toml"),
        "target": {
            "kind": ["test"],
            "crate_types": ["bin"],
            "name": "example",
            "src_path": str(source / "tests/example.rs"),
            "edition": "2024",
            "doc": False,
            "doctest": False,
            "test": True,
        },
        "profile": {
            "opt_level": "0",
            "debuginfo": "line-tables-only",
            "debug_assertions": True,
            "overflow_checks": True,
            "test": True,
        },
        "features": ["one", "two"],
        "filenames": [str(executable)],
        "executable": str(executable),
        "fresh": fresh,
    }


def cargo_stdout(*records: dict[str, Any]) -> bytes:
    return b"".join(
        json.dumps(record, separators=(",", ":")).encode() + b"\n" for record in records
    )


def nextest_stderr(
    *,
    tests: int = 1,
    passed: int = 1,
    skipped: int = 0,
    binaries: int = 1,
    skipped_binaries: int = 0,
    annotations: str = "",
) -> bytes:
    skips = []
    if skipped:
        skips.append(f"{skipped} {'test' if skipped == 1 else 'tests'}")
    if skipped_binaries:
        skips.append(
            f"{skipped_binaries} {'binary' if skipped_binaries == 1 else 'binaries'}"
        )
    suffix = f" ({' and '.join(skips)} skipped)" if skips else ""
    details = f" ({annotations})" if annotations else ""
    return (
        "    Finished `fast-build-nightly` profile [unoptimized + debuginfo] target(s) in 1.25s\n"
        f"    Starting {tests} {'test' if tests == 1 else 'tests'} across {binaries} {'binary' if binaries == 1 else 'binaries'}{suffix}\n"
        f"     Summary [  0.500s] {tests} {'test' if tests == 1 else 'tests'} run: {passed} passed{details}, {skipped} skipped\n"
    ).encode()


def junit_xml(number: int = 1, *, body: str = "", setup: bool = False) -> bytes:
    identifier = str(uuid.UUID(int=number, version=4))
    setup_suite = (
        '<testsuite name="@setup-script:fixture"><testcase classname="@setup-script:fixture" name="fixture"/></testsuite>'
        if setup
        else ""
    )
    return (
        f'<testsuites uuid="{identifier}" timestamp="2023-11-14T22:13:21.000Z" time="0.5">'
        + setup_suite
        + '<testsuite name="example::example"><testcase classname="example::example" name="works">'
        + body
        + "</testcase></testsuite></testsuites>"
    ).encode()


def fixture_fingerprints(*, populated: bool = True) -> dict[str, Any]:
    contents = b'{"rustc":123,"features":"[]"}\n'
    entries = (
        [
            {
                "path": cache.PROFILE + "/.fingerprint/example-0123/test-example.json",
                "size": len(contents),
                "sha256": digest_bytes(contents),
                "contents_base64": base64.b64encode(contents).decode(),
                "configuration": json.loads(contents),
            }
        ]
        if populated
        else []
    )
    return {
        "entries": entries,
        "files": len(entries),
        "bytes": sum(item["size"] for item in entries),
        "sha256": cache.digest(entries),
    }


def fixture_payload(metadata: bytes | None = None) -> dict[str, Any]:
    if metadata is None:
        return {
            "entries": None,
            "files": 1,
            "bytes": 10,
            "symlinks": 0,
            "content_tree_sha256": None,
        }
    entries = [
        {
            "path": ".rustc_info.json",
            "mode": 0o600,
            "kind": "file",
            "size": len(metadata),
            "sha256": digest_bytes(metadata),
        },
        {"path": cache.PROFILE, "mode": 0o755, "kind": "directory"},
        {"path": cache.PROFILE + "/deps", "mode": 0o755, "kind": "directory"},
        {
            "path": cache.PROFILE + "/deps/example",
            "mode": 0o755,
            "kind": "file",
            "size": 10,
            "sha256": "9" * 64,
        },
    ]
    return {
        "entries": entries,
        "files": 2,
        "bytes": len(metadata) + 10,
        "symlinks": 0,
        "content_tree_sha256": cache.digest(entries),
    }


class AcceptanceFixture(unittest.TestCase):
    def setUp(self) -> None:
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="uv-rust-cache-acceptance-test-", dir=scratch
        )
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.workspace = self.root / "workspace"
        self.controller_path = self.workspace / "controller"
        self.controller_path.mkdir(parents=True)
        self.github = {
            "GITHUB_REPOSITORY": acceptance.REPOSITORY,
            "GITHUB_SHA": "e" * 40,
            "GITHUB_RUN_ID": "123456",
            "GITHUB_RUN_ATTEMPT": "2",
            "GITHUB_JOB": "accept",
            "GITHUB_WORKFLOW_REF": acceptance.REPOSITORY
            + "/.github/workflows/linux-runner-acceptance.yml@refs/heads/fixture",
            "GITHUB_WORKFLOW_SHA": "e" * 40,
        }
        self.environment = {
            **self.github,
            "GITHUB_EVENT_NAME": "workflow_dispatch",
            "GITHUB_WORKSPACE": str(self.workspace),
            "UV_RUST_CACHE_ACCEPTANCE_STAGE": "source",
            "UV_RUST_CACHE_JOB_STARTED": "1700000000",
            "UV_RUST_CACHE_JOB_DEADLINE": str(
                1700000000 + acceptance.JOB_BUDGET_SECONDS
            ),
        }
        self.controller = {
            "commit": self.github["GITHUB_SHA"],
            "tree": "f" * 40,
            "path": str(self.controller_path),
            "helper_sha256": "a" * 64,
            "cache_helper_sha256": "b" * 64,
            "workflow_blobs": {path: "c" * 40 for path in acceptance.WORKFLOW_PATHS},
        }
        self.platform = {
            "system": "Linux",
            "architecture": "x86_64",
            "distribution": "ubuntu",
            "distribution_version": "24.04",
            "libc": ["glibc", "2.39"],
            "image": {},
        }

    def plan(self, case: str, *, stage: str = "source") -> dict[str, Any]:
        environment = {**self.environment, "UV_RUST_CACHE_ACCEPTANCE_STAGE": stage}
        with (
            mock.patch.object(acceptance, "ROOT", self.controller_path),
            mock.patch.object(
                acceptance,
                "controller_identity",
                return_value=copy.deepcopy(self.controller),
            ),
        ):
            return acceptance.plan_value(case, environment)

    def state(self, plan: dict[str, Any]) -> dict[str, Any]:
        identity = {
            "source": {
                key: plan["source"][key]
                for key in ("repository", "commit", "tree", "path")
            },
            "platform": copy.deepcopy(self.platform),
            "configuration": {
                "environment": {
                    "compilation": {
                        "names": sorted(cache.REQUIRED_ENVIRONMENT),
                        "sha256": cache.digest(cache.REQUIRED_ENVIRONMENT),
                    },
                    "registries": {"names": [], "sha256": cache.digest({})},
                },
                "cargo": [{"role": "ancestor-0/config.toml", "sha256": "d" * 64}],
            },
            "tools": cache_tests.fake_tools(),
            "paths": {
                "workspace": plan["workspace"],
                "source": plan["source"]["path"],
                "cargo_home": str(self.root / "home/.cargo"),
                "target": str(Path(plan["workspace"]) / "target"),
                "download_paths": list(cache.DOWNLOAD_PATHS),
                "target_paths": list(cache.TARGET_PATHS),
            },
            "implementation": {
                "commit": self.controller["commit"],
                "tree": self.controller["tree"],
                "script_sha256": self.controller["cache_helper_sha256"],
                "cache_action": cache.CACHE_ACTION,
            },
            "git": cache_tests.fake_executable("git", "6"),
            "github": copy.deepcopy(plan["github"]),
        }
        identity["source"]["inputs"] = [
            {
                "path": name,
                "kind": kind,
                "git_blob": "a" * 40,
                "sha256": "a" * 64,
                "size": 10,
            }
            for name, kind in (
                ("Cargo.lock", "dependency"),
                ("Cargo.toml", "dependency"),
                ("rust-toolchain.toml", "configuration"),
            )
        ]
        manifest = {
            "format": cache.FORMAT,
            "version": cache.VERSION,
            "workload": copy.deepcopy(cache.WORKLOAD),
            "identity": identity,
            "policy": {
                "save_allowed": plan["save_allowed"],
                "key_namespace": plan["namespace"],
            },
            "keys": cache.cache_keys(identity, plan["namespace"]),
        }
        keys = manifest["keys"]
        expected = plan["expected_target"]
        target_primary = "" if expected == "unattempted" else keys["target"]
        target_matched = (
            ""
            if expected in {"unattempted", "miss"}
            else keys["target_restore"] + acceptance.BASE
            if expected == "baseline"
            else keys["target"]
        )
        downloads_matched = (
            ""
            if plan["mode"] == "seed" or expected == "malformed"
            else keys["downloads"]
        )
        observations = {
            "downloads": cache.restore_observation(
                keys,
                "downloads",
                keys["downloads"],
                downloads_matched,
                str(downloads_matched == keys["downloads"]).lower(),
            ),
            "target": cache.restore_observation(
                keys,
                "target",
                target_primary,
                target_matched,
                str(target_matched == keys["target"]).lower(),
            ),
        }
        return cache.restored_manifest(manifest, observations)

    def write(self, directory: Path, name: str, value: Any) -> str:
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        return (
            acceptance.write_bytes(path, value)
            if isinstance(value, bytes)
            else acceptance.write_json(path, value)
        )

    def phase(
        self,
        plan: dict[str, Any],
        directory: Path,
        name: str,
        index: int,
        *,
        outcome: str = "success",
    ) -> tuple[int, int]:
        start = index * 10_000_000_000
        end = start + 9_000_000_000
        self.write(
            directory,
            f"phase-{name}-start.json",
            {
                "case": plan["case"],
                "phase": name,
                "event": "start",
                "monotonic_ns": start,
                "observed_at": "2023-11-14T22:13:20.000Z",
            },
        )
        self.write(
            directory,
            f"phase-{name}-end.json",
            {
                "case": plan["case"],
                "phase": name,
                "event": "end",
                "monotonic_ns": end,
                "elapsed_ns": end - start,
                "observed_at": "2023-11-14T22:13:20.900Z",
                "outcome": outcome,
            },
        )
        return start, end


class SourceAndManifestContract(AcceptanceFixture):
    def test_every_case_has_the_fixed_source_policy_and_namespace(self) -> None:
        for case, expected in acceptance.CASE_DATA.items():
            with self.subTest(case=case):
                plan = self.plan(case, stage=acceptance.STAGES[1])
                self.assertEqual(
                    (
                        plan["source"]["commit"],
                        plan["source"]["relative_directory"],
                        plan["mode"],
                        plan["save_allowed"],
                        plan["expected_target"],
                    ),
                    expected,
                )
                acceptance.recorded_plan(
                    plan, case, self.controller, self.github, acceptance.STAGES[1]
                )
                state = self.state(plan)
                self.assertEqual(acceptance.recorded_manifest(plan, state), state)
                self.assertTrue(all(acceptance.expectations(plan, state).values()))
        self.assertEqual(
            self.plan("candidate-relocated")["source"]["relative_directory"],
            "relocated-source",
        )

    def test_dispatch_and_plan_admission_fail_closed(self) -> None:
        for name, value in (
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_REPOSITORY", "another/repo"),
            ("GITHUB_SHA", "short"),
            ("GITHUB_WORKFLOW_SHA", "0" * 40),
            (
                "GITHUB_WORKFLOW_REF",
                "another/repo/.github/workflows/other.yml@refs/heads/main",
            ),
            ("GITHUB_RUN_ID", "0"),
            ("GITHUB_RUN_ATTEMPT", "-1"),
            ("UV_RUST_CACHE_JOB_DEADLINE", "1700000001"),
            ("UV_RUST_CACHE_ACCEPTANCE_STAGE", "all"),
        ):
            with (
                self.subTest(name=name),
                mock.patch.object(acceptance, "ROOT", self.controller_path),
                mock.patch.object(
                    acceptance, "controller_identity", return_value=self.controller
                ),
                self.assertRaises((acceptance.AcceptanceError, cache.CacheError)),
            ):
                acceptance.plan_value(
                    "baseline-cold", {**self.environment, name: value}
                )
        with self.assertRaisesRegex(
            acceptance.AcceptanceError, "outside the selected stage"
        ):
            self.plan("malformed-cache-fixture")

    def test_recorded_plan_rejects_other_source_and_run(self) -> None:
        original = self.plan("candidate-fallback")
        mutations = (
            lambda value: value["source"].update(commit=acceptance.BASE),
            lambda value: value["source"].update(tree="0" * 40),
            lambda value: value.update(namespace="unscoped"),
            lambda value: value.update(save_allowed=1),
            lambda value: value.update(deadline_unix=value["deadline_unix"] + 1),
            lambda value: value["process_owner"].update(commit="0" * 40),
            lambda value: value["controller"].update(helper_sha256="0" * 64),
            lambda value: value["github"].update(GITHUB_RUN_ATTEMPT="1"),
        )
        for index, mutate in enumerate(mutations):
            value = copy.deepcopy(original)
            mutate(value)
            with (
                self.subTest(index=index),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.recorded_plan(
                    value, "candidate-fallback", self.controller, self.github, "source"
                )

    def test_manifest_rejects_cross_source_namespace_path_and_permission(self) -> None:
        plan = self.plan("candidate-fallback")
        original = self.state(plan)
        mutations = (
            lambda value: value["identity"]["identity"]["source"].update(
                commit=acceptance.BASE
            ),
            lambda value: value["identity"]["identity"]["source"].update(
                path="/elsewhere/source"
            ),
            lambda value: value["identity"]["identity"]["paths"].update(
                target="/elsewhere/target"
            ),
            lambda value: value["identity"]["identity"]["implementation"].update(
                commit="0" * 40
            ),
            lambda value: value["identity"]["identity"]["github"].update(
                GITHUB_RUN_ID="999"
            ),
            lambda value: value["identity"]["policy"].update(save_allowed=False),
            lambda value: value["identity"]["policy"].update(
                key_namespace=plan["fault_namespace"]
            ),
            lambda value: value["restore"]["target"].update(
                matched=value["identity"]["keys"]["target"]
            ),
            lambda value: value["restore"]["downloads"].update(exact=1),
            lambda value: value.update(restore=[]),
        )
        for index, mutate in enumerate(mutations):
            value = copy.deepcopy(original)
            mutate(value)
            value["identity_sha256"] = digest_bytes(cache.json_bytes(value["identity"]))
            with (
                self.subTest(index=index),
                self.assertRaises((acceptance.AcceptanceError, cache.CacheError)),
            ):
                acceptance.recorded_manifest(plan, value)

    def test_observation_distinguishes_miss_exact_and_fallback(self) -> None:
        plan = self.plan("baseline-cold")
        state = self.state(plan)
        state["restore"]["target"] = cache.restore_observation(
            state["identity"]["keys"], "target", "", "", ""
        )
        self.assertFalse(acceptance.expectations(plan, state)["target"])
        plan = self.plan("candidate-fallback")
        state = self.state(plan)
        state["restore"]["target"]["matched"] = (
            state["identity"]["keys"]["target_restore"] + "1" * 40
        )
        self.assertFalse(acceptance.expectations(plan, state)["target"])

    def test_read_only_and_failure_outcomes_never_become_cache_misses(self) -> None:
        for case in acceptance.SOURCE_CASES:
            with self.subTest(case=case):
                plan = self.plan(case)
                state = self.state(plan)
                self.assertEqual(
                    state["identity"]["policy"]["save_allowed"],
                    acceptance.CASE_DATA[case][3],
                )
        outcomes = acceptance.step_outcomes(["restore=failure", "observation=skipped"])
        self.assertEqual(outcomes["restore"], "failure")
        self.assertEqual(outcomes["workload"], "unknown")
        for values in (
            ["restore=success", "restore=failure"],
            ["restore=miss"],
            ["unapproved=success"],
        ):
            with (
                self.subTest(values=values),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.step_outcomes(values)


class MeasurementContract(AcceptanceFixture):
    def setUp(self) -> None:
        super().setUp()
        self.source = self.workspace / "source"
        self.target = self.workspace / "target"
        self.artifact = cargo_artifact(self.source, self.target)

    def records(
        self, *records: dict[str, Any], complete: bool = True
    ) -> dict[str, Any]:
        return acceptance.cargo_records(
            cargo_stdout(*records), self.source, self.target, require_complete=complete
        )

    def test_actual_artifact_fields_are_retained_and_counted(self) -> None:
        nonfresh = {**self.artifact, "fresh": False}
        explicit = cargo_artifact(self.source, self.target, explicit_target=True)
        records = self.records(
            self.artifact,
            nonfresh,
            explicit,
            {"reason": "build-script-executed"},
            {"reason": "build-finished", "success": True},
        )
        self.assertEqual(
            (
                records["compiler_artifact_records"],
                records["fresh_records"],
                records["nonfresh_records"],
            ),
            (3, 2, 1),
        )
        self.assertEqual(records["record_reasons"]["build-script-executed"], 1)
        self.assertEqual(records["records"][0]["record"], self.artifact)
        self.assertEqual(
            records["records"][0]["unit"]["profile"], self.artifact["profile"]
        )
        self.assertEqual(records["records"][0]["unit"]["output_layouts"], ["native"])
        self.assertEqual(
            records["records"][2]["unit"]["output_layouts"], ["target:" + cache.HOST]
        )
        self.assertNotEqual(
            records["records"][0]["logical_unit_sha256"],
            records["records"][2]["logical_unit_sha256"],
        )

    def test_relocation_normalizes_paths_but_not_source_identity_fields(self) -> None:
        relocated = self.workspace / "relocated-source"
        original = self.records(
            self.artifact, {"reason": "build-finished", "success": True}
        )
        moved = acceptance.cargo_records(
            cargo_stdout(
                cargo_artifact(relocated, self.target),
                {"reason": "build-finished", "success": True},
            ),
            relocated,
            self.target,
            require_complete=True,
        )
        self.assertEqual(
            original["logical_unit_multiset"], moved["logical_unit_multiset"]
        )
        self.assertNotEqual(
            original["records"][0]["record"]["package_id"],
            moved["records"][0]["record"]["package_id"],
        )
        self.assertEqual(
            acceptance.normalize_locations(
                str(self.source) + "-other/file", self.source, self.target
            ),
            str(self.source) + "-other/file",
        )

    def test_non_cargo_text_is_not_counted_as_a_compilation(self) -> None:
        raw = b"procedural macro notice\n" + cargo_stdout(
            self.artifact, {"reason": "build-finished", "success": True}
        )
        records = acceptance.cargo_records(
            raw, self.source, self.target, require_complete=True
        )
        self.assertEqual(records["other_stdout_lines"], 1)
        self.assertEqual(
            records["other_stdout_sha256"], digest_bytes(b"procedural macro notice")
        )
        self.assertEqual(records["compiler_artifact_records"], 1)

    def test_missing_malformed_and_ambiguous_cargo_records_fail_closed(self) -> None:
        bad = []
        for mutation in (
            lambda value: value.update(fresh=1),
            lambda value: value.update(package_id=""),
            lambda value: value.update(manifest_path="Cargo.toml"),
            lambda value: value.update(features=["a", "a"]),
            lambda value: value.update(filenames=["/outside/file"]),
            lambda value: value["target"].update(kind=[]),
            lambda value: value["target"].update(test="true"),
            lambda value: value["profile"].update(debuginfo=True),
            lambda value: value["profile"].update(test=None),
        ):
            value = copy.deepcopy(self.artifact)
            mutation(value)
            bad.append(
                cargo_stdout(value, {"reason": "build-finished", "success": True})
            )
        bad.extend(
            (
                b"",
                cargo_stdout(self.artifact),
                cargo_stdout({"reason": "build-finished", "success": True}),
                cargo_stdout(
                    self.artifact, {"reason": "build-finished", "success": False}
                ),
                cargo_stdout(
                    self.artifact,
                    {"reason": "build-finished", "success": True},
                    {"reason": "build-finished", "success": True},
                ),
                b'{"reason":"unknown"}\n',
                b'{"reason":"compiler-artifact","fresh":true,"fresh":false}\n',
            )
        )
        for index, raw in enumerate(bad):
            with (
                self.subTest(index=index),
                self.assertRaises((acceptance.AcceptanceError, cache.CacheError)),
            ):
                acceptance.cargo_records(
                    raw, self.source, self.target, require_complete=True
                )
        partial = self.records(self.artifact, complete=False)
        self.assertEqual(partial["build_finished"], [])
        self.assertEqual(partial["compiler_artifact_records"], 1)

    def test_reported_times_are_separate_from_command_elapsed(self) -> None:
        result = acceptance.reported_times(
            b"\x1b[32m"
            + nextest_stderr(tests=4941, passed=4941, skipped=4)
            + b"\x1b[0m"
        )
        self.assertEqual(result["cargo_reported_build_seconds"], 1.25)
        self.assertEqual(result["nextest_reported_test_seconds"], 0.5)
        self.assertEqual(
            (result["tests"], result["passed"], result["skipped"]), (4941, 4941, 4)
        )
        self.assertEqual(acceptance.duration_seconds("1h 2m 3.5s"), 3723.5)
        for value in ("", "1s2m", "1m1m", "nan", "-1s"):
            with (
                self.subTest(value=value),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.duration_seconds(value)
        for raw in (
            b"",
            nextest_stderr() * 2,
            nextest_stderr().replace(b"1 test run", b"2 tests run"),
        ):
            with self.subTest(raw=raw), self.assertRaises(acceptance.AcceptanceError):
                acceptance.reported_times(raw)

    def test_only_complete_pinned_success_summaries_are_accepted(self) -> None:
        for details in (
            "",
            "1 slow",
            "1 flaky",
            "1 leaky",
            "1 slow, 1 flaky",
            "1 slow, 1 leaky",
            "1 flaky, 1 leaky",
            "1 slow, 1 flaky, 1 leaky",
        ):
            with self.subTest(annotations=details):
                result = acceptance.reported_times(
                    nextest_stderr(
                        tests=3,
                        passed=3,
                        skipped=1,
                        binaries=2,
                        skipped_binaries=1,
                        annotations=details,
                    )
                )
                self.assertEqual(result["skipped_binaries"], 1)
                self.assertEqual(
                    result["passed_annotations"],
                    {part.split()[1]: 1 for part in details.split(", ")}
                    if details
                    else {},
                )
        raw = nextest_stderr()
        for invalid in (
            raw.replace(b"1 test run", b"1/2 tests run"),
            raw.replace(b"1 passed, 0 skipped", b"1 passed, 1 failed, 0 skipped"),
            raw.replace(b"1 passed, 0 skipped", b"1 passed, 1 exec failed, 0 skipped"),
            raw.replace(b"1 passed, 0 skipped", b"1 passed, 1 timed out, 0 skipped"),
            raw.replace(b"1 passed, 0 skipped", b"1 passed"),
            raw.replace(b"1 passed, 0 skipped", b"0 passed, 0 skipped"),
            raw.replace(b"1 passed, 0 skipped", b"1 passed, 1 skipped"),
            raw.replace(b"Starting 1 test ", b"Starting 1 tests "),
            raw.replace(b"across 1 binary", b"across 0 binaries"),
            raw.replace(b"1 test run", b"1 tests run"),
            raw + b"     Summary [  0.501s] 1/2 tests run: 1 passed, 0 skipped\n",
            nextest_stderr(skipped=1).replace(b"1 test skipped)", b"1 tests skipped)"),
            nextest_stderr(skipped=1).replace(
                b"1 test skipped)", b"1 test skipped that already passed)"
            ),
            nextest_stderr(annotations="0 slow"),
            nextest_stderr(annotations="2 slow"),
            nextest_stderr(annotations="1 flaky, 1 slow"),
            nextest_stderr(annotations="1 slow, 1 slow"),
            nextest_stderr(annotations="1 unknown"),
        ):
            with (
                self.subTest(raw=invalid),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.reported_times(invalid)

    def test_junit_binds_a_new_run_and_distinguishes_setup_and_flaky_cases(
        self,
    ) -> None:
        summary = acceptance.junit_summary(
            junit_xml(setup=True, body="<flakyFailure/>")
        )
        self.assertEqual(
            (
                summary["testcases"],
                summary["setup_script_cases"],
                summary["flaky_attempts"],
            ),
            (1, 1, 1),
        )
        command = {
            "command_started_unix_ns": 1700000000000000000,
            "command_finished_unix_ns": 1700000002000000000,
        }
        acceptance.validate_junit_invocation(
            summary, command, acceptance.reported_times(nextest_stderr())
        )
        for body in ("<failure/>", "<error/>", "<skipped/>"):
            with self.subTest(body=body), self.assertRaises(acceptance.AcceptanceError):
                acceptance.validate_junit_invocation(
                    acceptance.junit_summary(junit_xml(body=body)),
                    command,
                    acceptance.reported_times(nextest_stderr()),
                )
        with self.assertRaisesRegex(acceptance.AcceptanceError, "another invocation"):
            acceptance.validate_junit_invocation(
                summary,
                {"command_started_unix_ns": 1, "command_finished_unix_ns": 2},
                acceptance.reported_times(nextest_stderr()),
            )

    def test_junit_rejects_duplicate_tests_entity_documents_and_bad_identity(
        self,
    ) -> None:
        duplicate = junit_xml().replace(
            b"</testsuite>",
            b'<testcase classname="example::example" name="works"/></testsuite>',
        )
        for raw in (
            duplicate,
            b"<!DOCTYPE test>" + junit_xml(),
            junit_xml().replace(
                b"2023-11-14T22:13:21.000Z", b"2023-11-14T22:13:21.000"
            ),
            junit_xml().replace(
                str(uuid.UUID(int=1, version=4)).encode(), b"not-a-run"
            ),
            junit_xml().replace(b'time="0.5"', b'time="nan"'),
        ):
            with (
                self.subTest(raw=raw),
                self.assertRaises((acceptance.AcceptanceError, ValueError)),
            ):
                acceptance.junit_summary(raw)


class EvidenceContract(AcceptanceFixture):
    def test_json_files_are_bounded_and_do_not_follow_links(self) -> None:
        path = self.root / "value.json"
        path.write_bytes(b'{"answer":42}\n')
        self.assertEqual(acceptance.read_json(path), {"answer": 42})
        with self.assertRaises(acceptance.AcceptanceError):
            acceptance.bounded_bytes(path, 1)
        link = self.root / "link.json"
        link.symlink_to(path)
        with self.assertRaises(OSError):
            acceptance.read_json(link)
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}'):
            with self.subTest(raw=raw), self.assertRaises(acceptance.AcceptanceError):
                acceptance.json_value(raw)

    def test_fingerprint_bytes_and_decoded_configuration_are_rechecked(self) -> None:
        directory = self.root / "evidence"
        original = fixture_fingerprints()
        self.write(directory, "fingerprints-built.json", original)
        self.assertEqual(
            acceptance.checked_fingerprints(directory, "built")["files"], 1
        )
        mutations = (
            lambda value: value["entries"][0].update(configuration={"other": True}),
            lambda value: value["entries"][0].pop("configuration"),
            lambda value: value["entries"][0].update(path="../outside"),
            lambda value: value["entries"].append(copy.deepcopy(value["entries"][0])),
        )
        for index, mutate in enumerate(mutations):
            value = copy.deepcopy(original)
            mutate(value)
            value.update(
                files=len(value["entries"]),
                bytes=sum(item["size"] for item in value["entries"]),
                sha256=cache.digest(value["entries"]),
            )
            replace_json(directory / "fingerprints-built.json", value)
            with (
                self.subTest(index=index),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.checked_fingerprints(directory, "built")

    def test_payload_content_digest_counts_and_metadata_are_rechecked(self) -> None:
        original = fixture_payload(b"{}\n")
        self.assertEqual(
            acceptance.checked_payload_inventory(original, hash_contents=True), original
        )
        self.assertEqual(
            acceptance.checked_payload_inventory(
                fixture_payload(), hash_contents=False
            )["files"],
            1,
        )
        for mutation in (
            lambda value: value.update(content_tree_sha256="0" * 64),
            lambda value: value.update(files=9),
            lambda value: value["entries"][0].update(path="../outside"),
            lambda value: value["entries"][0].update(
                path=cache.PROFILE + "/incremental/forbidden"
            ),
            lambda value: value["entries"][0].update(sha256="short"),
        ):
            value = copy.deepcopy(original)
            mutation(value)
            if value["content_tree_sha256"] != "0" * 64:
                value["content_tree_sha256"] = cache.digest(value["entries"])
            with (
                self.subTest(value=value),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.checked_payload_inventory(value, hash_contents=True)

    def test_upload_inventory_rejects_links_and_both_byte_budgets(self) -> None:
        directory = self.root / "upload"
        self.write(directory, "one", b"12345")
        self.write(directory, "two", b"67890")
        self.assertEqual(len(acceptance.evidence_inventory(directory)), 2)
        with (
            mock.patch.object(acceptance, "MAX_EVIDENCE_FILE_BYTES", 4),
            self.assertRaises(acceptance.AcceptanceError),
        ):
            acceptance.evidence_inventory(directory)
        with (
            mock.patch.object(acceptance, "MAX_EVIDENCE_BYTES", 9),
            self.assertRaises(acceptance.AcceptanceError),
        ):
            acceptance.evidence_inventory(directory)
        (directory / "linked").symlink_to(directory / "one")
        with self.assertRaisesRegex(acceptance.AcceptanceError, "symbolic link"):
            acceptance.evidence_inventory(directory)

    def test_phase_order_rejects_missing_overlapping_and_undeclared_phases(
        self,
    ) -> None:
        plan = self.plan("baseline-cold")
        state = self.state(plan)
        directory = self.root / "phases"
        directory.mkdir()
        names = ["setup", "restore", "workload", "prune", "save"]
        for index, name in enumerate(names, 1):
            self.phase(plan, directory, name, index)
        self.assertEqual(acceptance.checked_phase_order(plan, directory, state), names)
        path = directory / "phase-save-start.json"
        old = json.loads(path.read_bytes())
        value = {**old, "monotonic_ns": 1}
        replace_json(path, value)
        with self.assertRaises(acceptance.AcceptanceError):
            acceptance.checked_phase_order(plan, directory, state)
        replace_json(path, old)
        self.phase(plan, directory, "fault", 6)
        with self.assertRaisesRegex(acceptance.AcceptanceError, "phase set"):
            acceptance.checked_phase_order(plan, directory, state)


class CompleteEvidenceFixture(AcceptanceFixture):
    """Create decoder fixtures, never substitutes for actual cache-service evidence."""

    def command_record(
        self,
        plan: dict[str, Any],
        directory: Path,
        state: dict[str, Any],
        name: str,
        phase_bounds: tuple[int, int],
        stdout: bytes,
        stderr: bytes,
        start: dict[str, Any],
        setup: dict[str, Any],
    ) -> dict[str, Any]:
        self.write(directory, name + ".stdout", stdout)
        self.write(directory, name + ".stderr", stderr)
        target = Path(state["identity"]["identity"]["paths"]["target"])
        metadata: dict[str, Any] = {
            "case": plan["case"],
            "source": plan["source"],
            "plan_sha256": acceptance.file_reference(directory / "plan.json")["sha256"],
            "restore_manifest_sha256": acceptance.file_reference(
                directory / "restore-manifest.json"
            )["sha256"],
            "runtime_environment": acceptance.expected_runtime(
                plan, state, start, setup
            ),
            "process_owner": {
                **acceptance.PROCESS_OWNER,
                "repository_path": str(self.workspace / "process-owner"),
            },
        }
        cargo = cache.common_outputs(state["identity"])["cargo"]
        if name == "fetch":
            command = [cargo, "fetch", "--locked"]
        elif name == "nextest":
            command = [
                cargo,
                *cache.WORKLOAD["cargo_arguments"],
                "--cargo-message-format=json",
            ]
        else:
            metadata.update(
                python=cache_tests.fake_executable("python3", "8"),
                script={
                    **cache_tests.fake_executable("pruner", "7"),
                    **setup["pruner"],
                },
                marker_mtime_ns=1700000000000000000,
            )
            command = [
                metadata["python"]["invocation"],
                str(
                    Path(plan["source"]["path"])
                    / "scripts/prune_cargo_workspace_cache.py"
                ),
                str(target / cache.PROFILE),
                str(self.workspace / "results" / plan["case"] / "cargo-cache-marker"),
            ]
        started = phase_bounds[0] + 1_000_000_000
        finished = started + 2_000_000_000
        process_group = 1000 + list(acceptance.CASE_DATA).index(plan["case"])
        record = {
            "format": acceptance.FORMAT + "-command",
            "version": acceptance.VERSION,
            "name": name,
            "command": command,
            "cwd": plan["source"]["path"],
            "metadata": metadata,
            "deadline_unix": plan["deadline_unix"],
            "status": "success",
            "launcher_pid": 999,
            "child_pid": process_group,
            "process_group": process_group,
            "command_returncode": 0,
            "exit_code": 0,
            "termination_complete": True,
            "cleanup_status": "complete",
            "cleanup_events": [
                {
                    "event": {
                        "stage": "initial",
                        "syscall": "killpg",
                        "process_group": process_group,
                        "signal": 0,
                        "result": "absent",
                        "errno": errno.ESRCH,
                    },
                    "count": 1,
                    "first_observed_at": "2023-11-14T22:13:22.000Z",
                    "last_observed_at": "2023-11-14T22:13:22.000Z",
                }
            ],
            "command_started_unix_ns": 1700000000000000000,
            "command_finished_unix_ns": 1700000002000000000,
            "command_started_monotonic_ns": started,
            "command_finished_monotonic_ns": finished,
            "command_elapsed_ns": finished - started,
            "stdout": acceptance.file_reference(directory / (name + ".stdout")),
            "stderr": acceptance.file_reference(directory / (name + ".stderr")),
        }
        self.write(directory, "command-" + name + ".json", record)
        return record

    def make_case(
        self, directory: Path, case: str, *, stage: str = "source"
    ) -> tuple[dict[str, Any], dict[str, Any], dict[str, str]]:
        plan = self.plan(case, stage=stage)
        state = self.state(plan)
        self.write(directory, "plan.json", plan)
        manifest_sha256 = self.write(directory, "restore-manifest.json", state)
        actual_source = state["identity"]["identity"]["source"]
        start = {
            "format": acceptance.FORMAT + "-start",
            "version": acceptance.VERSION,
            "plan": plan,
            "actual_source": actual_source,
            "machine": {"fixture": True},
            "temporary_directory": str(self.root / "home/code/tmp/acceptance" / case),
            "started_at": "2023-11-14T22:13:20.000Z",
        }
        setup = {
            "source": actual_source,
            "uv": {
                **cache_tests.fake_executable("uv", "6"),
                "version": "uv 0.12.10 (fixture x86_64-unknown-linux-gnu)",
            },
            "python_requests": {"sha256": "6" * 64, "size": 7, "values": ["3.12.13"]},
            "installed_pythons": [
                {
                    "version": "3.12.13",
                    "key": "cpython-3.12.13-linux-x86_64-gnu",
                    "executable": cache_tests.fake_executable("python3.12", "5"),
                }
            ],
            "nextest_configuration": {"sha256": "6" * 64, "size": 19},
            "pruner": {"sha256": "7" * 64, "size": 19},
            "filesystems": {
                "/btrfs": {"type": "btrfs", "device": 1},
                "/tmpfs": {"type": "tmpfs", "device": 2},
                "/minix": {"type": "minix", "device": 3},
            },
            "dbus_address_sha256": "4" * 64,
        }
        self.write(directory, "start.json", start)
        self.write(directory, "setup-evidence.json", setup)
        self.write(
            directory,
            "source-final.json",
            {"status": "success", "source": actual_source},
        )
        self.write(
            directory,
            "identity-final.json",
            {
                "unchanged": True,
                "source": actual_source,
                "restore_manifest_sha256": manifest_sha256,
            },
        )
        expected = acceptance.expectations(plan, state)
        if plan["expected_target"] == "malformed":
            self.write(
                directory, "restored-rustc-info.json", acceptance.MALFORMED_RUSTC_INFO
            )
            expected["malformed_metadata_restored"] = True
        self.write(
            directory,
            "restore-observation.json",
            {
                "source_manifest": str(self.root / "manifest.json"),
                "source_manifest_sha256": manifest_sha256,
                "service_observation_origin": "official-cache-actions",
                "restore": state["restore"],
                "expectations": expected,
            },
        )
        outcomes = {name: "skipped" for name in acceptance.OUTCOME_NAMES}
        outcomes.update(
            {
                name: "success"
                for name in acceptance.OUTCOME_NAMES
                if name == "setup" or name.startswith("setup-")
            }
        )
        outcomes["observation"] = "success"
        if plan["mode"] == "seed":
            outcomes.update(
                {
                    name: "success"
                    for name in ("seed-prepare", "seed-downloads", "seed-observe")
                }
            )
        else:
            outcomes["restore"] = "success"
        writable = (
            plan["mode"] == "full"
            and plan["save_allowed"]
            and not state["restore"]["target"]["exact"]
        )
        names = [
            "setup",
            "restore",
            "fault" if plan["mode"] == "fault-fixture" else "workload",
        ]
        if writable:
            names.append("prune")
        if writable or plan["mode"] in {"seed", "fault-fixture"}:
            names.append("save")
        phases = {
            name: self.phase(plan, directory, name, index)
            for index, name in enumerate(names, 1)
        }
        if plan["mode"] == "fault-fixture":
            self.fault_fixture(plan, directory, state, manifest_sha256)
            outcomes["fault-prepare"] = outcomes["fault-save"] = "success"
        else:
            outcomes["workload"] = "success"
            name = "fetch" if plan["mode"] == "seed" else "nextest"
            source = Path(plan["source"]["path"])
            target = Path(state["identity"]["identity"]["paths"]["target"])
            stdout = (
                b""
                if name == "fetch"
                else cargo_stdout(
                    cargo_artifact(source, target, fresh=not writable),
                    {"reason": "build-finished", "success": True},
                )
            )
            stderr = b"" if name == "fetch" else nextest_stderr()
            self.command_record(
                plan,
                directory,
                state,
                name,
                phases["workload"],
                stdout,
                stderr,
                start,
                setup,
            )
            self.write(
                directory,
                "post-workload-identity.json",
                {"unchanged": True, "source": actual_source},
            )
            if name == "fetch":
                outcomes["seed-save-plan"] = outcomes["seed-save"] = "success"
                self.write(
                    directory,
                    "seed-publication-plan.json",
                    {
                        "kind": "isolated-downloads-fixture",
                        "source_manifest_sha256": manifest_sha256,
                        "downloads_key": state["identity"]["keys"]["downloads"],
                        "downloads_paths": list(cache.DOWNLOAD_PATHS),
                        "source": plan["source"],
                        "normal_nextest_workload_ran": False,
                    },
                )
            else:
                self.workload_fixture(plan, directory, state, stdout, stderr)
                if writable:
                    self.write(directory, "cargo-cache-marker", b"")
                    self.command_record(
                        plan,
                        directory,
                        state,
                        "prune",
                        phases["prune"],
                        b"",
                        b"",
                        start,
                        setup,
                    )
                    self.write(
                        directory, "fingerprints-pruned.json", fixture_fingerprints()
                    )
                    self.write(directory, "payload-pruned.json", fixture_payload())
                    outcomes["prune"] = outcomes["save"] = outcomes["save-target"] = (
                        "success"
                    )
        return plan, state, outcomes

    def workload_fixture(
        self,
        plan: dict[str, Any],
        directory: Path,
        state: dict[str, Any],
        stdout: bytes,
        stderr: bytes,
    ) -> None:
        source = Path(plan["source"]["path"])
        target = Path(state["identity"]["identity"]["paths"]["target"])
        records = acceptance.cargo_records(
            stdout, source, target, require_complete=True
        )
        records_sha256 = self.write(directory, "cargo-artifacts.json", records)
        junit = junit_xml(list(acceptance.CASE_DATA).index(plan["case"]) + 1)
        junit_sha256 = self.write(directory, "junit.xml", junit)
        value: dict[str, Any] = {
            "command_complete": True,
            "completed_full_workload": True,
            "cargo_records": {
                "path": "cargo-artifacts.json",
                "sha256": records_sha256,
                **{
                    key: records[key]
                    for key in (
                        "compiler_artifact_records",
                        "fresh_records",
                        "nonfresh_records",
                        "unit_multiset",
                        "logical_unit_multiset",
                    )
                },
            },
            "executable_artifacts": [
                {
                    "path": str(
                        Path(records["records"][0]["record"]["executable"]).relative_to(
                            target
                        )
                    ),
                    "sha256": "9" * 64,
                    "size": 10,
                }
            ],
            "reported_times": acceptance.reported_times(stderr),
            "junit": {
                "source_path": str(target / "nextest/ci-linux/junit.xml"),
                "sha256": junit_sha256,
                "size": len(junit),
                "mtime_ns": 1700000002000000000,
                "summary": acceptance.junit_summary(junit),
            },
        }
        if plan["expected_target"] == "malformed":
            repaired = b'{"rustc_fingerprint":123}\n'
            value["rustc_info_after"] = {
                "sha256": self.write(directory, "rustc-info-after.json", repaired),
                "size": len(repaired),
                "valid_json": True,
                "differs_from_fixture": True,
            }
        self.write(directory, "workload-evidence.json", value)
        self.write(
            directory,
            "fingerprints-restored.json",
            fixture_fingerprints(populated=plan["expected_target"] != "miss"),
        )
        self.write(directory, "fingerprints-built.json", fixture_fingerprints())
        self.write(directory, "payload-restored.json", fixture_payload())
        self.write(directory, "payload-built.json", fixture_payload())

    def fault_fixture(
        self,
        plan: dict[str, Any],
        directory: Path,
        state: dict[str, Any],
        manifest_sha256: str,
    ) -> None:
        original = b'{"rustc_fingerprint":123}\n'
        before = fixture_payload(original)
        after = fixture_payload(acceptance.MALFORMED_RUSTC_INFO)
        identity = {
            **state["identity"],
            "policy": {"save_allowed": False, "key_namespace": plan["fault_namespace"]},
            "keys": cache.cache_keys(
                state["identity"]["identity"], plan["fault_namespace"]
            ),
        }
        original_sha256 = self.write(directory, "rustc-info-original.json", original)
        before_sha256 = self.write(directory, "fault-payload-before.json", before)
        after_sha256 = self.write(directory, "fault-payload-after.json", after)
        identity_sha256 = self.write(directory, "fault-identity.json", identity)
        self.write(
            directory,
            "fault-fixture.json",
            {
                "kind": "intentionally-malformed-target-fixture",
                "source_manifest_sha256": manifest_sha256,
                "source": plan["source"],
                "normal_nextest_workload_ran": False,
                "original_metadata_sha256": original_sha256,
                "malformed_metadata_sha256": digest_bytes(
                    acceptance.MALFORMED_RUSTC_INFO
                ),
                "before_inventory_sha256": before_sha256,
                "after_inventory_sha256": after_sha256,
                "changed_paths": [".rustc_info.json"],
                "identity_sha256": identity_sha256,
                "target_key": identity["keys"]["target"],
                "target_paths": list(cache.TARGET_PATHS),
            },
        )

    def case_result(
        self,
        directory: Path,
        plan: dict[str, Any],
        outcomes: dict[str, str],
        *,
        job_status: str = "success",
    ) -> tuple[dict[str, Any], str]:
        self.write(
            directory,
            "workflow-outcomes.json",
            {"job_status": job_status, "steps": outcomes},
        )
        evaluation = acceptance.evaluate_case(plan, directory, outcomes, job_status)
        result = {
            "format": acceptance.FORMAT + "-case",
            "version": acceptance.VERSION,
            "case": plan["case"],
            "plan_sha256": acceptance.file_reference(directory / "plan.json")["sha256"],
            "job_status": job_status,
            "outcomes": outcomes,
            **evaluation,
            "evidence": acceptance.evidence_inventory(directory),
            "finished_at": "2023-11-14T22:13:23.000Z",
        }
        return result, self.write(directory, "result.json", result)

    def make_stage(self, *, stage: str = "source") -> tuple[Path, list[str]]:
        directory = self.root / "artifacts"
        selected = (
            acceptance.SOURCE_CASES + acceptance.FAULT_CASES
            if stage == acceptance.STAGES[1]
            else acceptance.SOURCE_CASES
        )
        associations = []
        prefix = "ci-rust-cache-acceptance-123456-2-"
        for number, case in enumerate(selected, 1):
            root = directory / (prefix + case)
            plan, _state, outcomes = self.make_case(root, case, stage=stage)
            result, checksum = self.case_result(root, plan, outcomes)
            self.assertTrue(result["complete"], result["errors"])
            associations.append(f"{case}={number}:{number:064x}:{checksum}:success")
        return directory, associations

    def aggregate(
        self,
        directory: Path,
        associations: list[str],
        name: str,
        *,
        stage: str = "source",
    ) -> tuple[int, dict[str, Any]]:
        output = self.root / (name + ".json")
        with mock.patch.object(
            acceptance,
            "controller_identity",
            return_value=copy.deepcopy(self.controller),
        ):
            returncode = acceptance.aggregate(
                directory,
                output,
                {**self.environment, "UV_RUST_CACHE_ACCEPTANCE_STAGE": stage},
                associations,
            )
        return returncode, json.loads(output.read_bytes())


class CaseAndAggregateContract(CompleteEvidenceFixture):
    def test_full_source_and_fault_stages_recompute_from_all_evidence(self) -> None:
        directory, associations = self.make_stage(stage=acceptance.STAGES[1])
        returncode, result = self.aggregate(
            directory, associations, "complete", stage=acceptance.STAGES[1]
        )
        self.assertEqual(returncode, 0)
        self.assertEqual(len(result["cases"]), 8)
        self.assertTrue(result["source_stage_complete"])
        self.assertTrue(result["selected_stage_complete"])
        self.assertTrue(all(result["source_comparisons"].values()))
        self.assertTrue(result["malformed_cache_stage_complete"])
        self.assertTrue(all(result["malformed_cache_comparisons"].values()))
        self.assertFalse(result["production_adoption_qualified"])
        self.assertFalse(result["speedup_claimed"])

    def test_malformed_cache_consumer_must_match_its_actual_producer(self) -> None:
        directory, associations = self.make_stage(stage=acceptance.STAGES[1])
        returncode, result = self.aggregate(
            directory, associations, "fault-bindings", stage=acceptance.STAGES[1]
        )
        self.assertEqual(returncode, 0)
        cases = result["cases"]
        self.assertTrue(all(acceptance.malformed_cache_comparisons(cases).values()))
        changed = copy.deepcopy(cases)
        changed["malformed-cache-consumer"]["summary"]["keys"]["target"] = "another-key"
        self.assertFalse(
            acceptance.malformed_cache_comparisons(changed)["isolated_fixture_restored"]
        )
        changed = copy.deepcopy(cases)
        changed["malformed-cache-fixture"]["summary"]["declared_compatibility"][
            "platform"
        ] = {"system": "different"}
        self.assertFalse(
            acceptance.malformed_cache_comparisons(changed)[
                "declared_compatibility_equal"
            ]
        )

    def test_failed_restore_is_adverse_even_with_other_complete_evidence(self) -> None:
        directory = self.root / "failed-restore"
        plan, _state, outcomes = self.make_case(directory, "baseline-exact")
        outcomes["restore"] = "failure"
        result = acceptance.evaluate_case(plan, directory, outcomes, "failure")
        self.assertFalse(result["complete"])
        self.assertFalse(result["checks"]["prior_job_success"])
        self.assertFalse(result["checks"]["restore"])
        self.assertNotIn("cache_miss", result)

    def test_read_only_save_and_missing_cleanup_proof_cannot_complete(self) -> None:
        directory = self.root / "read-only"
        plan, state, outcomes = self.make_case(directory, "baseline-exact")
        outcomes["save"] = "success"
        result = acceptance.evaluate_case(plan, directory, outcomes, "success")
        self.assertFalse(result["checks"]["save_policy"])
        outcomes["save"] = "skipped"
        command = json.loads((directory / "command-nextest.json").read_bytes())
        command["cleanup_events"] = []
        replace_json(directory / "command-nextest.json", command)
        self.assertFalse(acceptance.command_succeeded(command))
        with self.assertRaisesRegex(acceptance.AcceptanceError, "did not complete"):
            acceptance.checked_workload(plan, directory, state)

    def test_fault_fixture_rejects_a_changed_other_file_even_after_rehashing(
        self,
    ) -> None:
        directory = self.root / "fault"
        plan, state, _outcomes = self.make_case(
            directory, "malformed-cache-fixture", stage=acceptance.STAGES[1]
        )
        path = directory / "fault-payload-after.json"
        value = json.loads(path.read_bytes())
        value["entries"][-1]["sha256"] = "1" * 64
        value["content_tree_sha256"] = cache.digest(value["entries"])
        replace_json(path, value)
        fixture = json.loads((directory / "fault-fixture.json").read_bytes())
        fixture["after_inventory_sha256"] = acceptance.file_reference(path)["sha256"]
        replace_json(directory / "fault-fixture.json", fixture)
        with self.assertRaisesRegex(acceptance.AcceptanceError, "unrelated payload"):
            acceptance.checked_fault_fixture(plan, directory, state)

    def test_fault_fixture_rejects_forged_metadata_content_identity(self) -> None:
        directory = self.root / "fault-metadata"
        plan, state, _outcomes = self.make_case(
            directory, "malformed-cache-fixture", stage=acceptance.STAGES[1]
        )
        path = directory / "fault-payload-after.json"
        value = json.loads(path.read_bytes())
        value["entries"][0]["sha256"] = "1" * 64
        value["content_tree_sha256"] = cache.digest(value["entries"])
        replace_json(path, value)
        fixture = json.loads((directory / "fault-fixture.json").read_bytes())
        fixture["after_inventory_sha256"] = acceptance.file_reference(path)["sha256"]
        replace_json(directory / "fault-fixture.json", fixture)
        with self.assertRaisesRegex(
            acceptance.AcceptanceError, "retained metadata bytes"
        ):
            acceptance.checked_fault_fixture(plan, directory, state)

    def test_partial_single_case_cannot_be_reported_as_a_complete_stage(self) -> None:
        directory = self.root / "partial-artifacts"
        root = directory / "ci-rust-cache-acceptance-123456-2-downloads-seed"
        plan, _state, outcomes = self.make_case(root, "downloads-seed")
        value, checksum = self.case_result(root, plan, outcomes)
        self.assertTrue(value["complete"])
        returncode, result = self.aggregate(
            directory, [f"downloads-seed=1:{'1' * 64}:{checksum}:success"], "partial"
        )
        self.assertEqual(returncode, 1)
        self.assertTrue(result["cases"]["downloads-seed"]["complete"])
        self.assertFalse(result["source_stage_complete"])
        self.assertFalse(result["selected_stage_complete"])

    def test_service_download_failure_and_unexpected_case_are_adverse(self) -> None:
        directory, associations = self.make_stage()
        failed = [
            value.removesuffix(":success") + ":failure"
            if value.startswith("baseline-exact=")
            else value
            for value in associations
        ]
        returncode, result = self.aggregate(directory, failed, "download-failed")
        self.assertEqual(returncode, 1)
        self.assertFalse(result["cases"]["baseline-exact"]["complete"])
        self.assertEqual(
            result["cases"]["baseline-exact"]["service_artifact"]["download_outcome"],
            "failure",
        )
        (directory / "unexpected").mkdir()
        returncode, result = self.aggregate(directory, associations, "unexpected-case")
        self.assertEqual(returncode, 1)
        self.assertFalse(result["exact_case_set"])

    def test_rehashed_case_inventory_does_not_authorize_false_completion(self) -> None:
        directory, associations = self.make_stage()
        root = directory / "ci-rust-cache-acceptance-123456-2-candidate-exact"
        (root / "junit.xml").unlink()
        result = json.loads((root / "result.json").read_bytes())
        result["evidence"] = acceptance.evidence_inventory(root)
        replace_json(root / "result.json", result)
        checksum = acceptance.file_reference(root / "result.json")["sha256"]
        associations = [
            f"candidate-exact=5:{5:064x}:{checksum}:success"
            if value.startswith("candidate-exact=")
            else value
            for value in associations
        ]
        returncode, aggregate = self.aggregate(
            directory, associations, "forged-completion"
        )
        self.assertEqual(returncode, 1)
        self.assertFalse(aggregate["cases"]["candidate-exact"]["complete"])
        self.assertIn("error", aggregate["cases"]["candidate-exact"])

    def test_artifact_associations_reject_unselected_or_malformed_records(self) -> None:
        selected = acceptance.SOURCE_CASES
        good = f"baseline-cold=1:{'a' * 64}:{'b' * 64}:success"
        self.assertEqual(
            acceptance.service_artifacts([good], selected)["baseline-cold"]["id"], 1
        )
        for values in (
            [good, good],
            [good, good.replace("baseline-cold=", "baseline-exact=")],
            [good.replace("=1:", "=0:")],
            [good.removesuffix(":success")],
            [good.replace(":success", ":miss")],
            [f"malformed-cache-fixture=1:{'a' * 64}:{'b' * 64}:success"],
            ["baseline-cold=:::success"],
        ):
            with (
                self.subTest(values=values),
                self.assertRaises(acceptance.AcceptanceError),
            ):
                acceptance.service_artifacts(values, selected)


def workflow_jobs(source: str) -> dict[str, list[str]]:
    lines = plan_tests.block(source.splitlines(), "jobs:", 0)
    names = [
        match.group(1)
        for line in lines
        if (match := re.fullmatch(r"  ([a-z][a-z0-9-]*):", line))
    ]
    if not names or len(names) != len(set(names)):
        raise ValueError("Unsupported workflow jobs")
    return {name: plan_tests.block(lines, name + ":", 2) for name in names}


def workflow_field(lines: list[str], name: str, indent: int = 4) -> str:
    prefix = " " * indent + name + ": "
    values = [line.removeprefix(prefix) for line in lines if line.startswith(prefix)]
    if len(values) != 1:
        raise ValueError("Expected one workflow field")
    return plan_tests.scalar(values[0])


def workflow_steps(job: list[str]) -> list[Any]:
    lines = plan_tests.block(job, "steps:", 4)
    starts = [index for index, line in enumerate(lines) if line.startswith("      - ")]
    if not starts or any(line.strip() for line in lines[: starts[0]]):
        raise ValueError("Unsupported workflow step list")
    return [
        plan_tests.Step(lines[start:end])
        for start, end in zip(starts, [*starts[1:], len(lines)], strict=True)
    ]


def step_by_name(steps: list[Any], name: str) -> Any:
    found = [step for step in steps if step.field("name", "") == name]
    if len(found) != 1:
        raise ValueError("Expected one named workflow step")
    return found[0]


class WorkflowContract(AcceptanceFixture):
    def setUp(self) -> None:
        super().setUp()
        self.caller = (ROOT / acceptance.WORKFLOW_PATHS[0]).read_text()
        self.callee = (ROOT / acceptance.WORKFLOW_PATHS[1]).read_text()
        self.jobs = workflow_jobs(self.caller)
        self.accept_job = workflow_jobs(self.callee)["accept"]
        self.steps = workflow_steps(self.accept_job)

    def test_manual_caller_has_only_the_fixed_sequential_cases(self) -> None:
        for source, event in (
            (self.caller, "workflow_dispatch"),
            (self.callee, "workflow_call"),
        ):
            lines = source.splitlines()
            self.assertEqual(
                plan_tests.flat_mapping(plan_tests.block(lines, "permissions:", 0), 2),
                {"contents": "read"},
            )
            triggers = [
                line.strip()
                for line in plan_tests.block(lines, "on:", 0)
                if re.fullmatch(r"  [a-z_]+:", line)
            ]
            self.assertEqual(triggers, [event + ":"])
            self.assertEqual(source.count("permissions:"), 1)
            self.assertNotIn("pull_request_target", source)
            self.assertNotIn("secrets: inherit", source)
        cases = (*acceptance.SOURCE_CASES, *acceptance.FAULT_CASES)
        self.assertEqual(list(self.jobs), [*cases, "aggregate"])
        for index, case in enumerate(cases):
            with self.subTest(case=case):
                job = self.jobs[case]
                self.assertEqual(
                    workflow_field(job, "uses"),
                    "./.github/workflows/ci-rust-cache-acceptance-job.yml",
                )
                self.assertEqual(
                    plan_tests.flat_mapping(plan_tests.block(job, "with:", 4), 6),
                    {"case": case, "stage": "${{ inputs.stage }}"},
                )
                if index:
                    self.assertEqual(workflow_field(job, "needs"), cases[index - 1])
        self.assertEqual(
            workflow_field(self.jobs["downloads-seed"], "if"),
            "${{ github.repository == 'astral-sh/uv-dev' && github.event_name == 'workflow_dispatch' }}",
        )
        for case in acceptance.FAULT_CASES:
            self.assertEqual(
                workflow_field(self.jobs[case], "if"),
                "${{ inputs.stage == 'source-and-malformed-cache' }}",
            )
        self.assertEqual(
            workflow_field(self.accept_job, "if"),
            "${{ github.repository == 'astral-sh/uv-dev' && github.event_name == 'workflow_dispatch' }}",
        )
        self.assertEqual(
            workflow_field(self.accept_job, "runs-on"), "depot-ubuntu-24.04-16"
        )
        self.assertEqual(workflow_field(self.accept_job, "timeout-minutes"), "45")

    def test_case_checkouts_and_service_calls_are_source_and_policy_bound(self) -> None:
        checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
        controller = step_by_name(self.steps, "Checkout acceptance controller")
        self.assertEqual(controller.field("uses"), checkout)
        self.assertEqual(
            controller.mapping("with"),
            {
                "ref": "${{ github.sha }}",
                "path": "controller",
                "persist-credentials": "false",
            },
        )
        source = step_by_name(self.steps, "Checkout the fixed test source")
        self.assertEqual(source.field("uses"), checkout)
        self.assertEqual(
            source.mapping("with"),
            {
                "repository": acceptance.REPOSITORY,
                "ref": "${{ steps.plan.outputs.source-commit }}",
                "path": "${{ steps.plan.outputs.source-relative-directory }}",
                "persist-credentials": "false",
            },
        )
        process = step_by_name(self.steps, "Checkout process-control source")
        self.assertEqual(
            process.mapping("with")["ref"], acceptance.PROCESS_OWNER["commit"]
        )
        self.assertEqual(
            process.mapping("with")["sparse-checkout"], acceptance.PROCESS_OWNER["path"]
        )
        self.assertEqual(process.mapping("with")["persist-credentials"], "false")
        restore = step_by_name(self.steps, "Restore the source-specific cache")
        self.assertEqual(
            restore.field("uses"), "./controller/.github/actions/uv-rust-cache/restore"
        )
        self.assertEqual(
            restore.mapping("with"),
            {
                "source-directory": "${{ steps.plan.outputs.source-directory }}",
                "source-repository": acceptance.REPOSITORY,
                "source-commit": "${{ steps.plan.outputs.source-commit }}",
                "save-if": "${{ steps.plan.outputs.save-if }}",
                "key-namespace": "${{ steps.plan.outputs.namespace }}",
            },
        )
        save = step_by_name(self.steps, "Save the completed source cache")
        self.assertEqual(
            save.field("uses"), "./controller/.github/actions/uv-rust-cache/save"
        )
        self.assertEqual(
            save.mapping("with"),
            {
                "manifest": "${{ steps.observation.outputs.manifest }}",
                "manifest-sha256": "${{ steps.observation.outputs.manifest-sha256 }}",
            },
        )
        direct = [
            step.field("uses", "")
            for step in self.steps
            if step.field("uses", "").startswith("actions/cache/")
        ]
        self.assertEqual(
            direct,
            [
                cache.CACHE_ACTION.replace("actions/cache@", "actions/cache/restore@"),
                cache.CACHE_ACTION.replace("actions/cache@", "actions/cache/save@"),
                cache.CACHE_ACTION.replace("actions/cache@", "actions/cache/save@"),
            ],
        )
        fault = step_by_name(self.steps, "Publish the isolated malformed fixture")
        self.assertIn(
            "steps.fault-prepare.outputs.fault-fixture-ready == 'true'",
            fault.field("if"),
        )
        self.assertEqual(
            fault.mapping("with"),
            {
                "path": "${{ steps.fault-prepare.outputs.target-paths }}",
                "key": "${{ steps.fault-prepare.outputs.target-key }}",
            },
        )

    def test_each_artifact_download_uses_its_exact_id_and_digest_failure(self) -> None:
        steps = workflow_steps(self.jobs["aggregate"])
        download = "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"
        by_id = {
            step.field("id"): step
            for step in steps
            if step.field("uses", "") == download
        }
        self.assertEqual(
            set(by_id), {"download-" + case for case in acceptance.CASE_DATA}
        )
        for case in acceptance.CASE_DATA:
            with self.subTest(case=case):
                step = by_id["download-" + case]
                self.assertEqual(
                    step.field("if"),
                    "${{ always() && needs." + case + ".outputs.artifact-id != '' }}",
                )
                self.assertEqual(
                    step.mapping("with"),
                    {
                        "artifact-ids": "${{ needs." + case + ".outputs.artifact-id }}",
                        "path": "case-artifacts/ci-rust-cache-acceptance-${{ github.run_id }}-${{ github.run_attempt }}-"
                        + case,
                        "digest-mismatch": "error",
                    },
                )
        aggregate = step_by_name(steps, "Verify the selected acceptance stage")
        environment = aggregate.mapping("env")
        for case in acceptance.CASE_DATA:
            self.assertIn(
                "${{ needs."
                + case
                + ".outputs.artifact-id }}:${{ needs."
                + case
                + ".outputs.artifact-digest }}:${{ needs."
                + case
                + ".outputs.result-sha256 }}:${{ steps.download-"
                + case
                + ".outcome }}",
                environment.values(),
            )

    def test_full_workload_gets_only_the_cargo_json_observation_flag(self) -> None:
        plan = self.plan("baseline-exact")
        state = self.state(plan)
        directory = acceptance.evidence_directory(plan)
        directory.mkdir(parents=True)
        observed = []

        def run(
            _directory: Path, _name: str, command: list[str], *_args: Any, **kwargs: Any
        ) -> dict[str, Any]:
            kwargs["before_launch"]()
            observed.append(command)
            return {"exit_code": 7}

        with (
            mock.patch.object(acceptance, "manifest_state", return_value=state),
            mock.patch.object(acceptance, "runtime_environment", return_value=({}, {})),
            mock.patch.object(
                acceptance, "load_process_groups", return_value=(None, {})
            ),
            mock.patch.object(acceptance, "record_inventory"),
            mock.patch.object(acceptance, "phase"),
            mock.patch.object(acceptance, "run_command", side_effect=run),
            mock.patch.object(acceptance, "command_succeeded", return_value=False),
            mock.patch.object(
                acceptance,
                "actual_source",
                return_value=state["identity"]["identity"]["source"],
            ),
            mock.patch.object(
                acceptance,
                "analyze_workload",
                return_value={"completed_full_workload": False},
            ),
            mock.patch.object(cache, "write_outputs"),
        ):
            result = acceptance.run_work(
                plan,
                self.root / "manifest.json",
                "a" * 64,
                self.root,
                {},
                self.root / "outputs",
            )
        self.assertEqual(result, 7)
        self.assertEqual(
            observed,
            [
                [
                    cache.common_outputs(state["identity"])["cargo"],
                    *cache.WORKLOAD["cargo_arguments"],
                    "--cargo-message-format=json",
                ]
            ],
        )

    def test_workflow_source_extractor_rejects_changed_shape(self) -> None:
        with self.assertRaises(ValueError):
            workflow_jobs("jobs:\n  one:\n    uses: ./one\n  one:\n    uses: ./two\n")
        with self.assertRaises(ValueError):
            workflow_field(["    uses: ./one", "    uses: ./two"], "uses")
        with self.assertRaises(ValueError):
            workflow_steps(["    steps:", "      unexpected: value"])


class RealProcessControls(AcceptanceFixture):
    def setUp(self) -> None:
        super().setUp()
        if PROCESS_OWNER_REPOSITORY is None or os.name != "posix":
            self.skipTest("Pass the exact process-owner repository for POSIX controls")
        self.environment = dict(os.environ)
        self.groups, self.owner = acceptance.load_process_groups(
            PROCESS_OWNER_REPOSITORY, self.environment
        )
        self.interpreter = cache.file_identity(Path(sys.executable))
        self.helper = cache.file_identity(ACTION / "acceptance.py")

    def run_control(
        self, name: str, code: str, *, seconds: float = 5, grace_seconds: float = 0.5
    ) -> tuple[Path, dict[str, Any]]:
        directory = self.root / name
        directory.mkdir()

        def verify() -> None:
            acceptance.require(
                cache.file_identity(Path(sys.executable)) == self.interpreter
                and cache.file_identity(ACTION / "acceptance.py") == self.helper,
                "Process-control source or interpreter changed",
            )

        record = acceptance.run_command(
            directory,
            "nextest",
            [self.interpreter["invocation"], "-c", code],
            self.root,
            self.environment,
            self.groups,
            {
                "case": "local-control-" + name,
                "process_owner": self.owner,
                "interpreter": self.interpreter,
                "acceptance_helper": self.helper,
            },
            deadline_unix=time.time() + seconds,
            before_launch=verify,
            grace_seconds=grace_seconds,
        )
        self.retain(name, directory, record)
        return directory, record

    def retain(self, name: str, directory: Path, record: dict[str, Any]) -> None:
        if RETAIN_DIRECTORY is None:
            return
        destination = RETAIN_DIRECTORY / name
        shutil.copytree(directory, destination)
        acceptance.write_json(
            destination / "control.json",
            {
                "test": self.id(),
                "process_owner": self.owner,
                "interpreter": self.interpreter,
                "acceptance_helper": self.helper,
                "command_record_sha256": acceptance.file_reference(
                    destination / "command-nextest.json"
                )["sha256"],
                "exit_code": record["exit_code"],
                "termination_complete": record["termination_complete"],
            },
        )

    def assert_group_absent(self, process_group: int) -> None:
        with self.assertRaises(ProcessLookupError):
            os.killpg(process_group, 0)

    def test_success_and_original_nonzero_exit(self) -> None:
        for name, code, expected in (
            ("success", "print('done', flush=True)", 0),
            ("nonzero", "raise SystemExit(7)", 7),
        ):
            with self.subTest(name=name):
                directory, record = self.run_control(name, code)
                self.assertEqual(record["exit_code"], expected)
                self.assertTrue(record["termination_complete"])
                self.assertEqual(acceptance.command_succeeded(record), expected == 0)
                self.assertEqual(
                    record["stdout"],
                    acceptance.file_reference(directory / "nextest.stdout"),
                )
                self.assert_group_absent(record["process_group"])

    def test_deadline_cleans_owned_descendant_without_touching_another_group(
        self,
    ) -> None:
        child_code = "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('child-ready', flush=True); time.sleep(30)"
        leader_code = (
            "import signal,subprocess,sys,time\n"
            f"child=subprocess.Popen([sys.executable,'-c',{child_code!r}], stdout=subprocess.PIPE, text=True)\n"
            "def stop(_signum,_frame):\n child.kill(); child.wait(); raise SystemExit(0)\n"
            "signal.signal(signal.SIGTERM,stop)\n"
            "assert child.stdout.readline().strip() == 'child-ready'\n"
            "print('ready '+str(child.pid),flush=True)\n"
            "time.sleep(30)\n"
        )
        unrelated = subprocess.Popen(
            [self.interpreter["invocation"], "-c", "import time; time.sleep(30)"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        try:
            directory, record = self.run_control(
                "deadline-descendant", leader_code, seconds=1.5, grace_seconds=1
            )
            self.assertEqual(
                (record["status"], record["exit_code"]), ("timed_out", 124)
            )
            self.assertTrue(record["termination_complete"])
            self.assertIsNone(unrelated.poll())
            self.assert_group_absent(record["process_group"])
            child_pid = int(
                (directory / "nextest.stdout")
                .read_text()
                .strip()
                .removeprefix("ready ")
            )
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)
            self.assertTrue(
                all(
                    item["event"].get("process_group", record["process_group"])
                    == record["process_group"]
                    for item in record["cleanup_events"]
                )
            )
        finally:
            _code, complete = self.groups.stop_process(
                unrelated, 1, process_group=unrelated.pid
            )
            self.assertTrue(complete)

    def test_successful_leader_cannot_leave_a_live_owned_descendant(self) -> None:
        code = "import subprocess,sys; child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); print(child.pid,flush=True)"
        directory, record = self.run_control(
            "leader-exits", code, seconds=5, grace_seconds=2
        )
        self.assertEqual(record["command_returncode"], 0)
        self.assertEqual(
            (record["status"], record["exit_code"]), ("left_descendants", 1)
        )
        self.assertTrue(record["termination_complete"])
        self.assertFalse(acceptance.command_succeeded(record))
        self.assert_group_absent(record["process_group"])
        child_pid = int((directory / "nextest.stdout").read_text().strip())
        with self.assertRaises(ProcessLookupError):
            os.kill(child_pid, 0)

    def test_repeated_signals_do_not_interrupt_bounded_cleanup(self) -> None:
        directory = self.root / "repeated-signals"
        stop = threading.Event()
        sent: list[int] = []
        code = "import signal,time; signal.signal(signal.SIGTERM,lambda *_: print('term',flush=True)); print('ready',flush=True); time.sleep(30)"

        def signal_when_ready() -> None:
            expires = time.monotonic() + 5
            output = directory / "nextest.stdout"
            for text, signum in (("ready", signal.SIGTERM), ("term", signal.SIGINT)):
                while not stop.is_set() and time.monotonic() < expires:
                    try:
                        ready = text in output.read_text()
                    except FileNotFoundError:
                        ready = False
                    if ready:
                        os.kill(os.getpid(), signum)
                        sent.append(int(signum))
                        break
                    stop.wait(0.01)
                else:
                    return

        thread = threading.Thread(target=signal_when_ready, daemon=True)
        with self.groups.RunSignals() as outer_signals:
            thread.start()
            try:
                _directory, record = self.run_control(
                    "repeated-signals", code, seconds=6, grace_seconds=1
                )
            finally:
                stop.set()
                thread.join(timeout=2)
            self.assertIsNone(
                outer_signals.signum, "A signal missed the command's guard"
            )
        self.assertEqual(sent, [int(signal.SIGTERM), int(signal.SIGINT)])
        self.assertEqual(
            (record["status"], record["exit_code"], record["signal"]),
            ("interrupted", 128 + signal.SIGTERM, int(signal.SIGTERM)),
        )
        self.assertTrue(record["termination_complete"])
        self.assertTrue(
            any(
                item["event"].get("stage") == "kill"
                and item["event"].get("result") == "sent"
                for item in record["cleanup_events"]
            )
        )
        self.assert_group_absent(record["process_group"])

    def test_launch_verification_failure_does_not_start_a_child(self) -> None:
        directory = self.root / "verification-failure"
        directory.mkdir()

        def reject() -> None:
            raise acceptance.AcceptanceError("The observed source changed")

        record = acceptance.run_command(
            directory,
            "nextest",
            [self.interpreter["invocation"], "-c", "raise SystemExit(99)"],
            self.root,
            self.environment,
            self.groups,
            {"case": "local-control-verification", "process_owner": self.owner},
            deadline_unix=time.time() + 5,
            before_launch=reject,
            grace_seconds=0.5,
        )
        self.retain("verification-failure", directory, record)
        self.assertEqual(record["exit_code"], 1)
        self.assertIsNone(record["child_pid"])
        self.assertTrue(record["termination_complete"])
        self.assertFalse(acceptance.command_succeeded(record))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--process-owner-repository", type=Path)
    parser.add_argument("--retain-directory", type=Path)
    arguments, remainder = parser.parse_known_args()
    PROCESS_OWNER_REPOSITORY = arguments.process_owner_repository
    RETAIN_DIRECTORY = arguments.retain_directory
    if RETAIN_DIRECTORY is not None:
        RETAIN_DIRECTORY.mkdir(parents=True, exist_ok=False)
    unittest.main(argv=[sys.argv[0], *remainder])
