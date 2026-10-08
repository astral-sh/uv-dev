#!/usr/bin/env python3

"""Install `pylint` and packages with native extensions into the system Python.

To run locally, create a venv with seed packages.
"""

import argparse
import logging
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional

# Resolve harness helpers even when Python uses an isolated import path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from package_fixtures import FixtureIndex, load_profiles

logger = logging.getLogger(__name__)


def install_package(
    *,
    uv: str,
    package: str,
    version: Optional[str] = None,
    path: Optional[Path] = None,
    import_check: Optional[str] = None,
):
    """Install a package into the system Python."""

    if path is not None:
        requirement = str(path)
    elif version is not None:
        requirement = f"{package}=={version}"
    else:
        requirement = package

    logger.info(f"Installing the package `{requirement}`.")
    subprocess.run(
        [uv, "--no-config", "pip", "install", requirement, "--system"]
        + allow_externally_managed,
        env=fixture_env,
        cwd=temp_dir,
        check=True,
    )

    logger.info(f"Checking that `{package}` can be imported with `{sys.executable}`.")
    code = subprocess.run(
        [sys.executable, "-c", import_check or f"import {package}"],
        cwd=temp_dir,
        check=False,
    )
    if code.returncode != 0:
        raise RuntimeError(f"Could not import {package}.")

    code = subprocess.run(
        [uv, "--no-config", "pip", "show", package, "--system"], check=False
    )
    if code.returncode != 0:
        raise RuntimeError(f"Could not show {package}.")


def install_native_extension(*, uv: str):
    """Build, install, and run a small native extension with the system Python."""

    fixture = Path(__file__).resolve().parents[1] / "test/packages/native_extension"
    path = Path(temp_dir) / "native_extension"
    shutil.copytree(fixture, path)
    install_package(
        uv=uv,
        package="uv_test_native_extension",
        path=path,
        import_check=(
            "import uv_test_native_extension as extension; "
            "assert extension.answer() == 42"
        ),
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")

    parser = argparse.ArgumentParser(description="Check a Python interpreter.")
    parser.add_argument("--uv", help="Path to a uv binary.")
    parser.add_argument(
        "--externally-managed",
        action="store_true",
        help="Set if the Python installation has an EXTERNALLY-MANAGED marker.",
    )
    parser.add_argument(
        "--python",
        required=False,
        help="Set if the system Python version must be explicitly specified, e.g., for prereleases.",
    )
    parser.add_argument(
        "--check-python-version",
        required=False,
        help="Verify that this tool has been started with the specified python version. Omitting the patch number will match any patch number.",
    )
    parser.add_argument(
        "--check-path",
        required=False,
        action="store_true",
        help="Attempt to verify that the PATH is set up so that this tool's python will match the python version that uv would automatically pick up.",
    )
    parser.add_argument(
        "--fixture-cache", type=Path, help="Retain verified archives for replay"
    )
    parser.add_argument(
        "--offline", action="store_true", help="Use only prepared fixture archives"
    )
    parser.add_argument(
        "--identity",
        type=Path,
        help="Save the selected workload and served archive identities",
    )
    args = parser.parse_args()

    uv: str = os.path.abspath(args.uv) if args.uv else "uv"
    allow_externally_managed = (
        ["--break-system-packages"] if args.externally_managed else []
    )
    python = ["--python", args.python] if args.python else []

    profile_name = "system-{}.{}".format(*sys.version_info[:2])
    profiles = load_profiles()
    if profile_name not in profiles:
        raise RuntimeError("No reviewed interpreter workload: " + profile_name)
    profile = profiles[profile_name]
    pylint_requirement = "pylint==" + profile["roots"]["pylint"]
    pydantic_core_version = profile["roots"].get("pydantic-core")

    if args.check_python_version:
        version = ".".join(map(str, sys.version_info[:3]))
        if args.check_python_version != version and not version.startswith(
            args.check_python_version + "."
        ):
            raise RuntimeError(
                f"Expected to be running {args.check_python_version} but we are on {version}."
            )

    if args.check_path:
        process = subprocess.run(
            [
                "python",
                "-c",
                "import os, sys; sys.stdout.buffer.write(os.fsencode(sys.executable))",
            ],
            check=True,
            stdout=subprocess.PIPE,
        )
        system_python_path = os.path.normcase(
            os.path.normpath(os.fsdecode(process.stdout))
        )
        our_python_path = os.path.normcase(os.path.normpath(sys.executable))

        if our_python_path != system_python_path:
            raise RuntimeError(
                f"Script was ran with {our_python_path} but `python` resolves to {system_python_path}"
            )

    # Ensure that pip is available (e.g., the Chainguard distroless image ships
    # Python but not pip).
    try:
        import pip  # noqa: F401
    except ModuleNotFoundError:
        logger.info("pip not found, running ensurepip...")
        subprocess.run(
            [sys.executable, "-m", "ensurepip"],
            check=True,
        )

    # Create a temporary directory.
    with tempfile.TemporaryDirectory() as temp_dir, FixtureIndex(
        profile, args.fixture_cache or Path(temp_dir) / "archives", offline=args.offline
    ) as fixture, fixture.record_identity(args.identity):
        constraints, build_constraints = fixture.constraints(temp_dir)
        fixture_env = fixture.environment(os.environ)
        fixture_env.update(
            {
                "UV_INDEX_URL": fixture.url,
                "UV_CACHE_DIR": str(Path(temp_dir) / "uv-cache"),
                "UV_CONSTRAINT": constraints.name,
                "UV_BUILD_CONSTRAINT": build_constraints.name,
            }
        )
        # Ensure that the package (`pylint`) isn't installed.
        logger.info("Checking that `pylint` isn't installed.")
        code = subprocess.run(
            [sys.executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode == 0:
            raise RuntimeError("The package `pylint` is installed (but shouldn't be).")

        # Install the package (`pylint`).
        logger.info("Installing the package `pylint`.")
        subprocess.run(
            [
                uv,
                "--no-config",
                "pip",
                "install",
                pylint_requirement,
                "--system",
                "--verbose",
            ]
            + allow_externally_managed
            + python,
            cwd=temp_dir,
            check=True,
            env=fixture_env,
        )

        # Ensure that the package (`pylint`) is installed.
        logger.info(
            f"Checking that `pylint` is installed with `{sys.executable} -m pip`."
        )
        code = subprocess.run(
            [sys.executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode != 0:
            raise RuntimeError("The package `pylint` isn't installed (but should be).")

        logger.info("Checking that `pylint` is in the path.")
        if shutil.which("pylint") is None:
            raise RuntimeError("The package `pylint` isn't in the path.")

        # Uninstall the package (`pylint`).
        logger.info("Uninstalling the package `pylint`.")
        subprocess.run(
            [uv, "--no-config", "pip", "uninstall", "pylint", "--system"]
            + allow_externally_managed
            + python,
            cwd=temp_dir,
            check=True,
        )

        # Ensure that the package (`pylint`) isn't installed.
        logger.info("Checking that `pylint` isn't installed.")
        code = subprocess.run(
            [sys.executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode == 0:
            raise RuntimeError("The package `pylint` is installed (but shouldn't be).")

        # Create a virtual environment with `uv`.
        logger.info("Creating virtual environment with `uv`...")
        subprocess.run(
            [
                uv,
                "--no-config",
                "venv",
                ".venv",
                "--seed",
                "--python",
                sys.executable,
            ],
            env=fixture_env,
            cwd=temp_dir,
            check=True,
        )

        if os.name == "nt":
            executable = os.path.join(temp_dir, ".venv", "Scripts", "python.exe")
        else:
            executable = os.path.join(temp_dir, ".venv", "bin", "python")

        logger.info("Querying virtual environment...")
        subprocess.run(
            [executable, "--version"],
            cwd=temp_dir,
            check=True,
        )

        logger.info("Installing into `uv` virtual environment...")

        # Disable the `CONDA_PREFIX` and `VIRTUAL_ENV` environment variables, so that
        # we only rely on virtual environment discovery via the `.venv` directory.
        # Our "system Python" here might itself be a Conda environment!
        env = fixture_env.copy()
        env["CONDA_PREFIX"] = ""
        env["VIRTUAL_ENV"] = ""
        subprocess.run(
            [uv, "--no-config", "pip", "install", pylint_requirement, "--verbose"],
            cwd=temp_dir,
            check=True,
            env=env,
        )

        # Ensure that the package (`pylint`) isn't installed globally.
        logger.info("Checking that `pylint` isn't installed.")
        code = subprocess.run(
            [sys.executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode == 0:
            raise RuntimeError(
                "The package `pylint` is installed globally (but shouldn't be)."
            )

        # Ensure that the package (`pylint`) is installed in the virtual environment.
        logger.info("Checking that `pylint` is installed.")
        code = subprocess.run(
            [executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode != 0:
            raise RuntimeError(
                "The package `pylint` isn't installed in the virtual environment."
            )

        # Uninstall the package (`pylint`).
        logger.info("Uninstalling the package `pylint`.")
        subprocess.run(
            [uv, "--no-config", "pip", "uninstall", "pylint", "--verbose"],
            cwd=temp_dir,
            check=True,
            env=env,
        )

        # Ensure that the package (`pylint`) isn't installed in the virtual environment.
        logger.info("Checking that `pylint` isn't installed.")
        code = subprocess.run(
            [executable, "-m", "pip", "show", "pylint"],
            cwd=temp_dir,
            check=False,
        )
        if code.returncode == 0:
            raise RuntimeError(
                "The package `pylint` is installed in the virtual environment (but shouldn't be)."
            )

        # Build and import a native extension on interpreters with build tools in CI.
        if sys.version_info < (3, 13) and sys.implementation.name != "graalpy":
            install_native_extension(uv=uv)

        # Attempt to install `pydantic_core`.
        # This ensures that we can successfully install and recognize a package that may
        # be installed into `platlib`.
        #
        # `pydantic_core` doesn't distribute wheels for non-CPython interpreters, nor
        # for Python 3.13 (at time of writing).
        if (
            sys.version_info >= (3, 7)
            and sys.version_info < (3, 13)
            and sys.implementation.name == "cpython"
        ):
            install_package(
                uv=uv, package="pydantic_core", version=pydantic_core_version
            )

        # Next, create a virtual environment with `venv`, to ensure that `uv` can
        # interoperate with `venv` virtual environments.
        shutil.rmtree(os.path.join(temp_dir, ".venv"))
        logger.info("Creating virtual environment with `venv`...")
        subprocess.run(
            [sys.executable, "-m", "venv", ".venv"],
            cwd=temp_dir,
            check=True,
        )

        # Install the package (`pylint`) into the virtual environment.
        logger.info("Installing into `venv` virtual environment...")
        subprocess.run(
            [uv, "--no-config", "pip", "install", pylint_requirement, "--verbose"],
            cwd=temp_dir,
            check=True,
            env=env,
        )

        # Uninstall the package (`pylint`).
        logger.info("Uninstalling the package `pylint`.")
        subprocess.run(
            [uv, "--no-config", "pip", "uninstall", "pylint", "--verbose"],
            cwd=temp_dir,
            check=True,
            env=env,
        )
