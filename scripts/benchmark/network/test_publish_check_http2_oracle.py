"""Exercise the publishing reference against a real local TLS/HTTP2 proxy."""

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
    "publish_check_http2_oracle", HERE / "publish_check_http2_oracle.py"
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
class PublishCheckHttp2OracleTests(unittest.TestCase):
    def command(self, directory: Path, *arguments: str) -> list[str]:
        return [
            *DRIVER,
            str(HERE / "publish_check_http2_oracle.py"),
            "--manifest",
            str(directory / "publish-fixtures.json"),
            "--directory",
            str(directory),
            "--profiles",
            str(directory / "publish-profiles.json"),
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
                str(HERE / "make_publish_fixtures.py"),
                "--directory",
                str(directory),
                "--projects",
                "3",
                "--builds",
                "2",
            ],
            capture_output=True,
            check=True,
        )
        (directory / "publish-profiles.json").write_text(
            json.dumps(
                {
                    "bounded": {"latency_ms": 80},
                    "flaky": {
                        "latency_ms": 80,
                        "path_failures": {
                            "/simple/uv-bench-publish-000/": {"status": 503, "count": 1}
                        },
                    },
                }
            )
        )

    def test_limits_conditional_reads_and_retries(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            self.fixtures(directory)
            fixtures = oracle.bench.Fixtures(
                directory / "publish-fixtures.json", directory, True
            )
            required_bytes = sum(len(body) for body in fixtures.simple.values())
            for profile, route, concurrency, waves, latency, statuses in (
                ("bounded", "current", 1, 3, 240, {200: 3}),
                ("bounded", "current", 2, 2, 160, {200: 3}),
                ("bounded", "revalidate", 2, 2, 160, {304: 3}),
                ("flaky", "current", 2, 2, 160, {200: 3, 503: 1}),
            ):
                with self.subTest(
                    profile=profile, route=route, concurrency=concurrency
                ):
                    output = directory / f"{profile}-{route}-{concurrency}.json"
                    completed = subprocess.run(
                        self.command(
                            directory,
                            "--profile",
                            profile,
                            "--route",
                            route,
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
                    self.assertEqual(result["requests"], sum(statuses.values()))
                    self.assertEqual(
                        result["frontend_protocols"],
                        {"HTTP/2.0": sum(statuses.values())},
                    )
                    self.assertEqual(len(result["transfers"]), 3)
                    self.assertEqual(result["frontend_connections"], 1)
                    self.assertEqual(
                        {
                            str(transfer["http_version"])
                            for transfer in result["transfers"]
                        },
                        {"2"},
                    )
                    self.assertEqual(
                        collections.Counter(
                            event["status"] for event in result["events"]
                        ),
                        statuses,
                    )
                    self.assertEqual(
                        oracle.bench.maximum_active(result["events"]), concurrency
                    )
                    self.assertEqual(result["required_waves"], waves)
                    self.assertEqual(result["required_latency_ms"], latency)
                    self.assertEqual(
                        result["optimistic_network_floor_seconds"], latency / 1000
                    )
                    self.assertEqual(
                        result["required_bytes"],
                        required_bytes if route == "current" else 0,
                    )
                    if route == "revalidate":
                        self.assertEqual(result["actual_bytes"], 0)

    def test_selected_files_and_fixture_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            self.fixtures(directory)
            filename = "uv_bench_publish_002-1.0-2-py3-none-any.whl"
            output = directory / "selected.json"
            command = self.command(
                directory,
                "--profile",
                "bounded",
                "--filename",
                filename,
                "--output",
                str(output),
            )
            subprocess.run(command, capture_output=True, check=True)
            result = json.loads(output.read_text())
            self.assertEqual(result["filenames"], [filename])
            self.assertEqual(result["requests"], 1)
            self.assertEqual(result["frontend_protocols"], {"HTTP/2.0": 1})
            with (directory / filename).open("ab") as wheel:
                wheel.write(b"extra")
            failed = subprocess.run(
                command, capture_output=True, text=True, check=False
            )
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn(
                "Fixture hash mismatch: " + str(directory / filename), failed.stderr
            )


if __name__ == "__main__":
    unittest.main()
