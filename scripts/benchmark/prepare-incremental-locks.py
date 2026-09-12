"""Prime offline relocking workloads from real published application requirements."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

INITIAL_CUTOFF = "2025-10-01T00:00:00Z"
UPDATED_CUTOFF = "2026-01-01T00:00:00Z"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    parser.add_argument("--refresh-locks", action="store_true")
    args = parser.parse_args()
    fixtures = root / "scripts/benchmark/incremental-locks"
    fixtures.mkdir(exist_ok=True)
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment["UV_PYTHON_INSTALL_DIR"] = str(root / ".cache/bench-python")
    environment["UV_PYTHON_DOWNLOADS"] = "never"
    for workload in json.loads(
        Path(__file__).with_name("incremental-locks.json").read_text()
    ):
        directory = root / ".cache/bench-incremental-locks" / workload["name"]
        project = directory / "project"
        project.mkdir(parents=True, exist_ok=True)
        manifest = (
            f"[project]\nname = '{workload['name']}-consumer'\nversion = '0.1.0'\n"
            f"requires-python = '==3.11.*'\ndependencies = [{json.dumps(workload['requirement'])}]\n"
        )
        (project / "pyproject.toml").write_text(manifest)
        command = [
            str(args.uv.resolve()),
            "--no-config",
            "--no-progress",
            "--cache-dir",
            str(directory / "cache"),
            "--project",
            str(project),
            "lock",
            "--managed-python",
            "--python",
            "3.11.13",
        ]
        locked = fixtures / f"{workload['name']}.lock"
        # A clean solve primes the metadata needed when the previous lock's preferences are
        # discarded. The captured initial lock remains the benchmark's fixed input.
        (project / "uv.lock").unlink(missing_ok=True)
        subprocess.run(
            [*command, "--exclude-newer", INITIAL_CUTOFF], env=environment, check=True
        )
        if args.refresh_locks:
            shutil.copyfile(project / "uv.lock", locked)
        if not locked.is_file():
            raise FileNotFoundError(f"Missing {locked}; run with --refresh-locks")
        shutil.copyfile(locked, project / "uv.lock")
        subprocess.run(
            [*command, "--exclude-newer", UPDATED_CUTOFF], env=environment, check=True
        )
        (project / "uv.lock").unlink()
        subprocess.run(
            [*command, "--exclude-newer", UPDATED_CUTOFF], env=environment, check=True
        )
        shutil.copyfile(locked, project / "uv.lock")


if __name__ == "__main__":
    main()
