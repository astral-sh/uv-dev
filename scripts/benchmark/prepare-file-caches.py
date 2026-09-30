"""Prepare real content-addressed wheel caches for pruning benchmarks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import tempfile
from collections import defaultdict
from pathlib import Path

PYTHON = "3.12.11"
AIRFLOW_REQUIREMENTS_SHA256 = (
    "afd8872484cca2a0d5a86b1b6831136f07c2b79050b203885a9c832082c2ed6d"
)


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def hardlink_groups(cache: Path) -> list[list[str]]:
    groups = defaultdict(list)
    for directory, names, files in os.walk(cache, followlinks=False):
        for name in names + files:
            path = Path(directory) / name
            metadata = path.lstat()
            if stat.S_ISREG(metadata.st_mode) and metadata.st_nlink > 1:
                groups[metadata.st_dev, metadata.st_ino].append(
                    path.relative_to(cache).as_posix()
                )
    return sorted(sorted(paths) for paths in groups.values() if len(paths) > 1)


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-file-caches"
    )
    args = parser.parse_args()
    directory = args.directory.resolve()
    binary = args.uv.resolve()
    fixtures = root / ".cache/bench-fixtures"
    locks = root / "scripts/benchmark/file-cache-locks"
    airflow = locks / "airflow-multicloud.txt"
    if digest(airflow) != AIRFLOW_REQUIREMENTS_SHA256:
        raise ValueError("The pinned Airflow requirements changed")
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment["UV_PYTHON_INSTALL_DIR"] = str(root / ".cache/bench-python")
    command = [str(binary), "--no-config"]
    subprocess.run(
        [
            *command,
            "--cache-dir",
            str(root / ".cache"),
            "python",
            "install",
            "--no-bin",
            "--no-registry",
            PYTHON,
        ],
        env=environment,
        check=True,
    )
    environment["UV_PYTHON_DOWNLOADS"] = "never"
    temporary_root = root / "target"
    temporary_root.mkdir(exist_ok=True)
    for name in ("packse", "airflow_multicloud"):
        destination = directory / name
        if destination.exists():
            shutil.rmtree(destination)
        destination.mkdir(parents=True)
        cache = destination / "cache"
        requirements = destination / "requirements.txt"
        prefix = [
            *command,
            "--cache-dir",
            str(cache),
            "--preview-features",
            "content-addressed-cache",
        ]
        with tempfile.TemporaryDirectory(
            prefix="bench-file-cache-", dir=temporary_root
        ) as temporary:
            project = Path(temporary)
            if name == "packse":
                for source, target in (
                    ("packse.pyproject.toml", "pyproject.toml"),
                    ("packse.lock", "uv.lock"),
                ):
                    shutil.copyfile(fixtures / source, project / target)
                subprocess.run(
                    [
                        *prefix,
                        "--project",
                        str(project),
                        "export",
                        "--frozen",
                        "--no-default-groups",
                        "--no-emit-project",
                        "--format",
                        "requirements-txt",
                        "--output-file",
                        str(requirements),
                    ],
                    env=environment,
                    check=True,
                    stdout=subprocess.DEVNULL,
                )
            else:
                shutil.copyfile(airflow, requirements)
            python = project / ".venv"
            subprocess.run(
                [*prefix, "venv", "--managed-python", "--python", PYTHON, str(python)],
                env=environment,
                check=True,
            )
            subprocess.run(
                [
                    *prefix,
                    "pip",
                    "sync",
                    "--python",
                    str(python),
                    "--default-index",
                    "https://pypi.org/simple",
                    "--only-binary",
                    ":all:",
                    "--no-binary",
                    "dill",
                    "--build-constraints",
                    str(locks / "build-constraints.txt"),
                    "--require-hashes",
                    "--link-mode",
                    "hardlink",
                    str(requirements),
                ],
                env=environment,
                check=True,
            )
            installed = json.loads(
                subprocess.check_output(
                    [
                        *prefix,
                        "pip",
                        "list",
                        "--python",
                        str(python),
                        "--format",
                        "json",
                    ],
                    env=environment,
                )
            )
        objects = [path for path in (cache / "files-v0").rglob("*") if path.is_file()]
        groups = hardlink_groups(cache)
        if not objects or not groups:
            raise ValueError(f"The {name} cache has no shared file objects")
        manifest = {
            "name": name,
            "python": PYTHON,
            "requirements-sha256": digest(requirements),
            "installed": installed,
            "hardlinks": groups,
            "file-objects": len(objects),
            "file-object-bytes": sum(path.stat().st_size for path in objects),
        }
        (destination / "manifest.json").write_text(json.dumps(manifest) + "\n")
        print(
            f"{name}: {len(installed)} packages, {len(objects)} file objects, "
            f"{manifest['file-object-bytes']:,} bytes"
        )


if __name__ == "__main__":
    main()
