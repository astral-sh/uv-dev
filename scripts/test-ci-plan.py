"""Exercise release-build selection in the CI planner without running builds."""

import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

WORKFLOW = Path(__file__).resolve().parent.parent / ".github/workflows/plan.yml"


def classification_script() -> str:
    lines = WORKFLOW.read_text().splitlines()
    start = lines.index("          while IFS= read -r file; do")
    end = lines.index('          } >> "$GITHUB_OUTPUT"', start)
    return textwrap.dedent("\n".join(lines[start : end + 1]))


class ReleaseBuildSelection(unittest.TestCase):
    def plan(self, path: str, **flags: str) -> dict[str, str]:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "outputs"
            subprocess.run(
                [
                    "bash",
                    "--noprofile",
                    "--norc",
                    "-eo",
                    "pipefail",
                    "-c",
                    classification_script(),
                ],
                env={
                    "PATH": os.environ["PATH"],
                    "changed_files": path,
                    "GITHUB_OUTPUT": str(output),
                    **flags,
                },
                check=True,
                capture_output=True,
                text=True,
            )
            return dict(line.split("=", 1) for line in output.read_text().splitlines())

    def test_release_helpers_select_builds(self):
        for path in (
            "scripts/build_uv_pgo.py",
            "scripts/cargo.sh",
            "scripts/cargo.cmd",
            "scripts/install-cargo-extensions.sh",
            "scripts/transform_readme.py",
            "scripts/repair-sdist-cargo-lock.py",
            ".github/workflows/build-release-binaries.yml",
        ):
            with self.subTest(path=path):
                result = self.plan(path)
                self.assertEqual(result["build_release_binaries"], "true")
                self.assertEqual(result["test_macos"], "true")

    def test_unrelated_scripts_do_not_select_release_builds(self):
        result = self.plan("scripts/fetch-python-downloads.py")
        self.assertEqual(result["build_release_binaries"], "false")
        self.assertEqual(result["test_code"], "true")

    def test_skip_labels_still_skip_release_builds(self):
        for flag in (
            "has_skip_label",
            "has_build_skip_label",
            "has_build_skip_release_label",
        ):
            with self.subTest(flag=flag):
                result = self.plan("scripts/build_uv_pgo.py", **{flag: "1"})
                self.assertEqual(result["build_release_binaries"], "false")

    def test_explicit_release_build_label(self):
        result = self.plan("README.md", has_build_release_label="1")
        self.assertEqual(result["build_release_binaries"], "true")


if __name__ == "__main__":
    unittest.main()
