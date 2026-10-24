"""Create a read-mostly job venv from a prepared, relocatable `uv` environment on POSIX.

Install the seed with `--link-mode copy` and keep it for the lifetime of
all jobs. Use an absolute seed path in the following commands.

    uv venv --relocatable /path/to/seed
    UV_PROJECT_ENVIRONMENT=/path/to/seed uv sync --locked --no-editable --link-mode copy
    python3 project_environment.py /path/to/seed /path/to/job
    /path/to/job/bin/python -IB -c 'import sys; print(sys.prefix)'
    uv pip list --python /path/to/job/bin/python

Package links permit writes through to the seed. Use trusted, read-only workloads
and disable bytecode writes or give Python a private `-X pycache_prefix`.

Pass `--stable-imports` and reuse a trusted `-X pycache_prefix` across jobs to reuse
bytecode for an identical seed. Python then imports from the seed's `site-packages`.
Job-local additions to `site-packages` are outside this mode's import path.
"""

import argparse
import os
import shutil
from pathlib import Path


def project(seed: Path, job: Path, *, stable_imports: bool = False) -> None:
    seed = seed.resolve(strict=True)
    job = job.parent.resolve(strict=True) / job.name
    if os.name != "posix" or job.is_relative_to(seed):
        raise ValueError("Choose a POSIX destination outside the seed")
    if "relocatable = true" not in (seed / "pyvenv.cfg").read_text().splitlines():
        raise ValueError("Create the seed with 'uv venv --relocatable'")
    sites = list((seed / "lib").glob("python*/site-packages"))
    if len(sites) != 1 or not sites[0].is_dir() or sites[0].is_symlink():
        raise ValueError("Expected one installed site-packages directory under lib/")
    (site,) = sites

    # Copy launchers so their relative shebangs select the job's interpreter.
    # Each job acquires its own environment lock.
    def skip(directory: str, _names: list[str]) -> set[str]:
        if Path(directory) == site.parent:
            return {site.name}
        if Path(directory) == seed:
            return {".lock"}
        return set()

    job.mkdir()
    try:
        shutil.copytree(seed, job, symlinks=True, dirs_exist_ok=True, ignore=skip)
        local = job / site.relative_to(seed)
        local.mkdir()
        for entry in site.iterdir():
            target = local / entry.name
            if entry.is_dir() and (
                entry.name.endswith(".dist-info") or entry.name == "__pycache__"
            ):
                # `uv` scans real metadata directories. A job-local bytecode
                # directory can read adjacent caches and acquire its own new files.
                target.mkdir()
                for child in entry.iterdir():
                    (target / child.name).symlink_to(child)
            else:
                target.symlink_to(entry)
        if stable_imports:
            # Replace the job's path in place at its original precedence. Using a single
            # path keeps `importlib.metadata` from seeing the same installation
            # through both paths. The job still processes its other `.pth` files.
            module = "_uv_benchmark_stable_imports"
            with (local / f"{module}.py").open("x", encoding="ascii") as source:
                source.write(
                    "import os\nimport sys\n\n"
                    "def activate():\n"
                    "    local = os.path.dirname(__file__)\n"
                    "    if local in sys.path:\n"
                    f"        sys.path[sys.path.index(local)] = {str(site)!a}\n"
                )
            # CPython 3.15 uses the matching `.start` in place of the executable
            # `.pth` line. Older interpreters use the same function via `.pth`.
            with (local / "00_uv_stable_imports.pth").open(
                "x", encoding="ascii"
            ) as hook:
                hook.write(f"import {module}; {module}.activate()\n")
            with (local / "00_uv_stable_imports.start").open(
                "x", encoding="ascii"
            ) as hook:
                hook.write(f"{module}:activate\n")
    except BaseException:
        shutil.rmtree(job)
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("seed", type=Path)
    parser.add_argument("job", type=Path)
    parser.add_argument("--stable-imports", action="store_true")
    args = parser.parse_args()
    project(args.seed, args.job, stable_imports=args.stable_imports)


if __name__ == "__main__":
    main()
