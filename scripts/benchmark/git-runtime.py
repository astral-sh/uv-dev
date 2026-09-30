"""Build and run a pinned Git runtime for local-transport benchmarks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

VERSION = "2.55.0"
COMMIT = "e9019fcafe0040228b8631c30f97ae1adb61bcdc"
SHA256 = "262c0e5f0f082c8c05e8192040a786ec0c8cafeeeaf8b7fa2a16c17be2deefa2"
BUILD_OPTIONS = [
    "RUNTIME_PREFIX=YesPlease",
    "NO_RUST=YesPlease",
    "NO_CURL=YesPlease",
    "NO_EXPAT=YesPlease",
    "NO_OPENSSL=YesPlease",
    "NO_GETTEXT=YesPlease",
    "NO_TCLTK=YesPlease",
    "NO_PERL=YesPlease",
    f"GIT_BUILT_FROM_COMMIT={COMMIT}",
]


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def environment(directory: Path) -> dict[str, str]:
    return {
        **os.environ,
        "PATH": str(directory / "bin") + os.pathsep + os.environ.get("PATH", ""),
        "GIT_EXEC_PATH": str(directory / "libexec/git-core"),
    }


def validate(directory: Path) -> bool:
    manifest = directory / "manifest.json"
    binary = directory / "bin/git"
    source = directory / "source.tar.gz"
    if not manifest.is_file() or not binary.is_file() or not source.is_file():
        return False
    if digest(source) != SHA256:
        return False
    expected = {
        "version": VERSION,
        "commit": COMMIT,
        "source_sha256": SHA256,
        "build_options": BUILD_OPTIONS,
        "binary_sha256": digest(binary),
    }
    if json.loads(manifest.read_text()) != expected:
        return False
    version = subprocess.check_output(
        [str(binary), "--version"], env=environment(directory), text=True
    ).strip()
    return version == f"git version {VERSION}"


def prepare(directory: Path, jobs: int) -> None:
    if validate(directory):
        return
    directory.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory.parent) as temporary:
        temporary = Path(temporary)
        archive = temporary / "git.tar.gz"
        with (
            urllib.request.urlopen(
                f"https://codeload.github.com/git/git/tar.gz/{COMMIT}", timeout=120
            ) as response,
            archive.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)
        if digest(archive) != SHA256:
            raise ValueError("Git source archive failed SHA-256 verification")
        with tarfile.open(archive) as source:
            source.extractall(temporary, filter="data")
        install = temporary / "install"
        # Only the file transport is measured. Excluding optional network and UI
        # dependencies keeps this runtime portable between Linux walltime hosts.
        subprocess.run(
            [
                "make",
                f"-j{jobs}",
                f"prefix={install}",
                *BUILD_OPTIONS,
                "install",
            ],
            cwd=temporary / f"git-{COMMIT}",
            check=True,
        )
        manifest = {
            "version": VERSION,
            "commit": COMMIT,
            "source_sha256": SHA256,
            "build_options": BUILD_OPTIONS,
            "binary_sha256": digest(install / "bin/git"),
        }
        archive.rename(install / "source.tar.gz")
        shutil.copyfile(temporary / f"git-{COMMIT}/COPYING", install / "COPYING")
        (install / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        if directory.exists():
            shutil.rmtree(directory)
        install.rename(directory)
    if not validate(directory):
        raise ValueError("Installed Git runtime did not match its manifest")
    print(f"Prepared Git {VERSION} at {directory}")


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-git-runtime"
    )
    commands = parser.add_subparsers(dest="action", required=True)
    build = commands.add_parser("prepare")
    build.add_argument("--jobs", type=int, default=4)
    run = commands.add_parser("run")
    run.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    directory = args.directory.resolve()
    if args.action == "prepare":
        if sys.version_info < (3, 12, 11):
            parser.error("prepare requires Python 3.12.11 or newer")
        if args.jobs < 1:
            parser.error("--jobs must be positive")
        prepare(directory, args.jobs)
        return
    if not validate(directory):
        parser.error("Run git-runtime.py prepare before using the Git runtime")
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("run requires a command")
    subprocess.run(command, env=environment(directory), check=True)


if __name__ == "__main__":
    main()
