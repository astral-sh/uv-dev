"""Check the single-fetch reference against a real local Git repository."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "git_oracle", Path(__file__).with_name("git_oracle.py")
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)
fixture_spec = importlib.util.spec_from_file_location(
    "git_fixtures", Path(__file__).with_name("make_git_fixtures.py")
)
assert fixture_spec is not None and fixture_spec.loader is not None
git_fixtures = importlib.util.module_from_spec(fixture_spec)
fixture_spec.loader.exec_module(git_fixtures)


class GitOracleTests(unittest.TestCase):
    def test_named_and_precise_cold_and_warm_fetches(self) -> None:
        scratch = Path.home() / "code" / "tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            root = Path(directory)
            descriptor = git_fixtures.make(root, 1)
            profiles = root / "profiles.json"
            profiles.write_text(json.dumps({"test": {"latency_ms": 1}}))
            output = root / "result.json"
            for precise in (False, True):
                for warm in (False, True):
                    arguments = [
                        "git_oracle.py",
                        "--directory",
                        str(root),
                        "--profiles",
                        str(profiles),
                        "--profile",
                        "test",
                        "--work-dir",
                        str(root / "trials"),
                        "--output",
                        str(output),
                    ]
                    if precise:
                        arguments += ["--revision", descriptor["commit"]]
                    if warm:
                        arguments += ["--warm"]
                    with (
                        self.subTest(precise=precise, warm=warm),
                        patch("sys.argv", arguments),
                        contextlib.redirect_stdout(io.StringIO()),
                    ):
                        oracle.main()
                        result = json.loads(output.read_text())
                        waves = (0 if precise else 1) if warm else 2
                        self.assertEqual(result["commit"], descriptor["commit"])
                        self.assertEqual(
                            result["revision"],
                            descriptor["commit"] if precise else None,
                        )
                        self.assertEqual(result["required_bytes"], 0)
                        self.assertEqual(result["required_waves"], waves)
                        self.assertEqual(
                            result["optimistic_network_floor_seconds"], waves / 1000
                        )
                        self.assertGreaterEqual(result["requests"], waves)
                        self.assertEqual(result["requests"], len(result["events"]))
                        self.assertEqual(
                            result["actual_bytes"],
                            sum(event["bytes"] for event in result["events"]),
                        )
                        self.assertIsNone(result["http2_proxy"])
                        self.assertIsNone(result["frontend_protocols"])

    def test_partial_http2_configuration_is_rejected(self) -> None:
        arguments = [
            "git_oracle.py",
            "--directory",
            ".",
            "--profiles",
            "profiles.json",
            "--profile",
            "test",
            "--work-dir",
            ".",
            "--output",
            "result.json",
        ]
        for option in ("--http2-proxy", "--tls-certificate", "--tls-key"):
            with (
                self.subTest(option=option),
                patch("sys.argv", [*arguments, option, "missing"]),
                contextlib.redirect_stderr(io.StringIO()) as error,
                self.assertRaises(SystemExit) as raised,
            ):
                oracle.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertIn("HTTP/2 requires", error.getvalue())


if __name__ == "__main__":
    unittest.main()
