#!/usr/bin/env python3
"""Measure actual uv metadata hook execution using dependency-free synthetic backends."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile
from email import message_from_bytes
from pathlib import Path

BACKEND = """import json
import os
from pathlib import Path
import zipfile

def record(hook):
    with open(os.environ["RESEARCH_HOOK_LOG"], "a") as stream:
        stream.write(json.dumps({"hook": hook}) + "\\n")

record("import")
METADATA = "Metadata-Version: 2.2\\nName: metadata-fixture\\nVersion: 1.0\\nRequires-Dist: metadata-dep==1.0\\n"

WHEEL_METADATA = METADATA

def get_requires_for_build_wheel(config_settings=None):
    record("get_requires_for_build_wheel")
    return []

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    record("prepare_metadata_for_build_wheel")
    target = Path(metadata_directory) / "metadata_fixture-1.0.dist-info"
    target.mkdir()
    (target / "METADATA").write_text(METADATA)
    return target.name

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    record("build_wheel")
    filename = "metadata_fixture-1.0-py3-none-any.whl"
    prefix = "metadata_fixture-1.0.dist-info/"
    with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
        wheel.writestr(prefix + "METADATA", WHEEL_METADATA)
        wheel.writestr(prefix + "WHEEL", "Wheel-Version: 1.0\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n")
        wheel.writestr(prefix + "RECORD", "")
    return filename
"""

BUILD_SYSTEM = """[build-system]
requires = []
build-backend = "backend"
backend-path = ["."]
"""


def create_case(directory: Path, case: str) -> Path:
    directory.mkdir()
    backend = BACKEND
    if case == "wheel-fallback":
        backend += "\ndel prepare_metadata_for_build_wheel\n"
    if case.endswith("disagreement"):
        backend += '\nWHEEL_METADATA = METADATA.replace("metadata-dep==1.0", "metadata-dep==2.0")\n'
    project = BUILD_SYSTEM
    if case in {"static-project", "static-disagreement"}:
        project += '\n[project]\nname = "metadata-fixture"\nversion = "1.0"\ndependencies = ["metadata-dep==1.0"]\n'
    elif case != "static-sdist":
        project += '\n[project]\nname = "metadata-fixture"\nversion = "1.0"\ndynamic = ["dependencies"]\n'
    files = {"pyproject.toml": project, "backend.py": backend}
    if case == "static-sdist":
        files["PKG-INFO"] = (
            "Metadata-Version: 2.2\nName: metadata-fixture\nVersion: 1.0\n"
            "Requires-Dist: metadata-dep==1.0\n"
        )
        archive = directory / "metadata_fixture-1.0.tar.gz"
        with tarfile.open(archive, "w:gz") as stream:
            for name, content in files.items():
                data = content.encode()
                entry = tarfile.TarInfo(f"metadata_fixture-1.0/{name}")
                entry.size = len(data)
                stream.addfile(entry, io.BytesIO(data))
        return archive
    for name, content in files.items():
        (directory / name).write_text(content)
    return directory


def collect(
    uv: str, workspace: Path, check: bool, observations: list, builds: list
) -> None:
    wheelhouse = workspace / "wheels"
    wheelhouse.mkdir()
    with zipfile.ZipFile(
        wheelhouse / "metadata_dep-1.0-py3-none-any.whl", "w"
    ) as wheel:
        prefix = "metadata_dep-1.0.dist-info/"
        wheel.writestr(
            prefix + "METADATA",
            "Metadata-Version: 2.2\nName: metadata-dep\nVersion: 1.0\n",
        )
        wheel.writestr(
            prefix + "WHEEL",
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        )
        wheel.writestr(prefix + "RECORD", "")
    for case in (
        "static-project",
        "static-sdist",
        "metadata-hook",
        "wheel-fallback",
        "static-disagreement",
        "hook-disagreement",
    ):
        source = create_case(workspace / case, case)
        requirements = workspace / f"{case}.in"
        requirements.write_text(f"metadata-fixture @ {source.as_uri()}\n")
        log = workspace / f"{case}.jsonl"
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("UV_")
        }
        environment["RESEARCH_HOOK_LOG"] = str(log)
        for cache_state in ("cold", "warm"):
            log.write_text("")
            started = time.perf_counter()
            process = subprocess.run(
                [
                    uv,
                    "--no-config",
                    "pip",
                    "compile",
                    str(requirements),
                    "--no-index",
                    "--find-links",
                    str(wheelhouse),
                    "--python",
                    sys.executable,
                    "--cache-dir",
                    str(workspace / f"cache-{case}"),
                    "--no-header",
                    "--no-annotate",
                ],
                env=environment,
                text=True,
                capture_output=True,
                check=False,
                timeout=60,
            )
            elapsed = time.perf_counter() - started
            hooks = [json.loads(line)["hook"] for line in log.read_text().splitlines()]
            dependencies = [
                line
                for line in process.stdout.splitlines()
                if line.startswith("metadata-dep==")
            ]
            observation = {
                "case": case,
                "cache": cache_state,
                "seconds": elapsed,
                "returncode": process.returncode,
                "hooks": hooks,
                "dependencies": dependencies,
                "stderr": process.stderr,
            }
            observations.append(observation)
            if process.returncode:
                raise RuntimeError(json.dumps(observation, indent=2))
            if check:
                if dependencies != ["metadata-dep==1.0"]:
                    raise AssertionError(
                        f"Dependency metadata disagrees: {observation}"
                    )
                if case.startswith("static-") and hooks:
                    raise AssertionError(
                        f"Static metadata executed backend: {observation}"
                    )
                if (
                    cache_state == "cold"
                    and case in {"metadata-hook", "hook-disagreement"}
                    and (
                        "prepare_metadata_for_build_wheel" not in hooks
                        or "build_wheel" in hooks
                    )
                ):
                    raise AssertionError(f"Expected metadata hook: {observation}")
                if (
                    cache_state == "cold"
                    and case == "wheel-fallback"
                    and "build_wheel" not in hooks
                ):
                    raise AssertionError(f"Expected wheel fallback: {observation}")
        log.write_text("")
        output = workspace / f"wheel-{case}"
        started = time.perf_counter()
        build = subprocess.run(
            [
                uv,
                "--no-config",
                "build",
                str(source),
                "--wheel",
                "--no-index",
                "--python",
                sys.executable,
                "--cache-dir",
                str(workspace / f"build-cache-{case}"),
                "--out-dir",
                str(output),
            ],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        seconds = time.perf_counter() - started
        build_observation = {
            "case": case,
            "seconds": seconds,
            "returncode": build.returncode,
            "stderr": build.stderr,
            "hooks": [
                json.loads(line)["hook"] for line in log.read_text().splitlines()
            ],
        }
        builds.append(build_observation)
        if build.returncode:
            raise RuntimeError(build.stderr)
        wheels = list(output.glob("*.whl"))
        if len(wheels) != 1:
            raise AssertionError(f"Expected exactly one built wheel: {wheels}")
        with zipfile.ZipFile(wheels[0]) as wheel:
            metadata = message_from_bytes(
                wheel.read("metadata_fixture-1.0.dist-info/METADATA")
            )
        wheel_dependencies = metadata.get_all("Requires-Dist", [])
        resolution_dependencies = observations[-1]["dependencies"]
        agreement = wheel_dependencies == resolution_dependencies
        build_observation.update(
            {
                "wheel_dependencies": wheel_dependencies,
                "resolution_dependencies": resolution_dependencies,
                "agreement": agreement,
            }
        )
        if check and agreement == case.endswith("disagreement"):
            raise AssertionError(f"Unexpected resolver/wheel comparison: {builds[-1]}")


def run(uv: str, workspace: Path, check: bool) -> dict:
    with open(uv, "rb") as binary:
        uv_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    report = {
        "uv_sha256": uv_sha256,
        "uv_executable": str(Path(uv).resolve()),
        "uv": subprocess.check_output([uv, "--version"], text=True).strip(),
        "python": sys.version,
        "observations": [],
        "builds": [],
    }
    try:
        collect(uv, workspace, check, report["observations"], report["builds"])
    except (
        AssertionError,
        OSError,
        RuntimeError,
        subprocess.SubprocessError,
        ValueError,
        zipfile.BadZipFile,
    ) as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True, help="uv executable to measure")
    parser.add_argument(
        "--work-dir",
        type=Path,
        required=True,
        help="Parent for disposable experiment files",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Assert expected synthetic fixture behavior",
    )
    args = parser.parse_args()
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="metadata-hooks-", dir=args.work_dir
    ) as directory:
        report = run(str(Path(uv).resolve()), Path(directory).resolve(), args.check)
        print(json.dumps(report, indent=2))
        if "error" in report:
            raise SystemExit(1)


if __name__ == "__main__":
    main()
