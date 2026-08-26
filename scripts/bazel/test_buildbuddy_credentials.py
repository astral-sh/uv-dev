# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run with `uv run --offline scripts/bazel/test_buildbuddy_credentials.py`."""

import json
import os
import runpy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HELPER = Path(__file__).with_name("buildbuddy_credentials.py")
FAKE_KEY = "fake-buildbuddy-test-key"
FAKE_KEY_BYTES = FAKE_KEY.encode("ascii")
REQUEST = json.dumps({"uri": "grpcs://remote.buildbuddy.io"})


class BuildBuddyCredentialsTests(unittest.TestCase):
    def run_helper(
        self,
        request=REQUEST,
        *,
        arguments=("get",),
        key=FAKE_KEY,
        host=None,
        key_file=None,
    ):
        # Do not inherit credentials or other configuration from the test environment.
        environment = {} if key is None else {"BUILDBUDDY_API_KEY": key}
        if host is not None:
            environment["UV_BAZEL_CREDENTIAL_HOST"] = host
        if key_file is not None:
            environment["UV_BAZEL_KEY_FILE"] = str(key_file)
        return subprocess.run(
            [sys.executable, str(HELPER), *arguments],
            input=request,
            capture_output=True,
            text=True,
            env=environment,
            timeout=5,
        )

    def make_key_file(self, contents=FAKE_KEY_BYTES, *, mode=0o600):
        directory = self.enterContext(tempfile.TemporaryDirectory())
        key_file = Path(directory) / "buildbuddy.key"
        key_file.write_bytes(contents)
        key_file.chmod(mode)
        return key_file

    def test_get_returns_credentials_without_logging(self):
        for uri in (
            "grpcs://remote.buildbuddy.io/build.bazel.remote.execution.v2.Capabilities/GetCapabilities",
            "https://remote.buildbuddy.io:443/cache",
        ):
            with self.subTest(uri=uri):
                result = self.run_helper(json.dumps({"uri": uri, "futureField": True}))
                self.assertEqual((result.returncode, result.stderr), (0, ""))
                self.assertEqual(
                    json.loads(result.stdout),
                    {"headers": {"x-buildbuddy-api-key": [FAKE_KEY]}},
                )

    def test_selected_tenant_returns_credentials_only_for_that_tenant(self):
        for host in ("remote.buildbuddy.io", "openai.buildbuddy.io"):
            for requested_host in ("remote.buildbuddy.io", "openai.buildbuddy.io"):
                with self.subTest(host=host, requested_host=requested_host):
                    result = self.run_helper(
                        json.dumps({"uri": f"https://{requested_host}:443/cache"}),
                        host=host,
                    )
                    if requested_host == host:
                        self.assertEqual((result.returncode, result.stderr), (0, ""))
                        self.assertEqual(
                            json.loads(result.stdout),
                            {"headers": {"x-buildbuddy-api-key": [FAKE_KEY]}},
                        )
                    else:
                        self.assertEqual(
                            (result.returncode, result.stdout, result.stderr),
                            (
                                1,
                                "",
                                "Credentials require HTTPS or grpcs on the selected BuildBuddy host and port 443.\n",
                            ),
                        )

    def test_invalid_selected_host_is_not_echoed(self):
        for host in (
            "",
            FAKE_KEY,
            "https://openai.buildbuddy.io",
            "remote.buildbuddy.io:443",
        ):
            with self.subTest():
                result = self.run_helper(host=host)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "UV_BAZEL_CREDENTIAL_HOST must be remote.buildbuddy.io or openai.buildbuddy.io.\n",
                    ),
                )

    def test_missing_key_fails(self):
        for key in (None, ""):
            with self.subTest(key=key):
                result = self.run_helper(key=key)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "Set UV_BAZEL_KEY_FILE or BUILDBUDDY_API_KEY before enabling BuildBuddy credentials.\n",
                    ),
                )

    def test_invalid_header_value_is_not_logged(self):
        for key in (
            f"{FAKE_KEY}\n",
            f"{FAKE_KEY}\N{SNOWMAN}",
            f"{FAKE_KEY} ",
            f"{FAKE_KEY}\x7f",
        ):
            with self.subTest():
                result = self.run_helper(key=key)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "BuildBuddy API keys must contain only visible ASCII characters.\n",
                    ),
                )

    def test_unexpected_action_is_not_echoed(self):
        for arguments in ((), (FAKE_KEY,), ("get", FAKE_KEY)):
            with self.subTest():
                result = self.run_helper(arguments=arguments)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (1, "", "Usage: buildbuddy_credentials.py get\n"),
                )

    def test_other_hosts_and_insecure_schemes_are_rejected(self):
        for uri in (
            "grpcs://tenant.example.com",
            "grpcs://openai.buildbuddy.io",
            "https://remote.buildbuddy.io.example.com",
            "https://remote.buildbuddy.io@other.example.com",
            "https://user@remote.buildbuddy.io",
            f"https://:{FAKE_KEY}@remote.buildbuddy.io",
            "https://@remote.buildbuddy.io",
            "http://remote.buildbuddy.io",
            "grpc://remote.buildbuddy.io",
            "https://remote.buildbuddy.io:80",
            "grpcs://remote.buildbuddy.io:8443",
        ):
            with self.subTest(uri=uri):
                result = self.run_helper(json.dumps({"uri": uri}))
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "Credentials require HTTPS or grpcs on the selected BuildBuddy host and port 443.\n",
                    ),
                )

    def test_malformed_input_is_not_echoed(self):
        for request in (
            FAKE_KEY,
            "[]",
            "{}",
            '{"uri": 123}',
            '{"uri": "https://["}',
            '{"uri": "https://remote.buildbuddy.io:invalid"}',
            '{"uri": "https://remote.buildbuddy.io:65536"}',
        ):
            with self.subTest():
                result = self.run_helper(request)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "Expected a JSON object containing a valid string 'uri'.\n",
                    ),
                )

    def test_key_file_takes_precedence_and_accepts_one_final_newline(self):
        for ending in (b"", b"\n", b"\r\n"):
            with self.subTest(ending=ending):
                key_file = self.make_key_file(FAKE_KEY_BYTES + ending)
                result = self.run_helper(
                    key="unused-fake-environment-key", key_file=key_file
                )
                self.assertEqual((result.returncode, result.stderr), (0, ""))
                self.assertEqual(
                    json.loads(result.stdout),
                    {"headers": {"x-buildbuddy-api-key": [FAKE_KEY]}},
                )

    def test_key_file_does_not_require_an_environment_key(self):
        result = self.run_helper(key=None, key_file=self.make_key_file())
        self.assertEqual((result.returncode, result.stderr), (0, ""))
        self.assertEqual(
            json.loads(result.stdout),
            {"headers": {"x-buildbuddy-api-key": [FAKE_KEY]}},
        )

    def test_invalid_key_file_contents_are_not_logged(self):
        for contents in (
            f"{FAKE_KEY}\nsecond-line".encode("ascii"),
            f"{FAKE_KEY}\n\n".encode("ascii"),
            f"{FAKE_KEY}\r".encode("ascii"),
            f"{FAKE_KEY} ".encode("ascii"),
            f"{FAKE_KEY}\N{SNOWMAN}".encode(),
            b"\xff",
        ):
            with self.subTest():
                result = self.run_helper(key_file=self.make_key_file(contents))
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "BuildBuddy API keys must contain only visible ASCII characters.\n",
                    ),
                )

    def test_empty_key_file_does_not_fall_back_to_environment(self):
        for contents in (b"", b"\n", b"\r\n"):
            with self.subTest(contents=contents):
                result = self.run_helper(key_file=self.make_key_file(contents))
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "Set UV_BAZEL_KEY_FILE or BUILDBUDDY_API_KEY before enabling BuildBuddy credentials.\n",
                    ),
                )

    def test_oversized_key_file_is_not_logged(self):
        result = self.run_helper(key_file=self.make_key_file(b"x" * 4097))
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (1, "", "UV_BAZEL_KEY_FILE exceeds the 4096-byte limit.\n"),
        )

    def test_missing_or_non_file_key_path_does_not_fall_back_to_environment(self):
        key_file = self.make_key_file()
        for path in ("", key_file.with_name(FAKE_KEY), key_file.parent):
            with self.subTest():
                result = self.run_helper(key_file=path)
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user.\n",
                    ),
                )

    @unittest.skipUnless(os.name == "posix", "Unix file permissions")
    def test_key_file_group_or_other_permissions_are_rejected(self):
        for mode in (0o640, 0o602, 0o601):
            with self.subTest(mode=mode):
                result = self.run_helper(key_file=self.make_key_file(mode=mode))
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr),
                    (
                        1,
                        "",
                        "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user.\n",
                    ),
                )

    @unittest.skipUnless(os.name == "posix", "Unix file ownership")
    def test_key_file_must_be_owned_by_current_user(self):
        key_file = self.make_key_file()
        namespace = runpy.run_path(str(HELPER))
        with mock.patch("os.getuid", return_value=os.getuid() + 1):
            with self.assertRaises(SystemExit) as error:
                namespace["read_key_file"](str(key_file))
        self.assertEqual(
            str(error.exception),
            "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user.",
        )

    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "No-follow file opening")
    def test_symlink_key_file_is_rejected(self):
        key_file = self.make_key_file()
        symlink = key_file.with_name("symlink.key")
        symlink.symlink_to(key_file)
        result = self.run_helper(key_file=symlink)
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (
                1,
                "",
                "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user.\n",
            ),
        )

    @unittest.skipUnless(
        hasattr(os, "mkfifo") and hasattr(os, "O_NONBLOCK"), "Unix named pipe"
    )
    def test_pipe_key_file_is_rejected_without_blocking(self):
        key_file = self.make_key_file()
        pipe = key_file.with_name("pipe.key")
        os.mkfifo(pipe, 0o600)
        result = self.run_helper(key_file=pipe)
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (
                1,
                "",
                "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user.\n",
            ),
        )


if __name__ == "__main__":
    unittest.main()
