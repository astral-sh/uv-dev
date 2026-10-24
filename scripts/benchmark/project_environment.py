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
"""

import argparse
import os
import shutil
from pathlib import Path


def project(seed: Path, job: Path) -> None:
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
            if entry.name == "__pycache__":
                continue
            target = local / entry.name
            if entry.name.endswith(".dist-info") and entry.is_dir():
                # `uv` discovers installed wheels by scanning real metadata directories.
                target.mkdir()
                for metadata in entry.iterdir():
                    (target / metadata.name).symlink_to(metadata)
            else:
                target.symlink_to(entry)
    except BaseException:
        shutil.rmtree(job)
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("seed", type=Path)
    parser.add_argument("job", type=Path)
    args = parser.parse_args()
    project(args.seed, args.job)


if __name__ == "__main__":
    main()
