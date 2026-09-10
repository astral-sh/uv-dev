# /// script
# requires-python = ">=3.12"
# dependencies = ["chevron-blue==0.3.0"]
# ///
"""Run a native bootstrap installer against an actual uv distribution archive.

Only the network transport is replaced with a local archive copy. Target selection,
the embedded checksum, extraction, native installation, and uninstallation all run
normally. No shell profiles or global installation receipts are modified.
"""

import argparse
import importlib.util
import os
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "native_installers", ROOT / "scripts/generate-native-installers.py"
)
assert SPEC and SPEC.loader
INSTALLERS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLERS)


def create_archive(directory: Path, target: str, destination: Path) -> Path:
    """Package already-built binaries in the published archive layout."""
    if target.endswith("-pc-windows-msvc"):
        archive = destination / f"uv-{target}.zip"
        with ZipFile(archive, "w", compression=ZIP_DEFLATED) as output:
            for name in ("uv.exe", "uvx.exe", "uvw.exe"):
                output.write(directory / name, name)
    else:
        archive = destination / f"uv-{target}.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            for name in ("uv", "uvx"):
                output.add(directory / name, f"uv-{target}/{name}", recursive=False)
    return archive


def check(archive: Path, installer: Path | None, version: str) -> None:
    """Install the archive in an isolated directory, then remove that installation."""
    filename = archive.name
    target = filename.removeprefix("uv-").removesuffix(".tar.gz").removesuffix(".zip")
    windows = target.endswith("-pc-windows-msvc")
    with tempfile.TemporaryDirectory(prefix="uv-bootstrap-check-") as temporary:
        root = Path(temporary)
        if installer is None:
            name = "uv-installer.ps1" if windows else "uv-installer.sh"
            installer = root / name
            installer.write_text(
                INSTALLERS.render_installers(
                    version, "astral-sh/uv", {target: INSTALLERS.digest(archive)}
                )[name],
                encoding="utf-8",
            )
        destination = root / "bin"
        env = dict(os.environ)
        for name in (
            "UV_UNMANAGED_INSTALL",
            "UV_DISABLE_UPDATE",
            "UV_INSTALLER_GITHUB_BASE_URL",
            "UV_INSTALLER_GHE_BASE_URL",
            "UV_GITHUB_TOKEN",
            "UV_PREVIEW_FEATURES",
            "CARGO_DIST_FORCE_INSTALL_DIR",
        ):
            env.pop(name, None)
        env.update(
            {
                "UV_DOWNLOAD_URL": "https://example.invalid/releases",
                "UV_INSTALL_DIR": str(destination),
                "UV_NO_MODIFY_PATH": "1",
                "UV_NO_CONFIG": "1",
                "UV_CACHE_DIR": str(root / "cache"),
                "AXOUPDATER_CONFIG_PATH": str(root / "legacy"),
                "TMPDIR": str(root),
                "UV_TEST_ARCHIVE": str(archive.resolve()),
                "UV_TEST_INSTALLER": str(installer.resolve()),
            }
        )
        if windows:
            shell = shutil.which("pwsh") or shutil.which("powershell")
            if not shell:
                raise RuntimeError("PowerShell is required")
            wrapper = root / "run.ps1"
            wrapper.write_text(
                "$ErrorActionPreference = 'Stop'\nfunction Invoke-WebRequest { param([switch]$UseBasicParsing, $Uri, $Headers, $OutFile) Copy-Item -LiteralPath $env:UV_TEST_ARCHIVE -Destination $OutFile }\n& $env:UV_TEST_INSTALLER\n",
                encoding="utf-8",
            )
            subprocess.run(
                [
                    shell,
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(wrapper),
                ],
                env=env,
                check=True,
            )
        else:
            tools = root / "transport"
            tools.mkdir()
            curl = tools / "curl"
            curl.write_text(
                '#!/bin/sh\nset -eu\nwhile [ "$1" != -o ]; do shift; done\ncp "$UV_TEST_ARCHIVE" "$2"\n',
                encoding="utf-8",
            )
            curl.chmod(0o755)
            env["PATH"] = f"{tools}{os.pathsep}{env.get('PATH', os.defpath)}"
            subprocess.run(["sh", str(installer)], env=env, check=True)
        executable = destination / ("uv.exe" if windows else "uv")
        output = subprocess.check_output(
            [str(executable), "--version"], env=env, text=True
        )
        fields = output.split()
        if (
            len(fields) < 2
            or fields[0] != "uv"
            or fields[1].partition("+")[0] != version
        ):
            raise ValueError(f"Unexpected installed version: {output!r}")
        for name in ("uvx.exe", "uvw.exe") if windows else ("uvx",):
            if not (destination / name).is_file():
                raise ValueError(f"Missing installed companion: {name}")
        subprocess.run(
            [str(executable), "self", "uninstall", "--dry-run"], env=env, check=True
        )
        subprocess.run([str(executable), "self", "uninstall"], env=env, check=True)
        for _ in range(100):
            if not executable.exists():
                break
            time.sleep(0.05)
        if executable.exists() or (destination / ".uv-receipt.json").exists():
            raise ValueError("Uninstall left the managed executable or receipt behind")


def check_legacy_downgrade(directory: Path, version: str) -> None:
    """Exercise a published pre-native release and its legacy self-update receipt."""
    name = "uv.exe" if os.name == "nt" else "uv"
    current = (directory / name).resolve()
    with tempfile.TemporaryDirectory(prefix="uv-downgrade-check-") as temporary:
        root = Path(temporary)
        destination = root / "bin"
        legacy = root / "legacy"
        env = dict(os.environ)
        for variable in (
            "UV_UNMANAGED_INSTALL",
            "UV_DISABLE_UPDATE",
            "UV_DOWNLOAD_URL",
            "INSTALLER_DOWNLOAD_URL",
            "UV_INSTALLER_GITHUB_BASE_URL",
            "UV_INSTALLER_GHE_BASE_URL",
            "UV_GITHUB_TOKEN",
            "AXOUPDATER_CONFIG_WORKING_DIR",
            "CARGO_DIST_FORCE_INSTALL_DIR",
        ):
            env.pop(variable, None)
        env.update(
            {
                "UV_NO_CONFIG": "1",
                "UV_NO_MODIFY_PATH": "1",
                "UV_CACHE_DIR": str(root / "cache"),
                "AXOUPDATER_CONFIG_PATH": str(legacy),
            }
        )
        install = [
            str(current),
            "self",
            "install",
            "--source-repository",
            "astral-sh/uv",
            "--install-dir",
            str(destination),
        ]
        subprocess.run(install, env=env, check=True)
        installed = destination / name
        subprocess.run([str(installed), "self", "update", version], env=env, check=True)
        if not (legacy / "uv-receipt.json").is_file():
            raise ValueError("Downgrade did not create a legacy receipt")
        subprocess.run(
            [str(installed), "self", "update", version, "--dry-run"],
            env=env,
            check=True,
        )
        subprocess.run(install, env=env, check=True)
        subprocess.run([str(installed), "self", "uninstall"], env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--archive", type=Path)
    source.add_argument("--binary-directory", type=Path)
    parser.add_argument("--target")
    parser.add_argument("--installer", type=Path)
    parser.add_argument(
        "--downgrade-version", help="Also exercise this published pre-native release"
    )
    parser.add_argument(
        "--version",
        default=tomllib.loads((ROOT / "crates/uv/Cargo.toml").read_text())["package"][
            "version"
        ],
    )
    args = parser.parse_args()
    if args.downgrade_version and not args.binary_directory:
        parser.error("--downgrade-version requires --binary-directory")
    if args.archive:
        check(args.archive, args.installer, args.version)
    else:
        if not args.target:
            parser.error("--target is required with --binary-directory")
        with tempfile.TemporaryDirectory(prefix="uv-bootstrap-archive-") as temporary:
            check(
                create_archive(args.binary_directory, args.target, Path(temporary)),
                args.installer,
                args.version,
            )
        if args.downgrade_version:
            check_legacy_downgrade(args.binary_directory, args.downgrade_version)


if __name__ == "__main__":
    main()
