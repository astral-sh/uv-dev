#!/usr/bin/env python3
"""Measure Insta workspace discovery using already-built nextest binaries."""

import argparse
import json
import os
import platform
import random
import re
import subprocess
import time
from pathlib import Path

BASE = "6142e13ac0b33f0adea07f28bd0835f378b8a8f4"
HEAD = "9ef883c1197733d9859af3045015b92a9c7d45f4"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
SUMMARY = re.compile(r"Summary\s+\[\s*([0-9.]+)s\]\s+([0-9]+) tests run:")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def run(args):
    root = args.workspace.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "diff", "--exit-code", BASE, "--", "Cargo.toml", "Cargo.lock", ".cargo/config.toml", "crates"],
        cwd=root, check=True, stdout=subprocess.DEVNULL,
    )
    environment = os.environ.copy()
    environment.pop("INSTA_WORKSPACE_ROOT", None)
    environment["INSTA_UPDATE"] = "no"
    command = [
        "cargo", "+1.98.1", "nextest", "run", "--color", "never",
        "--cargo-metadata", str(args.cargo_metadata.resolve()),
        "--binaries-metadata", str(args.binaries_metadata.resolve()),
        "--profile", args.profile, "--test-threads", str(args.threads),
        "--retries", "0", "--status-level", "fail", "--final-status-level", "fail",
    ]
    result = {
        "base": BASE, "head": HEAD, "replicate": args.replicate,
        "platform": platform.platform(), "machine": platform.machine(),
        "runner_name": os.environ.get("RUNNER_NAME"),
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "command": command, "pairs": [], "warmups": [],
        "method": "Paired baseline binaries with INSTA_WORKSPACE_ROOT absent or set to the workspace; compilation excluded.",
    }
    expected_count = None

    def sample(label, phase, index):
        nonlocal expected_count
        env = environment.copy()
        if label == "candidate":
            env["INSTA_WORKSPACE_ROOT"] = str(root)
        started = time.perf_counter()
        process = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
        elapsed = time.perf_counter() - started
        prefix = output / f"{phase}-{index:02}-{label}"
        prefix.with_suffix(".stdout").write_text(process.stdout)
        prefix.with_suffix(".stderr").write_text(process.stderr)
        match = SUMMARY.search(ANSI.sub("", process.stdout + process.stderr))
        row = {"variant": label, "external_seconds": elapsed, "returncode": process.returncode}
        if match:
            row.update(seconds=float(match[1]), tests=int(match[2]))
        print(json.dumps({"phase": phase, "index": index, **row}), flush=True)
        if process.returncode or not match:
            raise RuntimeError(f"Unsuccessful run: {prefix}")
        if expected_count is None:
            expected_count = row["tests"]
        if row["tests"] != expected_count:
            raise RuntimeError("The selected test count changed")
        return row

    for index in range(args.warmups):
        result["warmups"].append([sample(label, "warmup", index) for label in ("baseline", "candidate")])
        write_json(output / "results.json", result)
    orders = [("baseline", "candidate"), ("candidate", "baseline")] * ((args.pairs + 1) // 2)
    random.Random(args.seed).shuffle(orders)
    for index, order in enumerate(orders[:args.pairs]):
        rows = {label: sample(label, "pair", index) for label in order}
        result["pairs"].append({"index": index, "order": order, **rows})
        write_json(output / "results.json", result)
    print(json.dumps({"complete": True, "replicate": args.replicate, "pairs": len(result["pairs"]), "tests": expected_count}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=Path.cwd())
    parser.add_argument("--cargo-metadata", type=Path, required=True)
    parser.add_argument("--binaries-metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--replicate", default="local")
    parser.add_argument("--profile", default="ci-linux")
    parser.add_argument("--threads", type=int, default=20)
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--seed", type=int, default=1998)
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
