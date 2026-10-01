# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for applying snapshots downloaded from CI."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

SCRIPT = Path(__file__).with_name("apply-ci-snapshots.sh")
STUB = r"""
import json
import os
import sys
import tempfile
from pathlib import Path

root = Path(os.environ["SNAPSHOT_TEST_STATE"])
config = json.loads((root / "config.json").read_text())
command = Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps([command, *args]) + "\n")

if command == "mktemp" and args == ["-d"]:
    print(tempfile.mkdtemp(prefix="snapshots.", dir=os.environ["TMPDIR"]))
elif command == "git" and args == ["branch", "--show-current"]:
    print("fixture-branch")
elif command == "gh" and args[:2] == ["pr", "view"]:
    assert args == ["pr", "view", "fixture-branch", "--repo", "astral-sh/uv", "--json", "number", "--jq", ".number"]
    print("123")
elif command == "gh" and args[:2] == ["run", "list"]:
    assert args == ["run", "list", "--repo", "astral-sh/uv", "--workflow", "ci.yml", "--branch", "fixture-branch", "--limit", "1", "--json", "databaseId", "--jq", ".[0].databaseId"]
    print("456")
elif command == "gh" and args[:1] == ["api"]:
    assert args == ["api", "--paginate", "repos/astral-sh/uv/actions/runs/456/artifacts?per_page=100", "--jq", '.artifacts[] | select(.name | startswith("pending-snapshots-")) | .name']
    if config["case"] == "listing-failure":
        print("artifact listing failed", file=sys.stderr)
        raise SystemExit(23)
    if config["case"] != "no-artifacts":
        print("pending-snapshots-linux")
        print("pending-snapshots-windows")
elif command == "gh" and args[:2] == ["run", "download"]:
    assert args[:7] == ["run", "download", "456", "--repo", "astral-sh/uv", "--pattern", "pending-snapshots-*"]
    assert args[7] == "--dir" and len(args) == 9
    destination = Path(args[8])
    if config["case"] in {"no-artifacts", "listing-failure"}:
        print("no artifact matches any of the patterns provided", file=sys.stderr)
        raise SystemExit(1)
    if config["case"] == "download-failure":
        print("artifact download failed", file=sys.stderr)
        raise SystemExit(24)
    linux = destination / "pending-snapshots-linux"
    linux.mkdir()
    (linux / "first.snap.new").write_text("linux snapshot\n")
    if config["case"] == "partial-download-failure":
        print("second artifact download failed", file=sys.stderr)
        raise SystemExit(25)
    windows = destination / "pending-snapshots-windows"
    windows.mkdir()
    (windows / "second.pending-snap").write_text("windows snapshot\n")
elif command == "cargo":
    assert args in (["insta", "accept", "--workspace"], ["insta", "review", "--workspace"])
    pending = Path(os.environ["INSTA_PENDING_DIR"])
    (root / "applied.json").write_text(json.dumps({
        "action": args[1],
        "files": {str(path.relative_to(pending)): path.read_text() for path in sorted(pending.rglob("*")) if path.is_file()},
    }))
else:
    raise AssertionError(f"Unexpected command: {command} {args}")
"""


class ApplySnapshotsTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.tmp = self.root / "tmp"
        self.tmp.mkdir()
        for name in ("gh", "git", "cargo", "cargo-insta", "mktemp"):
            executable = self.bin / name
            executable.write_text(f"#!{sys.executable}\n{STUB}")
            executable.chmod(0o755)
        self.environment = os.environ | {
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "TMPDIR": str(self.tmp),
            "SNAPSHOT_TEST_STATE": str(self.root),
        }

    def run_script(self, case, *args):
        (self.root / "config.json").write_text(json.dumps({"case": case}))
        result = subprocess.run(
            ["bash", str(SCRIPT), *args],
            cwd=self.root,
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(list(self.tmp.iterdir()), [])
        return result

    def calls(self):
        return [
            json.loads(line)
            for line in (self.root / "calls.jsonl").read_text().splitlines()
        ]

    def assert_not_applied(self):
        self.assertFalse((self.root / "applied.json").exists())
        self.assertNotIn("cargo", [call[0] for call in self.calls()])

    def assert_applied(self, action):
        self.assertEqual(
            json.loads((self.root / "applied.json").read_text()),
            {
                "action": action,
                "files": {
                    "first.snap.new": "linux snapshot\n",
                    "second.pending-snap": "windows snapshot\n",
                },
            },
        )

    def test_no_artifacts_is_success(self):
        result = self.run_script("no-artifacts", "456")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No pending snapshot artifacts found in run 456.", result.stdout)
        self.assert_not_applied()

    def test_listing_failure_is_reported(self):
        result = self.run_script("listing-failure", "456")
        self.assertEqual(result.returncode, 23)
        self.assertIn("artifact listing failed", result.stderr)
        self.assertNotIn("No pending snapshot artifacts", result.stdout)
        self.assert_not_applied()

    def test_download_failure_is_reported(self):
        result = self.run_script("download-failure", "456")
        self.assertEqual(result.returncode, 24)
        self.assertIn("artifact download failed", result.stderr)
        self.assertNotIn("No pending snapshot artifacts", result.stdout)
        self.assert_not_applied()

    def test_partial_download_is_not_applied(self):
        result = self.run_script("partial-download-failure", "456")
        self.assert_not_applied()
        self.assertEqual(result.returncode, 25)
        self.assertIn("second artifact download failed", result.stderr)

    def test_complete_download_applies_all_artifacts(self):
        result = self.run_script("complete", "456")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_applied("accept")

    def test_review_action_is_forwarded(self):
        result = self.run_script("complete", "456", "review")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_applied("review")

    def test_run_id_is_detected_for_current_branch(self):
        result = self.run_script("complete", "review")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_applied("review")
        self.assertIn(["git", "branch", "--show-current"], self.calls())


if __name__ == "__main__":
    unittest.main()
