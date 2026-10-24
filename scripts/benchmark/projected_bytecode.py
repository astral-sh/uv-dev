"""Measure second-job imports with a shared bytecode cache on stock CPython.

    OPENBLAS_NUM_THREADS=1 python3 projected_bytecode.py /path/to/seed numpy --scratch .

Each external cache starts empty. The first job fills it and the second imports
the same module. Pass a copy of the same installation prepared with
`uv sync --compile-bytecode` as `--compiled-seed` to compare imports from a precompiled
seed with bytecode writes disabled.
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


def wheel_records(seed: Path) -> dict[str, bytes]:
    return {
        p.parent.name: p.read_bytes()
        for p in seed.glob("lib/python*/site-packages/*.dist-info/RECORD")
    }


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
            records = wheel_records(args.seed)
            if (
                not records
                or records != wheel_records(args.compiled_seed)
                or not (args.seed / "bin/python").samefile(
                    args.compiled_seed / "bin/python"
                )
            ):
                parser.error(
                    "--compiled-seed must have the same interpreter and installed wheel records"
                )
            seeds["precompiled"] = args.compiled_seed
        for mode, seed in seeds.items():
            jobs[mode] = [root / f"{mode}-{name}" for name in ("first", "second")]
            for job in jobs[mode]:
                project(seed, job, stable_imports=mode == "stable")
        if "precompiled" in jobs:
            audit = """
import importlib, pathlib, sys
def require_bytecode(event, args):
    if event == "compile" and isinstance(args[1], str) and "site-packages" in pathlib.Path(args[1]).parts:
        raise RuntimeError(f"compiled installed source during precompiled import: {args[1]}")
sys.addaudithook(require_bytecode)
module = importlib.import_module(sys.argv[1])
if not getattr(module, "__file__", None) or "site-packages" not in pathlib.Path(module.__file__).parts:
    raise ValueError("choose a module from the projected environment")
"""
            subprocess.run(
                [jobs["precompiled"][0] / "bin/python", "-IBc", audit, args.module],
                check=True,
            )
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
