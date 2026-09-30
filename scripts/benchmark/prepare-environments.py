"""Prime the pinned Python and package cache used by installed-environment benchmarks."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

PYTHON = "3.11.13"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    parser.add_argument("--discovery", action="store_true")
    parser.add_argument("--project-caches", action="store_true")
    args = parser.parse_args()
    cache = root / ".cache"
    fixtures = cache / "bench-fixtures"
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment["UV_PYTHON_INSTALL_DIR"] = str(cache / "bench-python")
    command = [str(args.uv.resolve()), "--no-config"]
    workloads = json.loads((root / "scripts/benchmark/environments.json").read_text())
    versions = sorted({PYTHON, *(workload["python"] for workload in workloads)})
    if args.discovery:
        versions = sorted({*versions, "3.10.18", "3.13.4"})
    subprocess.run(
        [
            *command,
            "--cache-dir",
            str(cache),
            "python",
            "install",
            "--no-bin",
            "--no-registry",
            *versions,
        ],
        env=environment,
        check=True,
    )
    environment["UV_PYTHON_DOWNLOADS"] = "never"
    temporary_root = root / "target"
    temporary_root.mkdir(exist_ok=True)
    selected = [(workload, cache) for workload in workloads]
    selected.append(({"project": "prefect", "python": PYTHON, "sync-args": []}, cache))
    shared_cache = cache / "bench-caches" / "shared"
    package_sets = {}
    if args.project_caches:
        if shared_cache.exists():
            shutil.rmtree(shared_cache)
        for workload in workloads:
            project_cache = cache / "bench-caches" / workload["name"]
            if project_cache.exists():
                shutil.rmtree(project_cache)
            selected.append((workload, project_cache))
            selected.append((workload, shared_cache))
    for workload, project_cache in selected:
        with tempfile.TemporaryDirectory(
            prefix="bench-project-", dir=temporary_root
        ) as temporary:
            project = Path(temporary)
            name = workload["project"]
            shutil.copyfile(
                fixtures / f"{name}.pyproject.toml", project / "pyproject.toml"
            )
            shutil.copyfile(fixtures / f"{name}.lock", project / "uv.lock")
            subprocess.run(
                [
                    *command,
                    "--cache-dir",
                    str(project_cache),
                    "--project",
                    str(project),
                    "sync",
                    "--frozen",
                    "--no-default-groups",
                    "--no-install-project",
                    "--managed-python",
                    "--python",
                    workload["python"],
                    *workload["sync-args"],
                ],
                env=environment,
                check=True,
            )
            if project_cache == shared_cache:
                installed = subprocess.run(
                    [
                        *command,
                        "--cache-dir",
                        str(project_cache),
                        "pip",
                        "list",
                        "--python",
                        str(project / ".venv"),
                        "--format",
                        "json",
                    ],
                    env=environment,
                    check=True,
                    capture_output=True,
                    text=True,
                )
                package_sets[workload["name"]] = sorted(
                    package["name"] for package in json.loads(installed.stdout)
                )
    if args.project_caches:
        (fixtures / "environment-packages.json").write_text(
            json.dumps(package_sets, indent=2) + "\n"
        )


if __name__ == "__main__":
    main()
