"""Exercise CodSpeed discovery without contacting GitHub or uploading profiles."""

import contextlib
import importlib.util
import io
import json
import os
import socket
import subprocess
import sys
import unittest
from collections import deque
from dataclasses import dataclass
from pathlib import Path
from unittest.mock import call, patch

SCRIPT = Path(__file__).resolve().parents[1] / "codspeed-profiles.py"
SPEC = importlib.util.spec_from_file_location("codspeed_profiles", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
profiles = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(profiles)

SOURCE_SHA = "a" * 40
DESTINATION_SHA = "b" * 40
RUN_ID = "42"
COMPARE = f"repos/astral-sh/uv-dev/compare/{SOURCE_SHA}...{DESTINATION_SHA}"
RUN = f"repos/astral-sh/uv/actions/runs/{RUN_ID}"
JOBS = f"{RUN}/jobs?per_page=100"
ARTIFACTS = f"{RUN}/artifacts?per_page=100"
RUNS = (
    "repos/astral-sh/uv/actions/workflows/ci.yml/runs"
    f"?event=push&head_sha={SOURCE_SHA}&per_page=100"
)


def source_run():
    return {
        "id": int(RUN_ID),
        "head_sha": SOURCE_SHA,
        "head_branch": "main",
        "event": "push",
        "path": ".github/workflows/ci.yml",
        "repository": {"full_name": "astral-sh/uv"},
        "head_repository": {"full_name": "astral-sh/uv"},
    }


def successful_jobs():
    return [
        {"jobs": [{"name": "bench / simulated", "conclusion": "success"}]},
        {
            "jobs": [
                {
                    "name": "bench / walltime on aarch64 linux",
                    "conclusion": "success",
                }
            ]
        },
    ]


def available_artifacts():
    return [
        {"artifacts": [{"name": "codspeed-profiles-simulation", "expired": False}]},
        {"artifacts": [{"name": "codspeed-profiles-walltime", "expired": False}]},
    ]


@dataclass(frozen=True)
class Response:
    path: str
    stdout: str
    stderr: str = ""
    returncode: int = 0


def response(path, value):
    return Response(path, json.dumps(value))


def failure(path, *, stdout="partial", stderr="gh: Server Error (HTTP 502)\n"):
    return Response(path, stdout, stderr, 1)


class CodSpeedDiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.reads = deque()
        self.calls = []
        self.errors = []
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(
            patch.dict(
                os.environ,
                {
                    "GITHUB_REPOSITORY": "astral-sh/uv-dev",
                    "GITHUB_REF": "refs/heads/main",
                    "GITHUB_SHA": DESTINATION_SHA,
                },
                clear=True,
            )
        )
        self.stack.enter_context(
            patch.object(subprocess, "run", side_effect=self.run_github)
        )
        self.stack.enter_context(
            patch.object(subprocess, "Popen", side_effect=AssertionError("subprocess"))
        )
        self.stack.enter_context(
            patch.object(socket, "getaddrinfo", side_effect=AssertionError("network"))
        )
        self.stack.enter_context(
            patch.object(
                socket.socket, "connect", side_effect=AssertionError("network")
            )
        )
        for name in ("request", "upload_request", "upload_profile", "oidc_token"):
            self.stack.enter_context(
                patch.object(profiles, name, side_effect=AssertionError("upload"))
            )
        self.sleep = self.stack.enter_context(patch.object(profiles.time, "sleep"))

    def run_github(self, command, **kwargs):
        self.assertEqual(command[:2], ["gh", "api"])
        options = command[2:-1]
        if options[:2] == ["--method", "GET"]:
            options = options[2:]
        self.assertIn(options, ([], ["--paginate", "--slurp"]))
        self.assertTrue(kwargs["text"])
        self.assertTrue(self.reads, f"Unexpected GitHub read: {command!r}")
        result = self.reads.popleft()
        self.assertEqual(command[-1], result.path)
        self.calls.append((command, kwargs))
        if result.returncode and kwargs.get("check"):
            error = subprocess.CalledProcessError(
                result.returncode,
                command,
                output=result.stdout,
                stderr=result.stderr,
            )
            self.errors.append(error)
            raise error
        return subprocess.CompletedProcess(
            command, result.returncode, result.stdout, result.stderr
        )

    def find(self, *, dispatched=True):
        arguments = [str(SCRIPT), "find", SOURCE_SHA]
        if dispatched:
            arguments.extend(("--run-id", RUN_ID))
        output = io.StringIO()
        with patch.object(sys, "argv", arguments), contextlib.redirect_stdout(output):
            profiles.main()
        self.assertEqual(list(self.reads), [])
        return output.getvalue()

    def test_find_recovers_from_the_jobs_api_failure(self):
        self.reads.extend(
            (
                response(COMPARE, {"status": "ahead"}),
                response(RUN, source_run()),
                failure(JOBS, stdout=json.dumps(successful_jobs()[:1])),
                response(JOBS, successful_jobs()),
                response(ARTIFACTS, available_artifacts()),
            )
        )
        self.assertEqual(self.find(), f"run-id={RUN_ID}\n")
        self.assertEqual(self.sleep.call_args_list, [call(5)])
        self.assertTrue(all(kwargs["timeout"] == 60 for _, kwargs in self.calls))
        self.assertTrue(
            all(command[2:4] == ["--method", "GET"] for command, _ in self.calls)
        )

    def test_find_recovers_from_an_object_read_failure(self):
        self.reads.extend(
            (
                failure(COMPARE),
                response(COMPARE, {"status": "identical"}),
                response(RUN, source_run()),
                response(JOBS, successful_jobs()),
                response(ARTIFACTS, available_artifacts()),
            )
        )
        self.assertEqual(self.find(), f"run-id={RUN_ID}\n")
        self.assertEqual(self.sleep.call_args_list, [call(5)])

    def test_paginated_retry_discards_partial_pages(self):
        self.reads.extend(
            (
                failure(JOBS, stdout='[{"jobs":[{"id":1}]}]'),
                response(JOBS, [{"jobs": [{"id": 1}]}, {"jobs": [{"id": 2}]}]),
            )
        )
        self.assertEqual(profiles.github_items(JOBS, "jobs"), [{"id": 1}, {"id": 2}])
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.sleep.call_args_list, [call(5)])

    def test_paginated_exhaustion_raises_the_last_error(self):
        self.reads.extend(failure(JOBS, stdout=str(index)) for index in range(5))
        output = io.StringIO()
        with (
            contextlib.redirect_stdout(output),
            self.assertRaises(subprocess.CalledProcessError) as raised,
        ):
            profiles.github_items(JOBS, "jobs")
        self.assertIs(raised.exception, self.errors[-1])
        self.assertEqual(raised.exception.output, "4")
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(len(self.calls), 5)
        self.assertEqual(self.sleep.call_args_list, [call(5)] * 4)

    def test_object_exhaustion_keeps_its_sanitized_error(self):
        self.reads.extend(failure(RUN, stderr="private response") for _ in range(5))
        with self.assertRaisesRegex(
            RuntimeError, "^Could not read GitHub workflow information$"
        ) as raised:
            profiles.github_object(RUN)
        self.assertTrue(raised.exception.__suppress_context__)
        self.assertEqual(len(self.calls), 5)
        self.assertEqual(self.sleep.call_args_list, [call(5)] * 4)

    def test_missing_mirror_is_not_retried(self):
        self.reads.append(failure(COMPARE, stderr="gh: Not Found (HTTP 404)\n"))
        self.assertEqual(self.find(), "run-id=\n")
        self.assertEqual(len(self.calls), 1)
        self.sleep.assert_not_called()

    def test_missing_required_run_uses_the_read_budget(self):
        self.reads.extend(
            failure(RUN, stderr="gh: Not Found (HTTP 404)\n") for _ in range(5)
        )
        with self.assertRaisesRegex(
            RuntimeError, "^Could not read GitHub workflow information$"
        ):
            profiles.github_object(RUN)
        self.assertEqual(len(self.calls), 5)
        self.assertEqual(self.sleep.call_args_list, [call(5)] * 4)

    def test_malformed_json_is_not_retried(self):
        self.reads.append(Response(JOBS, "not JSON"))
        with self.assertRaises(json.JSONDecodeError):
            profiles.github_items(JOBS, "jobs")
        self.assertEqual(len(self.calls), 1)
        self.sleep.assert_not_called()

    def test_malformed_page_is_not_retried(self):
        self.reads.append(response(JOBS, [{"wrong": []}]))
        with self.assertRaises(KeyError):
            profiles.github_items(JOBS, "jobs")
        self.assertEqual(len(self.calls), 1)
        self.sleep.assert_not_called()

    def test_object_json_is_not_retried(self):
        self.reads.append(Response(RUN, "not JSON"))
        with self.assertRaises(json.JSONDecodeError):
            profiles.github_object(RUN)
        self.assertEqual(len(self.calls), 1)
        self.sleep.assert_not_called()

    def test_timeout_is_not_retried(self):
        error = subprocess.TimeoutExpired(["gh", "api", JOBS], 60)
        with (
            patch.object(subprocess, "run", side_effect=error),
            self.assertRaises(subprocess.TimeoutExpired) as raised,
        ):
            profiles.github_items(JOBS, "jobs")
        self.assertIs(raised.exception, error)
        self.sleep.assert_not_called()

    def test_os_error_is_not_retried(self):
        error = OSError("cannot start GitHub CLI")
        with (
            patch.object(subprocess, "run", side_effect=error),
            self.assertRaises(OSError) as raised,
        ):
            profiles.github_object(RUN)
        self.assertIs(raised.exception, error)
        self.sleep.assert_not_called()

    def test_undispatched_find_keeps_source_selection(self):
        wrong_run = source_run()
        wrong_run["head_repository"] = {"full_name": "untrusted/uv"}
        self.reads.extend(
            (
                response(COMPARE, {"status": "ahead"}),
                response(RUNS, [{"workflow_runs": [wrong_run, source_run()]}]),
                response(JOBS, successful_jobs()),
                response(ARTIFACTS, available_artifacts()),
            )
        )
        self.assertEqual(self.find(dispatched=False), f"run-id={RUN_ID}\n")
        self.sleep.assert_not_called()

    def test_unsuccessful_benchmarks_are_not_imported(self):
        self.reads.extend(
            (
                response(COMPARE, {"status": "ahead"}),
                response(RUN, source_run()),
                response(JOBS, [{"jobs": []}]),
            )
        )
        self.assertEqual(self.find(), "run-id=\n")
        self.sleep.assert_not_called()

    def test_expired_artifacts_are_not_imported(self):
        artifacts = available_artifacts()
        artifacts[1]["artifacts"][0]["expired"] = True
        self.reads.extend(
            (
                response(COMPARE, {"status": "ahead"}),
                response(RUN, source_run()),
                response(JOBS, successful_jobs()),
                response(ARTIFACTS, artifacts),
            )
        )
        self.assertEqual(self.find(), "run-id=\n")
        self.sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
