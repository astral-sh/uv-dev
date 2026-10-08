#!/usr/bin/env python3

"""
Install packages on multiple versions of uv to check for cache compatibility errors.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import subprocess
import sys
import tempfile
from functools import partial
from pathlib import Path
from zipfile import ZipFile

from package_fixtures import FixtureIndex, load_profiles, normalize_name, sha256

logger = logging.getLogger(__name__)

DEFAULT_TEST_PACKAGES = [
    # anyio is used throughout our test suite as a minimal dependency
    "anyio",
    # flask is another standard test dependency for us, but bigger than anyio
    "flask",
]

if sys.platform == "linux":
    DEFAULT_TEST_PACKAGES += [
        # homeassistant has a lot of dependencies and should be built from source
        # this requires additional dependencies on macOS so we gate it to Linux
        "homeassistant",
    ]


def install_package(
    *,
    uv: str,
    package: str,
    flags: list[str],
    installed_name: str | None = None,
    fixture: FixtureIndex | None = None,
):
    """Install a package"""

    logger.info(f"Installing the package {package!r} with {uv!r}.")
    environment = os.environ if fixture is None else fixture.environment(os.environ)
    if fixture is not None:
        constraints, build_constraints = fixture.constraints(temp_dir)
        flags = flags + [
            "--index-url",
            fixture.url,
            "--constraint",
            str(constraints),
            "--build-constraint",
            str(build_constraints),
        ]
    subprocess.run(
        [
            uv,
            "--no-config",
            "pip",
            "install",
            "--python",
            str(Path(temp_dir) / ".venv"),
            package,
            "--cache-dir",
            os.path.join(temp_dir, "cache"),
        ]
        + flags,
        cwd=temp_dir,
        env=environment,
        check=True,
    )

    package_name = installed_name or package
    logger.info(f"Checking that `{package_name}` is available.")
    code = subprocess.run(
        [
            uv,
            "--no-config",
            "pip",
            "show",
            "--python",
            str(Path(temp_dir) / ".venv"),
            package_name,
        ],
        cwd=temp_dir,
        check=False,
    )
    if code.returncode != 0:
        raise RuntimeError(f"Could not show {package_name}.")
    if fixture is not None:
        installed = json.loads(
            subprocess.check_output(
                [
                    uv,
                    "--no-config",
                    "pip",
                    "list",
                    "--python",
                    str(Path(temp_dir) / ".venv"),
                    "--format",
                    "json",
                ],
                cwd=temp_dir,
                text=True,
            )
        )
        actual = {
            normalize_name(package["name"]): package["version"] for package in installed
        }
        if actual != fixture.profile["requirements"]:
            raise RuntimeError(
                f"Installed graph differs from the frozen profile: {actual}"
            )


def clean_cache(*, uv: str):
    subprocess.run(
        [uv, "cache", "clean", "--cache-dir", os.path.join(temp_dir, "cache")],
        cwd=temp_dir,
        check=True,
    )


def check_local_wheel_cache(*, uv_current: str, uv_previous: str):
    """Check that an older uv can read a wheel cached by the current uv."""

    package = "uv-cache-compatibility"
    distribution = package.replace("-", "_")
    version = "1.0.0"
    wheel = Path(temp_dir) / f"{distribution}-{version}-py3-none-any.whl"
    dist_info = f"{distribution}-{version}.dist-info"
    files = {
        f"{distribution}/__init__.py": "",
        f"{dist_info}/METADATA": (
            f"Metadata-Version: 2.1\nName: {package}\nVersion: {version}\n"
        ),
        f"{dist_info}/WHEEL": (
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    record = "".join(f"{path},,\n" for path in files)
    files[f"{dist_info}/RECORD"] = f"{record}{dist_info}/RECORD,,\n"

    with ZipFile(wheel, "w") as archive:
        for path, contents in files.items():
            archive.writestr(path, contents)

    install_package(uv=uv_current, package=str(wheel), flags=[], installed_name=package)
    install_package(
        uv=uv_previous,
        package=str(wheel),
        flags=["--reinstall"],
        installed_name=package,
    )


def check_cache_with_package(
    *,
    uv_current: str,
    uv_previous: str,
    package: str,
    fixture: FixtureIndex,
):
    install = partial(
        install_package,
        package=f"{package}=={fixture.profile['roots'][package]}",
        installed_name=package,
        fixture=fixture,
    )
    # The coverage here is rough and not particularly targeted — we're just performing various
    # operations in the hope of catching cache load issues. As cache problems are discovered in
    # the future, we should expand coverage with targeted cases.

    # First, install with the previous uv to populate the cache
    install(uv=uv_previous, flags=[])

    # Audit with the current uv, this shouldn't hit the cache but is fast
    install(uv=uv_current, flags=[])

    # Reinstall with the current uv
    install(uv=uv_current, flags=["--reinstall"])

    # Reinstall with the current uv and refresh a single entry
    install(
        uv=uv_current,
        flags=["--reinstall-package", package, "--refresh-package", package],
    )

    # Reinstall with the current uv post refresh
    install(uv=uv_current, flags=["--reinstall"])

    # Reinstall with the current uv post refresh
    install(uv=uv_previous, flags=["--reinstall"])

    # Clear the cache
    clean_cache(uv=uv_previous)

    # Install with the previous uv to populate the cache
    # Use `--no-binary` to force a local build of the wheel
    install(
        uv=uv_previous,
        flags=["--reinstall", "--no-binary", package],
    )

    # Reinstall with the current uv from the source-built wheel cache.
    install(
        uv=uv_current,
        flags=["--reinstall", "--no-binary", package],
    )

    # Clear the cache and reverse the source-build direction to ensure previous releases can read
    # wheel and source-distribution entries written by the current version.
    clean_cache(uv=uv_current)
    install(
        uv=uv_current,
        flags=["--reinstall", "--no-binary", package],
    )
    install(
        uv=uv_previous,
        flags=["--reinstall", "--no-binary", package],
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")

    parser = argparse.ArgumentParser(description="Check a Python interpreter.")
    parser.add_argument(
        "-c", "--uv-current", help="Path to a current uv binary.", required=True
    )
    parser.add_argument(
        "-p", "--uv-previous", help="Path to a previous uv binary.", required=True
    )
    parser.add_argument(
        "-t",
        "--test-package",
        action="append",
        type=str,
        choices=DEFAULT_TEST_PACKAGES,
        help="A frozen package profile to test. May be provided multiple times.",
    )
    parser.add_argument(
        "--fixture-cache",
        type=Path,
        required=True,
        help="Verified archive cache, retained for replay",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Require all archives to be prepared in the fixture cache",
    )
    parser.add_argument(
        "--identity",
        type=Path,
        required=True,
        help="Write the binary and workload identities used by this run",
    )
    args = parser.parse_args()

    uv_current = os.path.abspath(args.uv_current)
    uv_previous = os.path.abspath(args.uv_previous)
    test_packages = args.test_package or DEFAULT_TEST_PACKAGES

    profiles = load_profiles()
    identity = {
        "python": sys.version,
        "binaries": {
            label: {
                "sha256": sha256(binary),
                "version": subprocess.check_output(
                    [binary, "--version"], text=True
                ).strip(),
            }
            for label, binary in (("current", uv_current), ("previous", uv_previous))
        },
        "workloads": {},
    }
    try:
        # Each workload owns an environment and a uv cache. The archive cache is shared so the
        # exact same registry responses can be replayed after public network access is removed.
        for package in [None, *test_packages]:
            with tempfile.TemporaryDirectory() as temp_dir:
                subprocess.run(
                    [uv_current, "--no-config", "venv", "--python", sys.executable],
                    cwd=temp_dir,
                    check=True,
                )
                if package is None:
                    check_local_wheel_cache(
                        uv_current=uv_current, uv_previous=uv_previous
                    )
                    continue
                profile = profiles["cache-" + package]
                with FixtureIndex(
                    profile, args.fixture_cache, offline=args.offline
                ) as fixture:
                    try:
                        check_cache_with_package(
                            uv_current=uv_current,
                            uv_previous=uv_previous,
                            package=package,
                            fixture=fixture,
                        )
                    finally:
                        identity["workloads"][package] = fixture.identity()
    finally:
        args.identity.write_text(json.dumps(identity, indent=2, sort_keys=True) + "\n")
