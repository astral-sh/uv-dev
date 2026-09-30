"""Check complete registry and OSV retrieval, including pagination and retries."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent


class ToolAuditOracleTests(unittest.TestCase):
    def test_registry_and_osv_requests(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            root = Path(temporary)
            prefix = [
                "uv",
                "run",
                "--no-project",
                "--offline",
                "--python",
                sys.executable,
                "python",
                "-S",
            ]
            subprocess.run(
                [
                    *prefix,
                    str(HERE / "make_tool_audit_fixtures.py"),
                    "--directory",
                    str(root),
                    "--packages",
                    "3",
                    "--source",
                    "registry",
                ],
                capture_output=True,
                check=True,
            )
            setup = json.loads((root / "tool-audit-setup.json").read_text())
            self.assertEqual(
                [command[5] for command in setup],
                [
                    "{base}/audit-index-0",
                    "{base}/audit-index-1",
                    "{base}/audit-index-0",
                ],
            )
            profiles = json.loads((root / "tool-audit-profiles.json").read_text())
            profiles["retry"] = dict(
                profiles["fast"],
                osv_failures={"uv-bench-audit-tool-000:0": {"status": 503, "count": 1}},
                path_failures={
                    "/audit-index-0/uv-bench-audit-tool-000/": {
                        "status": 503,
                        "count": 1,
                    }
                },
            )
            (root / "profiles.json").write_text(json.dumps(profiles))
            for route, profile, cached, count, waves in [
                ("plain", "fast", False, 4, 2),
                ("paginated", "fast", False, 5, 2),
                ("plain", "retry", False, 6, 2),
                ("plain", "fast", True, 1, 1),
            ]:
                with self.subTest(route=route, profile=profile, cached=cached):
                    output = root / "result.json"
                    subprocess.run(
                        [
                            *prefix,
                            str(HERE / "tool_audit_oracle.py"),
                            "--manifest",
                            str(root / f"tool-audit-{route}-fixtures.json"),
                            "--directory",
                            str(root),
                            "--profiles",
                            str(root / "profiles.json"),
                            "--profile",
                            profile,
                            "--packages",
                            "3",
                            "--concurrency",
                            "2",
                            "--output",
                            str(output),
                            *(["--cached-registry"] if cached else []),
                        ],
                        capture_output=True,
                        check=True,
                    )
                    result = json.loads(output.read_text())
                    self.assertEqual(result["requests"], count)
                    self.assertEqual(result["required_waves"], waves)
                    self.assertLessEqual(
                        result["required_bytes"], result["actual_bytes"]
                    )
                    self.assertEqual(
                        sum(
                            event["path"] != "/v1/querybatch" and event["status"] == 200
                            for event in result["events"]
                        ),
                        0 if cached else 3,
                    )
                    self.assertLessEqual(result["max_active"], 4)


if __name__ == "__main__":
    unittest.main()
