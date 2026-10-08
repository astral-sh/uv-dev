#!/usr/bin/env python3

"""Install `pylint` and `numpy` into an embedded Python."""

import argparse
import logging
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# Embedded Python does not put the script directory on its isolated import path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from package_fixtures import FixtureIndex, load_profiles

logger = logging.getLogger(__name__)

if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")

    parser = argparse.ArgumentParser(
        description="Check an embedded Python interpreter."
    )
    parser.add_argument("--uv", help="Path to a uv binary.")
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

    if sys.version_info[:2] != (3, 11):
        raise RuntimeError("The embedded workload requires Python 3.11")
    profile = load_profiles()["embedded-3.11"]

    # Create a temporary directory.
    with (
        tempfile.TemporaryDirectory() as temp_dir,
        FixtureIndex(
            profile,
            args.fixture_cache or Path(temp_dir) / "archives",
            offline=args.offline,
        ) as fixture,
        fixture.record_identity(args.identity),
    ):
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

        # Install, verify, and uninstall a few packages.
        for package in ["pylint", "numpy"]:
            # Install the package.
            logger.info(
                f"Installing the package `{package}` into the virtual environment..."
            )
            subprocess.run(
                [
                    uv,
                    "--no-config",
                    "pip",
                    "install",
                    package + "==" + profile["roots"][package],
                    "--verbose",
                ],
                cwd=temp_dir,
                check=True,
                env=env,
            )

            # Ensure that the package is installed in the virtual environment.
            logger.info(f"Checking that `{package}` is installed.")
            code = subprocess.run(
                [executable, "-c", f"import {package}"],
                cwd=temp_dir,
                check=False,
            )
            if code.returncode != 0:
                raise RuntimeError(
                    f"The package `{package}` isn't installed in the virtual environment."
                )

            # Uninstall the package.
            logger.info(f"Uninstalling the package `{package}`.")
            subprocess.run(
                [uv, "--no-config", "pip", "uninstall", package, "--verbose"],
                cwd=temp_dir,
                check=True,
                env=env,
            )

            # Ensure that the package isn't installed in the virtual environment.
            logger.info(f"Checking that `{package}` isn't installed.")
            code = subprocess.run(
                [executable, "-m", "pip", "show", package],
                cwd=temp_dir,
                check=False,
            )
            if code.returncode == 0:
                raise RuntimeError(
                    f"The package `{package}` is installed in the virtual environment (but shouldn't be)."
                )
