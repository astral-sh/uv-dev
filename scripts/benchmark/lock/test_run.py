"""Check the benchmark launcher's host directories and receipt boundary."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from run import run_receipt

LAUNCHER = Path(__file__).with_name("run.py")


class LauncherTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)

    def assert_overlap_rejected(self, cache, output):
        command = [
            sys.executable,
            str(LAUNCHER),
            "black",
            "--uv",
            "/bin/false",
            "--cache",
            str(cache),
            "--output",
            str(output),
        ]
        result = subprocess.run(command, text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn(
            "cache and run output directories must be disjoint", result.stderr
        )
        self.assertFalse(cache.exists())
        self.assertFalse(output.exists())

    def test_same_directory(self):
        path = self.directory / "new"
        self.assert_overlap_rejected(path, path)

    def test_cache_inside_output(self):
        output = self.directory / "new"
        self.assert_overlap_rejected(output / "cache", output)

    def test_output_inside_cache_via_alias(self):
        alias = self.directory / "alias"
        alias.symlink_to(self.directory, target_is_directory=True)
        self.assert_overlap_rejected(self.directory / "new", alias / "new" / "run")

    def test_receipt_replaced(self):
        path = self.directory / "run.json"
        receipt = {"complete": False}
        with (
            self.assertRaisesRegex(RuntimeError, "receipt changed"),
            run_receipt(self.directory, receipt),
        ):
            self.assertFalse(json.loads(path.read_text())["complete"])
            path.unlink()
            path.write_text("a different file\n")
            receipt["complete"] = True
        self.assertEqual(path.read_text(), "a different file\n")

    @unittest.skipUnless(
        os.environ.get("UV_BENCHMARK_DOCKER_TEST") == "1",
        "requires a non-root Docker client and the pinned Python 3.12 image",
    )
    def test_container_mounts(self):
        probe = self.directory / "probe"
        probe.write_text(
            "#!/bin/sh\nset -eu\n"
            "test ! -e /out/run.json\n"
            "test ! -e /cache/run.json\n"
            "touch /cache/cache-canary /out/result-canary\n"
        )
        probe.chmod(0o755)
        output = self.directory / "run"
        cache = self.directory / "cache"
        subprocess.run(
            [
                sys.executable,
                LAUNCHER,
                "black",
                "--uv",
                probe,
                "--cache",
                cache,
                "--output",
                output,
            ],
            check=True,
        )
        self.assertTrue((cache / "cache-canary").is_file())
        self.assertTrue((output / "results" / "result-canary").is_file())
        self.assertTrue(json.loads((output / "run.json").read_text())["complete"])
