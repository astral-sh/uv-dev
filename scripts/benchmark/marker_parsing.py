"""Compare marker parsing with a fresh interner for every measurement."""

import argparse
import json
import os
import platform
import statistics
import subprocess
from pathlib import Path


def summarize(samples: list[int]) -> dict[str, float]:
    quartiles = statistics.quantiles(samples, n=4, method="inclusive")
    return {
        "median_ns": statistics.median(samples),
        "p25_ns": quartiles[0],
        "p75_ns": quartiles[2],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path, help="base marker_parse binary")
    parser.add_argument("head", type=Path, help="head marker_parse binary")
    parser.add_argument("--samples", type=int, default=31)
    parser.add_argument(
        "--cpu", type=int, help="pin both binaries to this CPU on Linux"
    )
    args = parser.parse_args()
    if args.samples < 2:
        parser.error("--samples must be at least 2")
    if args.cpu is not None:
        if not hasattr(os, "sched_setaffinity"):
            parser.error("--cpu requires CPU affinity support")
        os.sched_setaffinity(0, {args.cpu})

    cases = {
        "short-version": "python_version >= '3.9'",
        "short-and": "python_version < '3.12' and sys_platform != 'win32'",
        "short-or": "sys_platform == 'win32' or sys_platform == 'darwin'",
    }
    for operator, comparison in [("and", "!="), ("or", "==")]:
        for length in [32, 256, 1024]:
            cases[f"{operator}-{length}"] = f" {operator} ".join(
                f"os_name {comparison} 'target-{index:04}'" for index in range(length)
            )

    binaries = {"base": args.base.resolve(), "head": args.head.resolve()}
    results = []
    for name, marker in cases.items():
        samples: dict[str, list[int]] = {"base": [], "head": []}
        for iteration in range(args.samples):
            # Alternate order so a changing system load does not favor one revision.
            order = ("base", "head") if iteration % 2 == 0 else ("head", "base")
            for revision in order:
                output = subprocess.run(
                    [binaries[revision]],
                    input=marker,
                    text=True,
                    capture_output=True,
                    check=True,
                    timeout=60,
                )
                samples[revision].append(int(output.stdout.strip()))
        base = summarize(samples["base"])
        head = summarize(samples["head"])
        results.append(
            {
                "case": name,
                "input_bytes": len(marker.encode()),
                "base": base,
                "head": head,
                "head_over_base": head["median_ns"] / base["median_ns"],
                "samples_ns": samples,
            }
        )
    print(
        json.dumps(
            {
                "platform": platform.platform(),
                "samples_per_revision": args.samples,
                "cpu": args.cpu,
                "results": results,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
