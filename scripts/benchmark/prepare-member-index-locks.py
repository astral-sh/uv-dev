"""Prime real application graphs whose local package defines an explicit index."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

INDEX = "https://pypi.org/simple/?uv-benchmark=member-index"
CUTOFF = "2025-10-01T00:00:00Z"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    parser.add_argument("--refresh-locks", action="store_true")
    args = parser.parse_args()
    fixtures = root / "scripts/benchmark/member-index-locks"
    fixtures.mkdir(exist_ok=True)
    prepared = root / ".cache/bench-member-index-locks"
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment["UV_PYTHON_INSTALL_DIR"] = str(root / ".cache/bench-python")
    environment["UV_PYTHON_DOWNLOADS"] = "never"
    workloads = json.loads(
        Path(__file__).with_name("incremental-locks.json").read_text()
    )
    for workload in workloads:
        package = workload["requirement"].partition("==")[0].partition("[")[0]
        member = f"{workload['name']}-application"
        for layout in ("path", "workspace"):
            project = prepared / workload["name"] / layout
            (project / "application").mkdir(parents=True, exist_ok=True)
            source = (
                "{ workspace = true }"
                if layout == "workspace"
                else "{ path = 'application' }"
            )
            workspace = (
                "\n[tool.uv.workspace]\nmembers = ['application']\n"
                if layout == "workspace"
                else ""
            )
            (project / "pyproject.toml").write_text(
                f"[project]\nname = '{workload['name']}-consumer'\nversion = '0.1.0'\n"
                f"requires-python = '==3.11.*'\ndependencies = [{json.dumps(member)}]\n"
                f"[tool.uv.sources]\n{json.dumps(member)} = {source}\n{workspace}"
            )
            (project / "application/pyproject.toml").write_text(
                f"[project]\nname = {json.dumps(member)}\nversion = '0.1.0'\n"
                f"requires-python = '==3.11.*'\ndependencies = [{json.dumps(workload['requirement'])}]\n"
                f"[tool.uv.sources]\n{json.dumps(package)} = {{ index = 'member' }}\n"
                f"[[tool.uv.index]]\nname = 'member'\nurl = {json.dumps(INDEX)}\nexplicit = true\n"
            )
            command = [
                str(args.uv.resolve()),
                "--no-config",
                "--no-progress",
                "--cache-dir",
                str(prepared / "cache"),
                "--project",
                str(project),
                "lock",
                "--managed-python",
                "--python",
                "3.11.13",
                "--exclude-newer",
                CUTOFF,
            ]
            locked = fixtures / f"{workload['name']}-{layout}.lock"
            (project / "uv.lock").unlink(missing_ok=True)
            subprocess.run(command, env=environment, check=True)
            if args.refresh_locks:
                shutil.copyfile(project / "uv.lock", locked)
            if not locked.is_file():
                raise FileNotFoundError(f"Missing {locked}; run with --refresh-locks")
            shutil.copyfile(locked, project / "uv.lock")
            subprocess.run(
                [*command, "--offline", "--locked"], env=environment, check=True
            )


if __name__ == "__main__":
    main()
