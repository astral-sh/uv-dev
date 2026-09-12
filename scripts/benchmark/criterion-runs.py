"""Retain independent native Criterion runs and summarize their timing noise."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
from pathlib import Path


def relative_stddev(values: list[float]) -> float:
    mean = statistics.mean(values)
    return statistics.stdev(values) / mean if len(values) > 1 and mean else 0.0


def summarize(directory: Path, repetitions: int) -> list[dict]:
    measurements: dict[str, list[dict]] = {}
    expected = None
    for repetition in range(1, repetitions + 1):
        names = set()
        for metadata in sorted(
            (directory / f"run-{repetition}").glob("**/new/benchmark.json")
        ):
            name = json.loads(metadata.read_text())["full_id"]
            sample = json.loads(metadata.with_name("sample.json").read_text())
            times = [
                elapsed / count
                for elapsed, count in zip(sample["times"], sample["iters"], strict=True)
            ]
            estimates = json.loads(metadata.with_name("estimates.json").read_text())
            measurements.setdefault(name, []).append(
                {
                    "repetition": repetition,
                    "samples": len(times),
                    "median_ns": statistics.median(times),
                    "relative_stddev": relative_stddev(times),
                    "median_confidence_interval": estimates["median"][
                        "confidence_interval"
                    ],
                }
            )
            names.add(name)
        if not names or (expected is not None and names != expected):
            raise ValueError(
                f"Benchmark set changed or is empty in repetition {repetition}"
            )
        expected = names
    return [
        {
            "name": name,
            "median_ns": statistics.median(item["median_ns"] for item in runs),
            "between_run_relative_stddev": relative_stddev(
                [item["median_ns"] for item in runs]
            ),
            "maximum_within_run_relative_stddev": max(
                item["relative_stddev"] for item in runs
            ),
            "runs": runs,
        }
        for name, runs in sorted(measurements.items())
    ]


def output(*command: str, root: Path) -> str:
    return subprocess.check_output(command, cwd=root, text=True).strip()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--bench", required=True, action="append")
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--filter")
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    if args.repetitions < 2:
        parser.error("At least two independent repetitions are required")
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    binary = (
        args.binary
        or root / "target/profiling" / ("uv.exe" if os.name == "nt" else "uv")
    ).resolve()
    environment = os.environ | {"UV_BENCH_BINARY": str(binary)}
    command = ["cargo", "bench", "--locked", "--profile", "profiling", "-p", "uv-bench"]
    for bench in args.bench:
        command.extend(["--bench", bench])
    subprocess.run([*command, "--no-run"], cwd=root, env=environment, check=True)
    with binary.open("rb") as stream:
        binary_sha256 = hashlib.file_digest(stream, "sha256").hexdigest()
    metadata = {
        "commit": output("git", "rev-parse", "HEAD", root=root),
        "working_tree_dirty": bool(output("git", "status", "--porcelain", root=root)),
        "operating_system": platform.platform(),
        "architecture": platform.machine(),
        "rustc": output("rustc", "-Vv", root=root),
        "binary_sha256": binary_sha256,
        "binary_version": output(str(binary), "--version", root=root),
        "benchmarks": args.bench,
        "filter": args.filter,
        "repetitions": args.repetitions,
        "github": {
            key: os.environ[key]
            for key in (
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_JOB",
            )
            if key in os.environ
        },
    }
    (directory / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    for repetition in range(1, args.repetitions + 1):
        arguments = ["--", "--noplot"]
        if args.filter:
            arguments.append(args.filter)
        subprocess.run(
            [*command, *arguments],
            cwd=root,
            env=environment | {"CRITERION_HOME": str(directory / f"run-{repetition}")},
            check=True,
        )
    summary = summarize(directory, args.repetitions)
    (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for item in summary:
        print(
            f"{item['name']}: {item['median_ns'] / 1e6:.3f} ms; within-run RSD <= {item['maximum_within_run_relative_stddev']:.2%}; between-run RSD {item['between_run_relative_stddev']:.2%}"
        )


if __name__ == "__main__":
    main()
