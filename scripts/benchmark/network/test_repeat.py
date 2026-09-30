"""Check that larger studies retain the pilot's inputs and reject drift."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "network_repeat", Path(__file__).with_name("repeat.py")
)
assert spec is not None and spec.loader is not None
repeat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repeat)


class RepeatTests(unittest.TestCase):
    def setUp(self) -> None:
        scratch = Path.home() / "code" / "tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=scratch)
        self.root = Path(self.temporary.name)
        self.manifest = self.root / "manifest.json"
        self.manifest.write_text("[]\n")
        pairs = [
            {"parent": {"seconds": 1.0}, "head": {"seconds": 0.8}},
            {"parent": {"seconds": 1.1}, "head": {"seconds": 0.9}},
        ]
        self.pilot = {
            "parent_sha": "1" * 40,
            "head_sha": "2" * 40,
            "binaries": {},
            "manifest_sha256": repeat.bench.digest(self.manifest),
            "pairs": pairs,
            "summary": repeat.bench.summary(pairs),
            "profile": {"latency_ms": 150, "bytes_per_second": 1250000},
            "netem": {},
            "command": ["pip", "compile", "{work}/requirements.in"],
            "python_request": sys.executable,
            "python_executable_sha256": repeat.bench.digest(Path(sys.executable)),
            "requirements": ["example==1.0"],
            "cache_mode": "refresh",
            "lower_bound": {
                "required_bytes": 1234,
                "required_waves": 2,
                "required_latency_ms": 300,
                "seconds": 0.3,
            },
        }
        for side in ("parent", "head"):
            path = self.root / side
            path.write_bytes(side.encode())
            self.pilot["binaries"][side] = {
                "path": str(path),
                "sha256": repeat.bench.digest(path),
                "version": f"uv test ({self.pilot[f'{side}_sha'][:9]})",
            }
        self.args = argparse.Namespace(
            manifest=self.manifest,
            directory=self.root,
            work_dir=self.root / "trials",
            output=self.root / "result.json",
            pairs=30,
            warmups=3,
            timeout=300,
            uv=None,
            git_root=None,
            tls_certificate=None,
            tls_key=None,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def checked_inputs(self) -> float:
        versions = {
            binary["path"]: binary["version"]
            for binary in self.pilot["binaries"].values()
        }
        with (
            patch.object(repeat.bench, "netem_profile", return_value={}),
            patch.object(
                repeat.subprocess,
                "check_output",
                side_effect=lambda args, **_: versions[str(args[0])],
            ),
        ):
            return repeat.check_inputs(self.pilot, self.args)

    def test_inputs_and_legacy_timeout(self) -> None:
        self.assertEqual(self.checked_inputs(), 300)
        self.args.timeout = None
        with self.assertRaisesRegex(ValueError, "Pass --timeout"):
            self.checked_inputs()
        self.pilot["timeout_seconds"] = 600
        self.assertEqual(self.checked_inputs(), 600)
        self.args.timeout = 300
        with self.assertRaisesRegex(ValueError, "Timeout differs"):
            self.checked_inputs()

    def test_rejects_changed_binary(self) -> None:
        Path(self.pilot["binaries"]["head"]["path"]).write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "head binary hash differs"):
            self.checked_inputs()

    def test_rejects_changed_network_profile(self) -> None:
        self.pilot["netem"] = {"rtt_ms": 200}
        with self.assertRaisesRegex(ValueError, "Kernel network profile differs"):
            self.checked_inputs()

    def test_command_retains_workload_settings(self) -> None:
        self.pilot.update(
            templates={
                "uv.toml": "offline = false\n",
                "pyproject.toml": "[project]\n",
                "uv.lock": "version = 1\n",
                "pylock.toml": 'lock-version = "1.0"\n',
            },
            setup_commands=[["venv", "{work}/venv"]],
            environment_overrides={"UV_CONCURRENT_DOWNLOADS": "2"},
            verify_tree="{work}/site",
            normalize_tree_file=["**/_sysconfigdata_*.py"],
            normalize_tree_symlink=["cpython-3.12-linux-x86_64-gnu"],
            verify_file=["{work}/uv.lock"],
            compare_stderr=True,
        )
        with patch.object(repeat.shutil, "which", return_value="/path/to/uv"):
            command = repeat.command(self.pilot, self.args, self.root, 300)
        separator = command.index("--")
        self.assertEqual(command[separator + 1 :], self.pilot["command"])

        def value(flag: str) -> str:
            return command[command.index(flag) + 1]

        self.assertEqual(value("--cache-mode"), "refresh")
        self.assertEqual(value("--requirement"), "example==1.0")
        self.assertEqual(value("--env"), "UV_CONCURRENT_DOWNLOADS=2")
        self.assertEqual(value("--verify-tree"), "{work}/site")
        self.assertEqual(value("--normalize-tree-file"), "**/_sysconfigdata_*.py")
        self.assertEqual(
            value("--normalize-tree-symlink"), "cpython-3.12-linux-x86_64-gnu"
        )
        self.assertEqual(value("--verify-file"), "{work}/uv.lock")
        self.assertIn("--compare-stderr", command)
        self.assertEqual(
            json.loads(Path(value("--profiles")).read_text()),
            {"repeat": self.pilot["profile"]},
        )
        self.assertEqual(
            Path(value("--config-template")).read_text(), "offline = false\n"
        )
        self.assertEqual(Path(value("--project-template")).read_text(), "[project]\n")
        self.assertEqual(Path(value("--lock-template")).read_text(), "version = 1\n")
        self.assertEqual(
            Path(value("--pylock-template")).read_text(), 'lock-version = "1.0"\n'
        )
        self.assertEqual(
            json.loads(Path(value("--setup-commands")).read_text()),
            self.pilot["setup_commands"],
        )

    def test_repeat_comparison_accepts_legacy_defaults(self) -> None:
        repeated = copy.deepcopy(self.pilot)
        repeated.update(
            templates={},
            setup_commands=[],
            environment_overrides={},
            verify_tree=None,
            normalize_tree_file=[],
            normalize_tree_symlink=[],
            verify_file=[],
            compare_stderr=False,
            http2_proxy=None,
            git_repositories=None,
            git_version=None,
        )
        repeated["lower_bound"]["required_wait_ms"] = 0
        repeat.check_repeat(self.pilot, repeated)
        for key, value in (
            ("command", ["pip", "sync"]),
            ("environment_overrides", {"UV_CONCURRENT_DOWNLOADS": "1"}),
        ):
            changed = copy.deepcopy(repeated)
            changed[key] = value
            with self.assertRaisesRegex(ValueError, f"Repeated {key} differs"):
                repeat.check_repeat(self.pilot, changed)
        repeated["lower_bound"]["required_bytes"] += 1
        with self.assertRaisesRegex(
            ValueError, "Repeated bound required_bytes differs"
        ):
            repeat.check_repeat(self.pilot, repeated)

    def test_explicit_uv_does_not_require_path_lookup(self) -> None:
        self.args.uv = self.root / "uv"
        with patch.object(repeat.shutil, "which", return_value=None) as lookup:
            command = repeat.command(self.pilot, self.args, self.root, 300)
        self.assertEqual(command[0], str(self.args.uv.resolve()))
        lookup.assert_not_called()


if __name__ == "__main__":
    unittest.main()
