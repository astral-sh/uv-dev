import importlib.util
import os
import subprocess
import sys
import unittest
from collections.abc import Callable
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import cast
from unittest.mock import patch


class RuntimeInstallerTests(unittest.TestCase):
    def test_install_pins_project_and_interpreter_without_changing_environment(
        self,
    ) -> None:
        project_root = Path(__file__).resolve().parents[3]
        installer_path = project_root / ".github/actions/setup-automations/install.py"
        specification = importlib.util.spec_from_file_location(
            "automation_runtime_installer", installer_path
        )
        if specification is None or specification.loader is None:
            raise AssertionError("Could not load the runtime installer")
        installer = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(installer)
        install = cast(Callable[[], None], installer.main)

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "runtime"
            output = root / "output"
            inherited = {
                "RUNNER_TEMP": str(root),
                "GITHUB_OUTPUT": str(output),
                "UV_PROJECT": str(root / "wrong-project"),
                "UV_PYTHON": "3.13",
                "UV_DEFAULT_INDEX": "https://example.invalid/simple",
            }
            observed: list[tuple[list[str], dict[str, str]]] = []

            def sync(
                arguments: list[str], *, env: dict[str, str], check: bool
            ) -> subprocess.CompletedProcess[str]:
                self.assertTrue(check)
                observed.append((arguments, env))
                scripts = directory / ("Scripts" if os.name == "nt" else "bin")
                suffix = ".exe" if os.name == "nt" else ""
                scripts.mkdir(parents=True)
                (scripts / f"python{suffix}").touch()
                (scripts / f"uv-automations{suffix}").touch()
                return subprocess.CompletedProcess(arguments, 0)

            with (
                patch.dict(os.environ, inherited),
                patch.object(installer, "mkdtemp", return_value=str(directory)),
                patch("subprocess.run", side_effect=sync),
            ):
                install()
                self.assertEqual(os.environ["UV_PYTHON"], "3.13")

            self.assertEqual(len(observed), 1)
            arguments, environment = observed[0]
            project = str(project_root / "scripts/automations")
            self.assertEqual(
                arguments,
                [
                    "uv",
                    "sync",
                    "--directory",
                    project,
                    "--project",
                    project,
                    "--locked",
                    "--no-dev",
                    "--no-editable",
                    "--reinstall-package",
                    "uv-automations",
                    "--link-mode",
                    "copy",
                    "--python",
                    sys.executable,
                ],
            )
            self.assertEqual(environment["UV_PROJECT_ENVIRONMENT"], str(directory))
            self.assertEqual(
                environment["UV_DEFAULT_INDEX"], inherited["UV_DEFAULT_INDEX"]
            )
            self.assertNotIn("VIRTUAL_ENV", environment)
            self.assertEqual(
                {line.split("=", 1)[0] for line in output.read_text().splitlines()},
                {"executable", "python"},
            )
