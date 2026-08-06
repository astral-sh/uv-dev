import argparse
import hashlib
import json
import os
import platform
import random
import statistics
import subprocess
import sys
import time


def parse_arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--fixed", required=True)
    parser.add_argument("--python", required=True)
    parser.add_argument("--cache-dir", required=True)
    parser.add_argument("--script", action="append", default=[])
    parser.add_argument("--cores", type=int, nargs="+", default=[])
    parser.add_argument("--samples", type=int, default=20)
    return parser.parse_args()


def run(command):
    started = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, check=True)
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    return elapsed_ms, hashlib.sha256(result.stdout).hexdigest()


def commands(arguments):
    workloads = [("version", ["--version"])]
    workloads.append(
        (
            "cache_dir",
            ["cache", "dir", "--cache-dir", arguments.cache_dir, "--no-config"],
        )
    )
    for specification in arguments.script:
        name, script = specification.split("=", 1)
        workloads.append(
            (
                f"workspace_metadata_{name}",
                [
                    "workspace",
                    "metadata",
                    "--sync",
                    "--script",
                    script,
                    "--python",
                    arguments.python,
                    "--cache-dir",
                    arguments.cache_dir,
                    "--offline",
                    "--no-config",
                ],
            )
        )
    return workloads


def bootstrap_interval(deltas):
    generator = random.Random(4815162342)
    medians = sorted(
        statistics.median(generator.choices(deltas, k=len(deltas)))
        for _ in range(2_000)
    )
    return medians[50], medians[1949]


def main():
    arguments = parse_arguments()
    has_affinity = hasattr(os, "sched_getaffinity") and hasattr(os, "sched_setaffinity")
    original_affinity = sorted(os.sched_getaffinity(0)) if has_affinity else []
    available_cores = arguments.cores or [
        len(original_affinity) if has_affinity else os.cpu_count()
    ]
    workloads = commands(arguments)

    print(
        json.dumps(
            {
                "event": "configuration",
                "platform": platform.platform(),
                "machine": platform.machine(),
                "available_cores": available_cores,
                "samples": arguments.samples,
                "workloads": [name for name, _ in workloads],
            }
        ),
        flush=True,
    )

    try:
        for core_count in available_cores:
            if has_affinity:
                os.sched_setaffinity(0, set(original_affinity[:core_count]))
            elif core_count != os.cpu_count():
                raise SystemExit("CPU affinity is not available on this platform")

            for workload_name, workload_arguments in workloads:
                commands_by_variant = {
                    "baseline": [arguments.baseline, *workload_arguments],
                    "fixed": [arguments.fixed, *workload_arguments],
                }
                _, baseline_hash = run(commands_by_variant["baseline"])
                _, fixed_hash = run(commands_by_variant["fixed"])
                if workload_name != "version" and baseline_hash != fixed_hash:
                    raise SystemExit(f"Output differs for {workload_name}")

                samples = {"baseline": [], "fixed": []}
                paired_deltas = []
                random_generator = random.Random(core_count * 743 + len(workload_name))
                for _ in range(arguments.samples):
                    variants = ["baseline", "fixed"]
                    random_generator.shuffle(variants)
                    pair = {}
                    for variant in variants:
                        elapsed, _ = run(commands_by_variant[variant])
                        pair[variant] = elapsed
                        samples[variant].append(elapsed)
                    paired_deltas.append(pair["baseline"] - pair["fixed"])

                baseline_median = statistics.median(samples["baseline"])
                fixed_median = statistics.median(samples["fixed"])
                interval_low, interval_high = bootstrap_interval(paired_deltas)
                print(
                    json.dumps(
                        {
                            "event": "result",
                            "platform": platform.system(),
                            "cores": core_count,
                            "workload": workload_name,
                            "samples": arguments.samples,
                            "baseline_median_ms": round(baseline_median, 3),
                            "fixed_median_ms": round(fixed_median, 3),
                            "paired_median_saved_ms": round(
                                statistics.median(paired_deltas), 3
                            ),
                            "paired_median_saved_ci95_ms": [
                                round(interval_low, 3),
                                round(interval_high, 3),
                            ],
                            "improvement_percent": round(
                                (baseline_median - fixed_median)
                                / baseline_median
                                * 100,
                                1,
                            ),
                        }
                    ),
                    flush=True,
                )
    finally:
        if has_affinity:
            os.sched_setaffinity(0, set(original_affinity))


if __name__ == "__main__":
    main()
