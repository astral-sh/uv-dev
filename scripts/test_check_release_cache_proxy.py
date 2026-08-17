"""Integration tests for release cache-proxy placement."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check_release_cache_proxy.py")
RELEASE = ".github/workflows/release.yml"
BUILD = ".github/workflows/build-release-binaries.yml"
ACTION = ".github/actions/disable-github-caches/action.yml"
USES = "astral-sh/uv-dev/.github/actions/disable-github-caches@c3892c0a9adbb81c11bbda1eb62e020455665b6a"
BUILD_ENABLED = "${{ !inputs.allow-cache }}"


class ReleaseCacheProxyTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.action = {
            "inputs": {"enabled": {"default": "true"}},
            "runs": {
                "using": "node24",
                "pre": "pre.cjs",
                "main": "main.cjs",
                "post": "post.cjs",
            },
        }
        self.write(ACTION, self.action)
        for name in ("pre.cjs", "main.cjs", "post.cjs", "common.cjs", "action.py"):
            (self.root / Path(ACTION).parent / name).write_text("")

    def write(self, path: str, document: dict) -> None:
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(document))

    def steps(self, steps: list[dict], path: str = RELEASE) -> None:
        self.write(path, {"jobs": {"test": {"steps": steps}}})

    def check(self, expected_error: str = "") -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(self.root)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1 if expected_error else 0, result.stderr)
        self.assertEqual(
            result.stderr, f"error: {expected_error}\n" if expected_error else ""
        )
        self.assertEqual(
            result.stdout,
            ""
            if expected_error
            else "Release cache-denial action is required in every job.\n",
        )

    def test_proxy_must_be_first(self) -> None:
        for steps in (
            [],
            [{"run": "echo ok"}],
            [{"run": "echo ok"}, {"uses": USES}],
            [{"uses": "./.github/actions/disable-github-caches"}],
        ):
            with self.subTest(steps=steps):
                self.steps(steps)
                self.check(
                    f"{RELEASE}: jobs.test: cache-denial action must be the first step"
                )
        self.steps([{"uses": USES}, {"run": "echo ok"}])
        self.check()

    def test_release_cannot_skip_or_ignore_proxy(self) -> None:
        for settings in (
            {"if": "false"},
            {"continue-on-error": True},
            {"with": {"enabled": False}},
            {"with": {"enabled": BUILD_ENABLED}},
        ):
            with self.subTest(settings=settings):
                self.steps([{"uses": USES, **settings}])
                self.check(
                    f"{RELEASE}: jobs.test: cache-denial action must be enabled for releases"
                )

    def test_shared_build_preserves_ci_opt_out(self) -> None:
        self.write(RELEASE, {"jobs": {"build": {"uses": f"$/{BUILD}"}}})
        self.steps([{"uses": USES, "with": {"enabled": BUILD_ENABLED}}], BUILD)
        self.check()
        self.steps([{"uses": USES}], BUILD)
        self.check(
            f"{BUILD}: jobs.test: cache-denial action must be enabled for releases"
        )

    def test_reusable_workflows_are_checked(self) -> None:
        nested = ".github/workflows/nested.yml"
        self.write(RELEASE, {"jobs": {"nested": {"uses": f"./{nested}"}}})
        self.steps([{"run": "echo ok"}], nested)
        self.check(f"{nested}: jobs.test: cache-denial action must be the first step")
        self.steps([{"uses": USES}], nested)
        self.steps([{"run": "echo unrelated"}], ".github/workflows/release-prepare.yml")
        self.check()

    def test_entry_points_are_fixed(self) -> None:
        self.steps([{"uses": USES}])
        self.action["runs"]["pre"] = "other.cjs"
        self.write(ACTION, self.action)
        self.check(f"{ACTION}: cache-denial entry points or default changed")

    def test_entry_point_files_must_exist(self) -> None:
        self.steps([{"uses": USES}])
        for name in ("pre.cjs", "common.cjs", "action.py"):
            with self.subTest(name=name):
                path = self.root / Path(ACTION).parent / name
                path.unlink()
                self.check(f"{ACTION}: missing {name}")
                path.write_text("")


if __name__ == "__main__":
    unittest.main()
