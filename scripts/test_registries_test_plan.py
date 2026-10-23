# /// script
# requires-python = ">=3.12"
# dependencies = ["colorama>=0.4.6"]
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///

"""Tests for the registry installation test plans."""

import importlib.util
import sys
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "registries_test", Path(__file__).with_name("registries-test.py")
)
if spec is None or spec.loader is None:
    raise RuntimeError("Could not load registries-test.py")
registries_test = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = registries_test
spec.loader.exec_module(registries_test)
DEFAULT_PKG_NAME = registries_test.DEFAULT_PKG_NAME
plan_test = registries_test.plan_test


class PlanTests(unittest.TestCase):
    def test_environment_credentials(self):
        uv = Path("/test/uv")
        env = {
            "UV_TEST_EXAMPLE_TOKEN": "test-token",
            "UV_TEST_EXAMPLE_USERNAME": "test-user",
            "UV_TEST_EXAMPLE_PKG": "test-package",
            "UV_TEST_EXAMPLE_REQUIRE_METADATA_RANGE_REQUESTS": "TRUE",
            "UV_LOCKED": "1",
            "UNCHANGED": "value",
        }
        plan = plan_test(
            env,
            uv,
            "example",
            "https://example.com/simple/",
            verbosity=2,
            timeout=17,
            requires_python="3.13",
            auth_method="env",
        )

        self.assertEqual(
            plan.command("/test/project"),
            [
                uv,
                "add",
                "test-package",
                "--directory",
                "/test/project",
                "--no-cache",
                "-vv",
            ],
        )
        self.assertEqual(
            plan.auth_env,
            {
                "UV_INDEX_EXAMPLE_USERNAME": "test-user",
                "UV_INDEX_EXAMPLE_PASSWORD": "test-token",
            },
        )
        self.assertIsNone(plan.auth_command)
        self.assertEqual(
            plan.command_env(),
            {key: value for key, value in env.items() if key != "UV_LOCKED"}
            | plan.auth_env
            | {"UV_REQUIRE_METADATA_RANGE_REQUESTS": "true"},
        )
        self.assertEqual(plan.timeout, 17)
        self.assertEqual(plan.requires_python, "3.13")
        self.assertEqual(env["UV_LOCKED"], "1")
        self.assertNotIn("UV_INDEX_EXAMPLE_USERNAME", env)

    def test_text_store_credentials(self):
        uv = Path("/test/uv")
        env = {"UV_TEST_EXAMPLE_TOKEN": "test-token", "UV_LOCKED": "1"}
        plan = plan_test(
            env,
            uv,
            "example",
            "https://example.com/simple/",
            verbosity=0,
            timeout=30,
            requires_python="3.12",
            auth_method="text-store",
        )

        self.assertEqual(
            plan.auth_command,
            [
                uv,
                "auth",
                "login",
                "https://example.com/simple/",
                "--username",
                "__token__",
                "--password",
                "test-token",
            ],
        )
        self.assertEqual(plan.auth_env, {})
        self.assertEqual(plan.extra_args, [])
        self.assertEqual(plan.configuration.package, DEFAULT_PKG_NAME)
        self.assertEqual(plan.command_env(), {"UV_TEST_EXAMPLE_TOKEN": "test-token"})
        self.assertIs(plan.env, env)

    def test_public_registry(self):
        env = {
            "UV_TEST_EXAMPLE_PUBLIC": "TRUE",
            "UV_REQUIRE_METADATA_RANGE_REQUESTS": "false",
        }
        plan = plan_test(
            env,
            Path("/test/uv"),
            "example",
            "https://example.com/simple/",
            verbosity=0,
            timeout=30,
            requires_python="3.12",
            auth_method="env",
        )

        self.assertTrue(plan.configuration.public)
        self.assertIsNone(plan.configuration.token)
        self.assertIsNone(plan.auth_command)
        self.assertEqual(plan.auth_env, {})
        self.assertEqual(plan.command_env(), env)

    def test_credentials_are_omitted_from_repr(self):
        for auth_method in ("env", "text-store"):
            with self.subTest(auth_method=auth_method):
                plan = plan_test(
                    {
                        "UV_TEST_EXAMPLE_TOKEN": "test-secret-token",
                        "OTHER_SECRET": "test-environment-secret",
                    },
                    Path("/test/uv"),
                    "example",
                    "https://example.com/simple/",
                    verbosity=0,
                    timeout=30,
                    requires_python="3.12",
                    auth_method=auth_method,
                )

                self.assertNotIn("test-secret-token", repr(plan.configuration))
                self.assertNotIn("test-secret-token", repr(plan))
                self.assertNotIn("test-environment-secret", repr(plan))


if __name__ == "__main__":
    unittest.main()
