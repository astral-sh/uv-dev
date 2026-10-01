"""Check the authentication reference and its two network bounds."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "authentication_oracle", Path(__file__).with_name("auth_challenge_oracle.py")
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class AuthenticationOracleTests(unittest.TestCase):
    def test_closes_challenge_and_reuses_discovered_credentials(self) -> None:
        scratch = Path.home() / "code" / "tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            root = Path(directory)
            filename = "example-1.0-py3-none-any.whl"
            wheel = root / filename
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr(
                    "example-1.0.dist-info/METADATA",
                    "Metadata-Version: 2.3\nName: example\nVersion: 1.0\n",
                )
            manifest = root / "manifest.json"
            manifest.write_text(
                json.dumps(
                    [
                        {
                            "filename": filename,
                            "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
                            "size": wheel.stat().st_size,
                        }
                    ]
                )
            )
            challenge = {
                "authorization": "Basic dXNlcjpwYXNzd29yZA==",
                "body_bytes": 65536,
            }
            profile = {
                "latency_ms": 1,
                "bytes_per_second": 1000000,
                "auth_challenges": {
                    "/simple/example/": challenge,
                    f"/files/{filename}.metadata": challenge,
                },
            }
            profiles = root / "profiles.json"
            profiles.write_text(json.dumps({"test": profile}))
            output = root / "result.json"
            with patch(
                "sys.argv",
                [
                    "auth_challenge_oracle.py",
                    "--manifest",
                    str(manifest),
                    "--directory",
                    str(root),
                    "--profiles",
                    str(profiles),
                    "--profile",
                    "test",
                    "--filename",
                    filename,
                    "--output",
                    str(output),
                ],
            ):
                oracle.main()
            result = json.loads(output.read_text())
            fixtures = oracle.bench.Fixtures(manifest, root, True)
            required_bytes = len(fixtures.simple["example"]) + len(
                fixtures.metadata[filename + ".metadata"]
            )
            self.assertEqual(result["required_bytes"], required_bytes)
            self.assertEqual(result["required_waves"], 3)
            self.assertEqual(result["required_latency_ms"], 3)
            self.assertEqual(result["challenge_paths"], ["/simple/example/"])
            self.assertEqual(result["requests"], 3)
            self.assertEqual(result["origin_connections"], 2)
            self.assertEqual(
                [event["authentication"] for event in result["events"]],
                ["challenge", "accepted", "accepted"],
            )
            self.assertEqual(result["optimistic_network_floor_seconds"], 0.003)
            self.assertEqual(result["preauthenticated_network_floor_seconds"], 0.002)
            with patch(
                "sys.argv",
                [
                    "auth_challenge_oracle.py",
                    "--manifest",
                    str(manifest),
                    "--directory",
                    str(root),
                    "--profiles",
                    str(profiles),
                    "--profile",
                    "test",
                    "--filename",
                    filename,
                    "--preauthenticated",
                    "--output",
                    str(output),
                ],
            ):
                oracle.main()
            result = json.loads(output.read_text())
            self.assertTrue(result["preauthenticated"])
            self.assertEqual(result["required_waves"], 2)
            self.assertEqual(result["required_latency_ms"], 2)
            self.assertEqual(result["challenge_paths"], [])
            self.assertEqual(result["requests"], 2)
            self.assertEqual(result["origin_connections"], 1)
            self.assertEqual(result["optimistic_network_floor_seconds"], 0.002)


if __name__ == "__main__":
    unittest.main()
