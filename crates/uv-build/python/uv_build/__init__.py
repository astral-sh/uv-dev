"""Provide PEP 517 and PEP 660 build backend hooks.

Import large modules only when a hook needs them. Imports can increase startup time:
```
$ hyperfine \
     "/usr/bin/python3 -c \"print('hi')\"" \
     "/usr/bin/python3 -c \"from subprocess import check_call; print('hi')\""
Base: Time (mean ± σ):      11.0 ms ±   1.7 ms    [User: 8.5 ms, System: 2.5 ms]
With import: Time (mean ± σ):      15.2 ms ±   2.0 ms    [User: 12.3 ms, System: 2.9 ms]
Base 1.38 ± 0.28 times faster than with import
```

Use quoted Python 3.10 type annotations to avoid importing `typing` at runtime.
Older Python versions ignore the quoted annotations. Editors and type checkers can
still read them.
"""

TYPE_CHECKING = False
if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence
    from typing import Any

# Run `uv build-backend` instead of `uv-build` when this option is enabled.
# Downstream distributors that already provide `uv` can avoid building a separate
# executable with overlapping functionality.
USE_UV_EXECUTABLE = False


def warn_config_settings(config_settings: "Mapping[Any, Any] | None" = None) -> None:
    """Warn when a caller supplies unsupported build settings."""
    import sys

    if config_settings:
        print("Warning: Config settings are not supported", file=sys.stderr)


def call(
    args: "Sequence[str]", config_settings: "Mapping[Any, Any] | None" = None
) -> str:
    """Run the build backend and return its output filename."""
    import shutil
    import subprocess
    import sys

    warn_config_settings(config_settings)

    uv_bin_name = "uv" if USE_UV_EXECUTABLE else "uv-build"
    # Search `PATH` so the executable follows the PEP 517 build environment.
    uv_bin = shutil.which(uv_bin_name)
    if uv_bin is None:
        raise RuntimeError(f"{uv_bin_name} was not properly installed")
    build_backend_args = ["build-backend"] if USE_UV_EXECUTABLE else []
    # Send standard error to the caller and capture the output filename.
    result = subprocess.run(
        [uv_bin, *build_backend_args, *args], stdout=subprocess.PIPE, check=False
    )
    if result.returncode != 0:
        sys.exit(result.returncode)
    # Forward any output that appears before the filename.
    stdout = result.stdout.decode("utf-8").strip().splitlines(keepends=True)
    sys.stdout.writelines(stdout[:-1])
    # Show a clear error when the subprocess does not return a filename.
    if not stdout:
        print(
            f"{uv_bin_name} subprocess did not return a filename on stdout",
            file=sys.stderr,
        )
        sys.exit(1)
    return stdout[-1].strip()


def build_sdist(
    sdist_directory: str, config_settings: "Mapping[Any, Any] | None" = None
) -> str:
    """Build a source distribution with the PEP 517 `build_sdist` hook."""
    args = ["build-sdist", sdist_directory]
    return call(args, config_settings)


def build_wheel(
    wheel_directory: str,
    config_settings: "Mapping[Any, Any] | None" = None,
    metadata_directory: "str | None" = None,
) -> str:
    """Build a wheel with the PEP 517 `build_wheel` hook."""
    args = ["build-wheel", wheel_directory]
    if metadata_directory:
        args.extend([metadata_directory])
    return call(args, config_settings)


def get_requires_for_build_sdist(
    config_settings: "Mapping[Any, Any] | None" = None,
) -> "Sequence[str]":
    """Return the extra build requirements for a source distribution."""
    warn_config_settings(config_settings)
    return []


def get_requires_for_build_wheel(
    config_settings: "Mapping[Any, Any] | None" = None,
) -> "Sequence[str]":
    """Return the extra build requirements for a wheel."""
    warn_config_settings(config_settings)
    return []


def prepare_metadata_for_build_wheel(
    metadata_directory: str, config_settings: "Mapping[Any, Any] | None" = None
) -> str:
    """Prepare wheel metadata with the PEP 517 metadata hook."""
    args = ["prepare-metadata-for-build-wheel", metadata_directory]
    return call(args, config_settings)


def build_editable(
    wheel_directory: str,
    config_settings: "Mapping[Any, Any] | None" = None,
    metadata_directory: "str | None" = None,
) -> str:
    """Build an editable wheel with the PEP 660 `build_editable` hook."""
    args = ["build-editable", wheel_directory]
    if metadata_directory:
        args.extend([metadata_directory])
    return call(args, config_settings)


def get_requires_for_build_editable(
    config_settings: "Mapping[Any, Any] | None" = None,
) -> "Sequence[str]":
    """Return the extra build requirements for an editable wheel."""
    warn_config_settings(config_settings)
    return []


def prepare_metadata_for_build_editable(
    metadata_directory: str, config_settings: "Mapping[Any, Any] | None" = None
) -> str:
    """Prepare editable wheel metadata with the PEP 660 metadata hook."""
    args = ["prepare-metadata-for-build-editable", metadata_directory]
    return call(args, config_settings)
