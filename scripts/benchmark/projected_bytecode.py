"""Measure second-job imports with a shared bytecode cache on stock CPython.

    OPENBLAS_NUM_THREADS=1 python3 projected_bytecode.py /path/to/seed numpy --scratch .

Each external cache starts empty. The first job fills it and the second imports
the same module. Pass the same installation prepared with `uv sync --compile-bytecode`
as `--compiled-seed` to compare reading adjacent bytecode with writes disabled.
Environment creation, seed compilation and cache enumeration are untimed.
"""

import argparse
import itertools
import json
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter_ns

from project_environment import project


def run_import(job: Path, cache: Path | None, module: str) -> float:
    cmd = [
        job / "bin/python",
        "-I",
        *(["-B"] if cache is None else ["-X", f"pycache_prefix={cache}"]),
        "-c",
        "import importlib, sys; importlib.import_module(sys.argv[1])",
        module,
    ]
    start = perf_counter_ns()
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return (perf_counter_ns() - start) / 1e6


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("seed", type=Path)
    parser.add_argument("module")
    parser.add_argument("--scratch", required=True, type=Path)
    parser.add_argument("--compiled-seed", type=Path)
    parser.add_argument("--rounds", type=int, default=12)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    with TemporaryDirectory(dir=args.scratch.resolve()) as temp:
        root = Path(temp)
        jobs = {}
        seeds = {"linked": args.seed, "stable": args.seed}
        if args.compiled_seed:
            if not any(
                args.compiled_seed.glob(
                    "lib/python*/site-packages/**/__pycache__/*.pyc"
                )
            ):
                parser.error("--compiled-seed contains no installed package bytecode")
            seeds["precompiled"] = args.compiled_seed
        for mode, seed in seeds.items():
            jobs[mode] = [root / f"{mode}-{name}" for name in ("first", "second")]
            for job in jobs[mode]:
                project(seed, job, stable_imports=mode == "stable")
        orders = list(itertools.permutations(jobs))
        samples = []
        for index in range(args.rounds):
            for mode in orders[index % len(orders)]:
                first, second = jobs[mode]
                if mode == "precompiled":
                    run_import(first, None, args.module)
                    elapsed = run_import(second, None, args.module)
                    samples.append(
                        {"round": index + 1, "mode": mode, "second_import_ms": elapsed}
                    )
                    continue
                with TemporaryDirectory(dir=root) as fresh_cache:
                    cache = Path(fresh_cache)
                    run_import(first, cache, args.module)
                    before = set(cache.rglob("*.pyc"))
                    elapsed = run_import(second, cache, args.module)
                    added = set(cache.rglob("*.pyc")) - before
                    samples.append(
                        {
                            "round": index + 1,
                            "mode": mode,
                            "second_import_ms": elapsed,
                            "new_site_pyc": sum(
                                "site-packages" in p.relative_to(cache).parts
                                for p in added
                            ),
                            "new_hook_pyc": sum(
                                p.name.startswith("_uv_benchmark_stable_imports.")
                                for p in added
                            ),
                        }
                    )
        print(json.dumps(samples, indent=2))


if __name__ == "__main__":
    main()
