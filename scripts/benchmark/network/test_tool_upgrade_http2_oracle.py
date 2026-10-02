"""Exercise full-wheel upgrade references through a real TLS/HTTP2 proxy."""

from __future__ import annotations

import collections
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location(
    "tool_upgrade_http2_oracle", HERE / "tool_upgrade_http2_oracle.py"
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)
DRIVER = [
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
CADDY = os.environ.get("UV_BENCH_CADDY") or shutil.which("caddy")
CURL = os.environ.get("UV_BENCH_CURL") or shutil.which("curl")
CERTIFICATE = os.environ.get("UV_BENCH_TLS_CERTIFICATE")
KEY = os.environ.get("UV_BENCH_TLS_KEY")


@unittest.skipUnless(
    all((CADDY, CURL, CERTIFICATE, KEY)),
    "set UV_BENCH_CADDY, UV_BENCH_TLS_CERTIFICATE, and UV_BENCH_TLS_KEY",
)
class ToolUpgradeHttp2OracleTests(unittest.TestCase):
    def command(self, directory: Path, *arguments: str) -> list[str]:
        return [
            *DRIVER,
            str(HERE / "tool_upgrade_http2_oracle.py"),
            "--directory",
            str(directory),
            "--curl",
            CURL,
            "--http2-proxy",
            CADDY,
            "--tls-certificate",
            CERTIFICATE,
            "--tls-key",
            KEY,
            "--work-dir",
            str(directory / "trials"),
            *arguments,
        ]

    def fixtures(self, directory: Path) -> None:
        subprocess.run(
            [
                *DRIVER,
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
        profile_path = directory / "tool-upgrade-profiles.json"
        profiles = json.loads(profile_path.read_text())
        profiles["bounded"] = {**profiles["fast"], "latency_ms": 80}
        profiles["transient"] = {
            **profiles["bounded"],
            "path_failures": {
                "/upgrade-index-0/uv-bench-upgrade-shared-000/": {
                    "status": 503,
                    "count": 1,
                }
            },
        }
        profile_path.write_text(json.dumps(profiles))

    def test_dependencies_limits_and_retries(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            self.fixtures(directory)
            profiles = json.loads(
                (directory / "tool-upgrade-profiles.json").read_text()
            )
            scenarios = json.loads(
                (directory / "tool-upgrade-descriptor.json").read_text()
            )["scenarios"]
            fixtures = oracle.bench.Fixtures(
                directory / "tool-upgrade-fixtures.json", directory, True
            )
            for profile, scenario, concurrency, statuses in (
                ("bounded", "upgrade", 1, {200: 9}),
                ("bounded", "upgrade", 2, {200: 9}),
                ("transient", "upgrade", 2, {200: 9, 503: 1}),
                ("bounded", "current", 2, {200: 5}),
            ):
                with self.subTest(
                    profile=profile, scenario=scenario, concurrency=concurrency
                ):
                    output = directory / f"{profile}-{scenario}-{concurrency}.json"
                    completed = subprocess.run(
                        self.command(
                            directory,
                            "--profile",
                            profile,
                            "--scenario",
                            scenario,
                            "--concurrency",
                            str(concurrency),
                            "--output",
                            str(output),
                        ),
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(completed.returncode, 0, completed.stderr)
                    result = json.loads(output.read_text())
                    tasks = oracle.upgrade.requests(
                        fixtures, profiles[profile], scenarios[scenario]
                    )
                    bound = oracle.upgrade.bounds(
                        tasks, profiles[profile], concurrency, netem={}
                    )
                    self.assertTrue(
                        all(result[key] == value for key, value in bound.items())
                    )
                    self.assertEqual(
                        collections.Counter(
                            event["status"] for event in result["events"]
                        ),
                        statuses,
                    )
                    self.assertEqual(result["requests"], sum(statuses.values()))
                    self.assertEqual(
                        result["frontend_protocols"],
                        {"HTTP/2.0": sum(statuses.values())},
                    )
                    self.assertEqual(len(result["transfers"]), len(tasks))
                    self.assertEqual(
                        {
                            str(transfer["http_version"])
                            for transfer in result["transfers"]
                        },
                        {"2"},
                    )
                    self.assertEqual(
                        oracle.bench.maximum_active(result["events"]), concurrency
                    )
                    successful = {
                        event["path"]: event
                        for event in result["events"]
                        if event["status"] == 200
                    }
                    self.assertEqual(
                        {path: event["bytes"] for path, event in successful.items()},
                        {task["path"]: len(task["expected"]) for task in tasks},
                    )
                    for task in tasks:
                        for parent in task["parents"]:
                            self.assertGreaterEqual(
                                successful[task["path"]]["start"],
                                successful[tasks[parent]["path"]]["end"],
                            )
                    self.assertEqual(
                        len(result["transfer_waves"]), 2 if scenario == "upgrade" else 1
                    )
                    self.assertGreaterEqual(
                        result["frontend_connections"], len(result["transfer_waves"])
                    )

    def test_no_network_and_fixture_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            self.fixtures(directory)
            for scenario in ("pinned", "local", "empty"):
                output = directory / f"{scenario}.json"
                subprocess.run(
                    self.command(
                        directory,
                        "--profile",
                        "bounded",
                        "--scenario",
                        scenario,
                        "--output",
                        str(output),
                    ),
                    capture_output=True,
                    check=True,
                )
                result = json.loads(output.read_text())
                self.assertEqual(
                    (
                        result["requests"],
                        result["actual_bytes"],
                        result["required_bytes"],
                        result["optimistic_network_floor_seconds"],
                    ),
                    (0, 0, 0, 0),
                )
                self.assertEqual(result["frontend_protocols"], {})
                self.assertEqual(result["transfer_waves"], [])
            filename = "uv_bench_upgrade_tool_000-2.0-py3-none-any.whl"
            with (directory / filename).open("ab") as wheel:
                wheel.write(b"extra")
            failed = subprocess.run(
                self.command(
                    directory,
                    "--profile",
                    "bounded",
                    "--scenario",
                    "upgrade",
                    "--output",
                    str(directory / "invalid.json"),
                ),
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn(
                "Fixture hash mismatch: " + str(directory / filename), failed.stderr
            )


if __name__ == "__main__":
    unittest.main()
