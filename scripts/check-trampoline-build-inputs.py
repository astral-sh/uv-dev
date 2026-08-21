"""Check that production trampoline inputs select both trampoline CI gates."""

# /// script
# requires-python = ">=3.12"
# [tool.uv]
# no-build = true
# ///

from __future__ import annotations

import os
import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/plan.yml"


def local_dependencies() -> set[Path]:
    """Include local build dependencies for every target and optional feature."""
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    shared = workspace["workspace"]["dependencies"]
    pending = [ROOT / "crates/uv-trampoline", ROOT / "crates/uv-trampoline-builder"]
    visited = set()
    for patches in workspace.get("patch", {}).values():
        pending.extend(
            ROOT / entry["path"] for entry in patches.values() if "path" in entry
        )
    while pending:
        directory = pending.pop().resolve()
        if directory in visited:
            continue
        directory.relative_to(ROOT)
        visited.add(directory)
        manifest = tomllib.loads((directory / "Cargo.toml").read_text())
        tables = [manifest, *manifest.get("target", {}).values()]
        for table in tables:
            for kind in ("dependencies", "build-dependencies"):
                for name, entry in table.get(kind, {}).items():
                    if not isinstance(entry, dict):
                        continue
                    base = directory
                    if entry.get("workspace"):
                        entry = shared[name]
                        base = ROOT
                    if isinstance(entry, dict) and "path" in entry:
                        pending.append(base / entry["path"])
        for patches in manifest.get("patch", {}).values():
            pending.extend(
                directory / entry["path"]
                for entry in patches.values()
                if "path" in entry
            )
    return {directory.relative_to(ROOT) for directory in visited}


class TrampolineBuildInputs(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = WORKFLOW.read_text()
        lines = workflow.splitlines(keepends=True)
        plan = next(
            index for index, line in enumerate(lines) if line.strip() == "id: plan"
        )
        start = (
            next(
                index
                for index in range(plan, len(lines))
                if lines[index].strip() == "run: |"
            )
            + 1
        )
        body = []
        for line in lines[start:]:
            if line.strip() and not line.startswith("          "):
                break
            body.append(line.removeprefix("          "))
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.directory = Path(cls.temporary.name)
        cls.script = cls.directory / "plan.sh"
        cls.script.write_text("".join(body))
        # Supply changed-file fixtures while executing the actual planner shell.
        git = cls.directory / "git"
        git.write_text(
            '#!/bin/sh\n[ "$1" = diff ] || exit 1\nprintf "%s\\n" "$PLANNER_CHANGED_FILES"\n'
        )
        git.chmod(0o755)
        cls.labels = re.findall(r"^          (HAS_\w+):", workflow, re.MULTILINE)

    def plan(
        self, *paths: str, label: bool = False, skip: bool = False
    ) -> dict[str, str]:
        output = self.directory / "output"
        output.write_text("")
        environment = {
            "PATH": f"{self.directory}{os.pathsep}{os.environ['PATH']}",
            "GH_REF": "refs/pull/1/merge",
            "BASE_SHA": "base",
            "GITHUB_OUTPUT": str(output),
            "PLANNER_CHANGED_FILES": "\n".join(paths),
            **dict.fromkeys(self.labels, "false"),
        }
        environment["HAS_BUILD_WINDOWS_TRAMPOLINE_LABEL"] = str(label).lower()
        environment["HAS_SKIP_LABEL"] = str(skip).lower()
        subprocess.run(
            ["bash", "-e", "-o", "pipefail", str(self.script)],
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        )
        return dict(line.split("=", 1) for line in output.read_text().splitlines())

    def assert_gates(self, path: str, expected: bool, **kwargs):
        with self.subTest(path=path, **kwargs):
            result = self.plan(path, **kwargs)
            self.assertEqual(result["test_windows_trampoline"], str(expected).lower())
            self.assertEqual(
                result["test_windows_trampoline_check_binary"], str(expected).lower()
            )

    def test_local_dependency_inputs(self):
        # A new local dependency must not silently fall outside the planner's prefixes.
        for directory in sorted(local_dependencies()):
            for name in ("src/lib.rs", "build.rs", "Cargo.toml", "data.bin"):
                self.assert_gates((directory / name).as_posix(), True)

    def test_workspace_manifests(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
        for member in workspace["members"]:
            for directory in ROOT.glob(member):
                self.assert_gates(
                    (directory / "Cargo.toml").relative_to(ROOT).as_posix(), True
                )

    def test_generator_configuration(self):
        for path in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "crates/.cargo/config.toml",
            "crates/uv-trampoline/Cargo.lock",
            "crates/uv-trampoline/rust-toolchain.toml",
            "crates/uv-trampoline/.cargo/config.toml",
            "crates/uv-trampoline/Dockerfile",
            "crates/uv-trampoline-builder/src/bin/normalize-pe-timestamps.rs",
            "scripts/build-trampolines.sh",
            "scripts/build-trampolines-in-docker.sh",
            ".github/workflows/ci.yml",
            ".github/workflows/test-windows-trampolines.yml",
        ):
            self.assert_gates(path, True)

    def test_production_artifacts(self):
        for architecture in ("i686", "x86_64", "aarch64"):
            for variant in ("console", "gui"):
                self.assert_gates(
                    f"crates/uv-trampoline-builder/trampolines/uv-trampoline-{architecture}-{variant}.exe",
                    True,
                )

    def test_unrelated_paths(self):
        for path in (
            "README.md",
            "docs/index.md",
            "crates/uv-resolver/src/lib.rs",
            "crates/uv-static-extra/src/lib.rs",
            "crates/uv-trampoline-extra/src/lib.rs",
            "scripts/build-trampolines-not-a-generator.sh",
        ):
            self.assert_gates(path, False)

    def test_label_override_and_skip(self):
        self.assert_gates("", True, label=True)
        self.assert_gates("crates/uv-trampoline/rust-toolchain.toml", False, skip=True)
        self.assert_gates("", False, label=True, skip=True)


if __name__ == "__main__":
    unittest.main()
