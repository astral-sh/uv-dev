"""PEP 517/660 hooks executing the complete Rust backend as generated Python."""

import os
import sys

if os.name == "nt":
    from ._host_windows import Host
else:
    from ._host import Host
from ._vm import Machine, Program

_PROGRAM = None


def _new_program():
    if os.name == "nt":
        module = "._windows64" if sys.maxsize > 0xFFFFFFFF else "._windows32"
        return Program(package=__package__ + module)
    return Program()


def warn_config_settings(config_settings=None):
    if config_settings:
        print("Warning: Config settings are not supported", file=sys.stderr)


def call(args, config_settings=None, *, _version="0.13.0", _source_tree=None):
    global _PROGRAM
    warn_config_settings(config_settings)
    if _PROGRAM is None:
        _PROGRAM = _new_program()
    host = Host(args, _version, _source_tree)
    try:
        status = 0
        try:
            Machine(_PROGRAM, host).start()
        except Exception as error:
            if len(error.args) == 2 and error.args[0] is SystemExit:
                status = error.args[1]
            else:
                raise
        sys.stderr.write(host.stderr.getvalue().decode("utf-8"))
        if status:
            raise SystemExit(status)
        stdout = (
            host.stdout.getvalue().decode("utf-8").strip().splitlines(keepends=True)
        )
        sys.stdout.writelines(stdout[:-1])
        if not stdout:
            print(
                "uv-build subprocess did not return a filename on stdout",
                file=sys.stderr,
            )
            raise SystemExit(1)
        return stdout[-1].strip()
    finally:
        host.close()


def build_sdist(sdist_directory, config_settings=None):
    return call(["build-sdist", sdist_directory], config_settings)


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    args = ["build-wheel", wheel_directory]
    if metadata_directory:
        args.append(metadata_directory)
    return call(args, config_settings)


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    args = ["build-editable", wheel_directory]
    if metadata_directory:
        args.append(metadata_directory)
    return call(args, config_settings)


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    return call(
        ["prepare-metadata-for-build-wheel", metadata_directory], config_settings
    )


def prepare_metadata_for_build_editable(metadata_directory, config_settings=None):
    return call(
        ["prepare-metadata-for-build-editable", metadata_directory], config_settings
    )


def get_requires_for_build_wheel(config_settings=None):
    warn_config_settings(config_settings)
    return []


get_requires_for_build_sdist = get_requires_for_build_wheel
get_requires_for_build_editable = get_requires_for_build_wheel
