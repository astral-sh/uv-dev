"""Compare exact parent/head nextest archives without compilation in timed runs."""

import argparse
import hashlib
import json
import os
import platform
import random
import re
import subprocess
import time
from pathlib import Path

SUMMARY = re.compile(r"Summary\s+\[\s*([0-9.]+)s\]\s+([0-9]+) tests run:")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=Path.cwd())
    parser.add_argument("--archives", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--replicate", default="local")
    parser.add_argument("--profile", default="ci-linux")
    parser.add_argument("--threads", type=int, default=20)
    parser.add_argument("--targeted-pairs", type=int, default=12)
    parser.add_argument("--full-pairs", type=int, default=3)
    parser.add_argument("--full-filter", default="all()")
    parser.add_argument("--seed", type=int, default=29)
    args = parser.parse_args()
    root = args.workspace.resolve()
    archives = args.archives.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["INSTA_UPDATE"] = "no"
    environment.pop("INSTA_WORKSPACE_ROOT", None)
    environment.pop("CARGO_TARGET_DIR", None)
    commands = {}
    provenance = {}
    for variant in ("baseline", "candidate"):
        archive = archives / f"{variant}.tar.zst"
        destination = archives / variant
        destination.mkdir(exist_ok=True)
        subprocess.run(["tar", "-xf", str(archive), "-C", str(destination)], check=True)
        target = destination / "target"
        commands[variant] = [
            "cargo", "nextest", "run", "--color", "never",
            "--cargo-metadata", str(target / "nextest/cargo-metadata.json"),
            "--binaries-metadata", str(target / "nextest/binaries-metadata.json"),
            "--target-dir-remap", str(target), "--workspace-remap", str(root),
            "--profile", args.profile, "--test-threads", str(args.threads),
            "--retries", "0", "--status-level", "fail", "--final-status-level", "fail",
        ]
        with archive.open("rb") as stream:
            provenance[variant] = {"archive_sha256": hashlib.file_digest(stream, "sha256").hexdigest()}
    result = {
        "base": args.base, "head": args.head, "replicate": args.replicate,
        "platform": platform.platform(), "machine": platform.machine(),
        "runner_name": os.environ.get("RUNNER_NAME"), "run_id": os.environ.get("GITHUB_RUN_ID"),
        "provenance": provenance, "commands": commands, "workloads": {},
        "method": "Exact-revision nextest archives, extracted before measurement; compilation and warmups excluded.",
    }

    for workload, filter_expr, pair_count in (
        ("scenarios", "binary(=pip) and test(/^pip_install_scenarios::/)", args.targeted_pairs),
        ("workspace", args.full_filter, args.full_pairs),
    ):
        if pair_count == 0:
            continue
        expected_count = None
        rows = {"filter": filter_expr, "warmups": [], "pairs": []}
        result["workloads"][workload] = rows

        def sample(variant, phase, index):
            nonlocal expected_count
            command = commands[variant] + ["--filter-expr", filter_expr]
            started = time.perf_counter()
            process = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True)
            elapsed = time.perf_counter() - started
            prefix = output / f"{workload}-{phase}-{index:02}-{variant}"
            prefix.with_suffix(".stdout").write_text(process.stdout)
            prefix.with_suffix(".stderr").write_text(process.stderr)
            match = SUMMARY.search(ANSI.sub("", process.stdout + process.stderr))
            row = {"variant": variant, "external_seconds": elapsed, "returncode": process.returncode}
            if match:
                row.update(seconds=float(match[1]), tests=int(match[2]))
            print(json.dumps({"workload": workload, "phase": phase, "index": index, **row}), flush=True)
            if process.returncode or not match:
                write_json(output / "results.json", result)
                raise RuntimeError(f"Unsuccessful run: {prefix}")
            if expected_count is None:
                expected_count = row["tests"]
            if row["tests"] != expected_count:
                raise RuntimeError("The selected test count changed")
            return row

        rows["warmups"].append([sample(variant, "warmup", 0) for variant in ("baseline", "candidate")])
        write_json(output / "results.json", result)
        orders = [("baseline", "candidate"), ("candidate", "baseline")] * ((pair_count + 1) // 2)
        random.Random(args.seed).shuffle(orders)
        for index, order in enumerate(orders[:pair_count]):
            samples = {variant: sample(variant, "pair", index) for variant in order}
            rows["pairs"].append({"index": index, "order": order, **samples})
            write_json(output / "results.json", result)


if __name__ == "__main__":
    main()
