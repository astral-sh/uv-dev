# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Exercise a native uv binary using only temporary, locally generated wheels.

Run `uv run --offline scripts/bazel/smoke.py --binary /path/to/uv`.
The supplied binary must run on this machine; a remote Linux build cannot run on macOS.
"""

import argparse
import base64
import csv
import hashlib
import io
import os
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

VERSION = "0.0.1"


def write_wheel(wheelhouse, name, source, requires=None):
    package = name.replace("-", "_")
    dist_info = f"{package}-{VERSION}.dist-info"
    metadata = f"Metadata-Version: 2.1\nName: {name}\nVersion: {VERSION}\n"
    if requires:
        metadata += f"Requires-Dist: {requires}=={VERSION}\n"
    files = {
        f"{package}/__init__.py": source.encode(),
        f"{dist_info}/METADATA": (metadata + "\n").encode(),
        f"{dist_info}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: uv-bazel-smoke\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n"
        ).encode(),
    }
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for path, content in files.items():
        digest = base64.urlsafe_b64encode(hashlib.sha256(content).digest()).rstrip(b"=")
        writer.writerow([path, f"sha256={digest.decode()}", len(content)])
    record_path = f"{dist_info}/RECORD"
    writer.writerow([record_path, "", ""])
    files[record_path] = record.getvalue().encode()
    wheel = wheelhouse / f"{package}-{VERSION}-py3-none-any.whl"
    with zipfile.ZipFile(wheel, "x") as archive:
        for path, content in files.items():
            archive.writestr(
                zipfile.ZipInfo(path, date_time=(2020, 1, 1, 0, 0, 0)), content
            )


def isolated_environment(directory):
    # Start empty: do not inherit UV_*, PIP_*, credentials, Python paths, or venvs.
    environment = {
        "PATH": os.defpath,
        "PYTHONUTF8": "1",
        "UV_NO_CONFIG": "1",
        "UV_NO_SYSTEM_CONFIG": "1",
        "UV_OFFLINE": "1",
        "UV_PYTHON_DOWNLOADS": "never",
        "UV_CACHE_DIR": str(directory / "cache"),
        "UV_PYTHON_INSTALL_DIR": str(directory / "python-installs"),
        "XDG_CACHE_HOME": str(directory / "cache"),
        "XDG_CONFIG_HOME": str(directory / "config"),
        "XDG_DATA_HOME": str(directory / "data"),
        "APPDATA": str(directory / "config"),
        "LOCALAPPDATA": str(directory / "cache"),
        "TMPDIR": str(directory),
        "TMP": str(directory),
        "TEMP": str(directory),
    }
    # Windows needs its system directory to start Python reliably.
    for name in ("SYSTEMROOT", "WINDIR", "COMSPEC"):
        if name in os.environ:
            environment[name] = os.environ[name]
    return environment


def run_command(label, arguments, directory, environment):
    try:
        return subprocess.run(
            arguments,
            cwd=directory,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
            timeout=60,
        ).stdout
    except subprocess.CalledProcessError as error:
        raise RuntimeError(
            f"{label} failed (exit {error.returncode}):\n{error.stdout}{error.stderr}"
        ) from None
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{label} did not finish within 60 seconds.") from None


def smoke(binary):
    with tempfile.TemporaryDirectory(prefix="uv-bazel-smoke-") as temporary:
        directory = Path(temporary)
        environment = isolated_environment(directory)
        command = [str(binary), "--no-config", "--offline", "--no-python-downloads"]
        version = run_command(
            "version", [*command, "--version"], directory, environment
        )
        if not version.startswith("uv "):
            raise RuntimeError("The supplied executable did not identify itself as uv.")
        run_command("help", [*command, "--help"], directory, environment)

        wheelhouse = directory / "wheelhouse"
        wheelhouse.mkdir()
        write_wheel(wheelhouse, "uv-bazel-smoke-leaf", "VALUE = 41\n")
        write_wheel(
            wheelhouse,
            "uv-bazel-smoke-root",
            "from uv_bazel_smoke_leaf import VALUE as LEAF_VALUE\nVALUE = LEAF_VALUE + 1\n",
            requires="uv-bazel-smoke-leaf",
        )
        virtualenv = directory / "venv"
        run_command(
            "virtualenv creation",
            [*command, "venv", "--python", sys.executable, str(virtualenv)],
            directory,
            environment,
        )
        python = virtualenv / (
            "Scripts/python.exe" if os.name == "nt" else "bin/python"
        )
        run_command(
            "offline transitive installation",
            [
                *command,
                "pip",
                "install",
                "--python",
                str(python),
                "--no-index",
                "--find-links",
                str(wheelhouse),
                "--link-mode",
                "copy",
                f"uv-bazel-smoke-root=={VERSION}",
            ],
            directory,
            environment,
        )
        run_command(
            "installed package imports",
            [
                str(python),
                "-I",
                "-c",
                "import uv_bazel_smoke_root, uv_bazel_smoke_leaf\n"
                "values = (uv_bazel_smoke_root.VALUE, uv_bazel_smoke_leaf.VALUE)\n"
                "if values != (42, 41):\n"
                "    raise SystemExit(f'Unexpected imported values: {values}')\n",
            ],
            directory,
            environment,
        )
        print(version.strip())
    print(
        "Offline smoke passed: version/help, venv, two local wheels, imports (42, 41)."
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, required=True, help="Native uv executable"
    )
    args = parser.parse_args()
    try:
        binary = args.binary.expanduser().resolve(strict=True)
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError("--binary must name an executable file.")
        smoke(binary)
    except (OSError, RuntimeError, ValueError) as error:
        print(f"Smoke test failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
