"""Time one fresh lock in the benchmark container. Every run needs a new output."""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--prefetch-diagnostics", action="store_true")
    args = parser.parse_args()
    if not Path("/.dockerenv").exists() or not os.environ.get("UV_EXCLUDE_NEWER"):
        parser.error("run in Docker with an explicit UV_EXCLUDE_NEWER")

    args.output.mkdir(exist_ok=False)
    project = args.output / "pyproject.toml"
    python = f"{sys.version_info.major}.{sys.version_info.minor}"
    if args.fixture.suffix == ".in":
        requirements = [
            line
            for line in args.fixture.read_text().splitlines()
            if line and not line.startswith("#")
        ]
        project.write_text(
            '[project]\nname = "benchmark"\nversion = "0.0.0"\n'
            f'requires-python = ">={python}"\ndependencies = {json.dumps(requirements)}\n'
        )
    else:
        shutil.copyfile(args.fixture, project)

    # uv lock resolves every optional dependency declared by the root project.
    command = [
        "/bin/uv-bench",
        "lock",
        "--python",
        sys.executable,
        "--no-python-downloads",
    ]
    if args.prefetch_diagnostics:
        command.append("-v")
    with (
        (args.output / "stdout.txt").open("wb") as stdout,
        (args.output / "stderr.txt").open("wb") as stderr,
    ):
        start = time.perf_counter()
        result = subprocess.run(
            command, cwd=args.output, stdout=stdout, stderr=stderr, check=False
        )
        elapsed = time.perf_counter() - start
    summary = {
        "package": args.output.name,
        "exit_code": result.returncode,
        "time": elapsed,
        "python": sys.version,
        "exclude_newer": os.environ["UV_EXCLUDE_NEWER"],
        "extras": sorted(
            tomllib.loads(project.read_text())["project"].get(
                "optional-dependencies", {}
            )
        ),
        "command": command,
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(
        f"{args.output.name} exit={result.returncode} seconds={elapsed:.3f}", flush=True
    )


if __name__ == "__main__":
    main()
