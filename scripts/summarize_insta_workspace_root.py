#!/usr/bin/env python3
"""Summarize paired measurements with a two-sided 95% Student t interval."""

import argparse
import json
import math
import statistics
from pathlib import Path

# Critical values for 1 through 30 degrees of freedom.
T95 = (
    12.706204736, 4.302652730, 3.182446305, 2.776445105, 2.570581836,
    2.446911851, 2.364624252, 2.306004135, 2.262157163, 2.228138852,
    2.200985160, 2.178812830, 2.160368656, 2.144786688, 2.131449546,
    2.119905299, 2.109815578, 2.100922040, 2.093024054, 2.085963447,
    2.079613845, 2.073873068, 2.068657610, 2.063898562, 2.059538553,
    2.055529439, 2.051830516, 2.048407142, 2.045229642, 2.042272456,
)


def interval(values):
    count = len(values)
    if not 2 <= count <= 31:
        raise ValueError("The interval requires 2–31 independent observations")
    mean = statistics.mean(values)
    margin = T95[count - 2] * statistics.stdev(values) / math.sqrt(count)
    return {"mean": mean, "ci95": [mean - margin, mean + margin]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path)
    parser.add_argument("--unit", choices=["pair", "runner"], required=True)
    parser.add_argument("--expected-pairs", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    files = [json.loads(path.read_text()) for path in args.inputs]
    identities = {(data["base"], data["head"]) for data in files}
    if len(identities) != 1:
        raise ValueError("Mismatched source commits")
    counts = set()
    by_runner = []
    for data in files:
        pairs = data["pairs"]
        if len(pairs) != args.expected_pairs:
            raise ValueError(f"Incomplete replicate: {data['replicate']}")
        observations = []
        for pair in pairs:
            before, after = pair["baseline"], pair["candidate"]
            if before["returncode"] or after["returncode"]:
                raise ValueError("Unsuccessful measurement")
            counts.update([before["tests"], after["tests"]])
            observations.append({
                "baseline_seconds": before["seconds"],
                "candidate_seconds": after["seconds"],
                "reduction_seconds": before["seconds"] - after["seconds"],
                "reduction_percent": 100 * (1 - after["seconds"] / before["seconds"]),
            })
        by_runner.append(observations)
    if len(counts) != 1:
        raise ValueError("Mismatched selected test counts")
    keys = tuple(by_runner[0][0])
    if args.unit == "runner":
        observations = [{key: statistics.mean(row[key] for row in rows) for key in keys} for rows in by_runner]
    else:
        if len(files) != 1:
            raise ValueError("Pair-level analysis requires one runner")
        observations = by_runner[0]
    result = {
        "base": files[0]["base"], "head": files[0]["head"],
        "method": "Two-sided 95% Student t intervals on paired differences and relative reductions",
        "independent_unit": args.unit, "independent_observations": len(observations),
        "pairs": sum(len(rows) for rows in by_runner), "tests": counts.pop(),
        "inputs": [str(path) for path in args.inputs],
        **{key: interval([row[key] for row in observations]) for key in keys},
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
