"""Prime pinned, held-out ecosystem project resolutions for release benchmarks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import tomllib
import urllib.request
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / ".cache/bench-release/pgo/uv")
    args = parser.parse_args()
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment.update(
        UV_PYTHON_INSTALL_DIR=str(root / ".cache/bench-python"),
        UV_PYTHON_DOWNLOADS="never",
    )
    for workload in json.loads(
        Path(__file__).with_name("release-workloads.json").read_text()
    ):
        directory = root / ".cache/bench-release-workloads" / workload["name"]
        project = directory / "project"
        project.mkdir(parents=True, exist_ok=True)
        manifest = project / "pyproject.toml"
        if (
            not manifest.is_file()
            or hashlib.sha256(manifest.read_bytes()).hexdigest() != workload["sha256"]
        ):
            with urllib.request.urlopen(workload["url"], timeout=120) as response:
                content = response.read()
            if hashlib.sha256(content).hexdigest() != workload["sha256"]:
                raise ValueError(f"Unexpected project manifest: {workload['name']}")
            manifest.write_bytes(content)
        command = [
            str(args.uv.resolve()),
            "--no-config",
            "--no-progress",
            "--cache-dir",
            str(directory / "cache"),
        ]
        lock = [
            "lock",
            "--managed-python",
            "--python",
            "3.11.13",
            "--exclude-newer",
            workload["exclude_newer"],
        ]
        (project / "uv.lock").unlink(missing_ok=True)
        subprocess.run(
            [*command, "--project", str(project), *lock], env=environment, check=True
        )
        # A different project path verifies that the prepared metadata is relocatable.
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            copied = Path(temporary)
            shutil.copyfile(manifest, copied / "pyproject.toml")
            subprocess.run(
                [*command, "--offline", "--project", str(copied), *lock],
                env=environment,
                check=True,
            )
            package_count = len(
                tomllib.loads((copied / "uv.lock").read_text())["package"]
            )
        (directory / "metadata.json").write_text(
            json.dumps(workload | {"packages": package_count}, indent=2) + "\n"
        )
        print(
            f"Prepared {workload['name']}: {package_count} locked packages", flush=True
        )


if __name__ == "__main__":
    main()
