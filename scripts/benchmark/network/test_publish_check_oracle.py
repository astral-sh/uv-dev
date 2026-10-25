"""Check bounded repeat-publish references and deterministic fixture identities."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location(
    "publish_check_oracle", HERE / "publish_check_oracle.py"
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


class PublishCheckOracleTests(unittest.TestCase):
    def test_default_fixture_identity(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
            subprocess.run(
                [
                    *DRIVER,
                    str(HERE / "make_publish_fixtures.py"),
                    "--directory",
                    str(directory),
                ],
                capture_output=True,
                check=True,
            )
            rows = json.loads((directory / "publish-fixtures.json").read_text())
            identity = [
                {key: row[key] for key in ("filename", "sha256", "size")}
                for row in rows
            ]
            self.assertEqual(
                hashlib.sha256(
                    json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
                ).hexdigest(),
                "9f1d156e813f3fee43eca8caab7db648e4df280c8b5bece4b3ca979b5b2fd619",
            )

    def test_project_limits_and_conditional_requests(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            directory = Path(temporary)
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
            manifest = directory / "publish-fixtures.json"
            rows = json.loads(manifest.read_text())
            self.assertEqual(len(rows), 6)
            profiles = directory / "publish-profiles.json"
            profiles.write_text(json.dumps({"bounded": {"latency_ms": 40}}))
            fixtures = oracle.bench.Fixtures(manifest, directory, True)
            required_bytes = sum(len(body) for body in fixtures.simple.values())
            for route, concurrency, waves, latency in (
                ("current", 1, 3, 120),
                ("current", 2, 2, 80),
                ("revalidate", 2, 2, 80),
            ):
                with self.subTest(route=route, concurrency=concurrency):
                    output = directory / f"{route}-{concurrency}.json"
                    subprocess.run(
                        [
                            *DRIVER,
                            str(HERE / "publish_check_oracle.py"),
                            "--manifest",
                            str(manifest),
                            "--directory",
                            str(directory),
                            "--profiles",
                            str(profiles),
                            "--profile",
                            "bounded",
                            "--route",
                            route,
                            "--concurrency",
                            str(concurrency),
                            "--output",
                            str(output),
                        ],
                        capture_output=True,
                        check=True,
                    )
                    result = json.loads(output.read_text())
                    self.assertEqual(result["requests"], 3)
                    self.assertEqual(result["concurrency"], concurrency)
                    self.assertLessEqual(
                        oracle.bench.maximum_active(result["events"]), concurrency
                    )
                    self.assertEqual(result["required_waves"], waves)
                    self.assertEqual(result["required_latency_ms"], latency)
                    self.assertEqual(
                        result["optimistic_network_floor_seconds"], latency / 1000
                    )
                    expected_bytes = required_bytes if route == "current" else 0
                    self.assertEqual(result["required_bytes"], expected_bytes)
                    self.assertEqual(result["actual_bytes"], expected_bytes)
                    self.assertEqual(
                        {event["status"] for event in result["events"]},
                        {200 if route == "current" else 304},
                    )


if __name__ == "__main__":
    unittest.main()
