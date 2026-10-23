"""Check that the latest-version oracle fetches all pages with bounded connections."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", HERE / "make_scheduling_fixtures.py"
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


class LatestIndexOracleTests(unittest.TestCase):
    def test_shared_connections_and_complete_pages(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home() / "code" / "tmp") as temporary:
            root = Path(temporary)
            manifest = [
                scheduling.wheel(
                    root, f"tool-{number}", version, None, console_script=True
                )
                for number in range(3)
                for version in (1, 2)
            ]
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest))
            profiles = root / "profiles.json"
            profiles.write_text(json.dumps({"fast": {"latency_ms": 0}}))
            output = root / "result.json"
            subprocess.run(
                [
                    "uv",
                    "run",
                    "--no-project",
                    "--offline",
                    "--python",
                    sys.executable,
                    "python",
                    "-S",
                    str(HERE / "latest_index_oracle.py"),
                    "--manifest",
                    str(manifest_path),
                    "--directory",
                    str(root),
                    "--profiles",
                    str(profiles),
                    "--profile",
                    "fast",
                    "--concurrency",
                    "2",
                    "--output",
                    str(output),
                ],
                capture_output=True,
                check=True,
            )
            result = json.loads(output.read_text())
            self.assertEqual(result["packages"], 3)
            self.assertEqual(result["requests"], 3)
            self.assertEqual(result["origin_connections"], 2)
            self.assertEqual(result["required_waves"], 2)
            self.assertEqual(result["required_bytes"], result["actual_bytes"])
            with zipfile.ZipFile(root / manifest[0]["filename"]) as wheel:
                self.assertEqual(
                    wheel.read("tool_0-1.0.dist-info/entry_points.txt"),
                    b"[console_scripts]\ntool-0 = tool_0.cli:main\n",
                )
                self.assertIn("tool_0/cli.py", wheel.namelist())


if __name__ == "__main__":
    unittest.main()
