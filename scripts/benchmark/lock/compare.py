"""Compare two runs of the same lock workload and write CSV and SVG results."""

import argparse
import csv
import json
import math
import tomllib
from collections import Counter
from pathlib import Path


def read_results(run):
    receipt = json.loads((run / "run.json").read_text())
    if not receipt["complete"]:
        raise ValueError(f"incomplete run: {run}")
    folder = run / "results"
    if receipt["project"] == "corpus":
        folder /= "corpus"
    results = {}
    for summary in folder.glob("*/summary.json"):
        data = json.loads(summary.read_text())
        duration = data["time"]
        if not math.isfinite(duration) or duration <= 0:
            raise ValueError(f"invalid duration in {summary}")
        if data["package"] != summary.parent.name:
            raise ValueError(f"wrong project name in {summary}")
        results[data["package"]] = (data, summary.parent)
    if not results:
        raise ValueError(f"no completed resolutions in {run}")
    return receipt, results


def cause(result):
    if result is None:
        return "missing"
    summary, directory = result
    if summary["exit_code"] == 0:
        return "success"
    # The pinned harness records the POSIX wait status from wait4.
    if summary["exit_code"] in (124, 124 << 8):
        return "timeout"
    if summary.get("oom_kills") or summary.get("pids_limit_hits"):
        return "resources"
    stderr = (directory / "stderr.txt").read_text(errors="replace").lower()
    if "git executable not found" in stderr:
        return "missing-tool"
    if any(
        term in stderr for term in ("no space left", "out of memory", "os error 28")
    ):
        return "resources"
    if any(
        term in stderr
        for term in (
            "network connectivity is disabled",
            "offline mode",
            "not found in the cache",
        )
    ):
        return "offline-cache"
    if "failed to extract archive" in stderr:
        return "archive"
    # Source distributions report "Failed to download and build" for failures
    # at either stage. Use the nested cause to identify a transport failure.
    transport = stderr.replace("failed to download and build", "")
    if any(
        term in transport
        for term in (
            "failed to download",
            "failed to fetch",
            "error sending request",
            "timed out",
            "invalid peer certificate",
        )
    ):
        return "transport"
    if any(
        term in stderr
        for term in (
            "build backend returned an error",
            "build-backend",
            "failed to build",
            "failed to download and build",
            "build requirements",
            "build dependencies",
        )
    ):
        return "build"
    if "no solution found" in stderr:
        return "resolution"
    if "python" in stderr and (
        "not compatible with" in stderr or "no interpreter" in stderr
    ):
        return "interpreter"
    return "other"


def lock_metrics(directory):
    data = (directory / "uv.lock").read_bytes()
    packages = tomllib.loads(data.decode())["package"]
    names = {p["name"] for p in packages}
    versions = {(p["name"], p["version"]) for p in packages if "version" in p}
    return {
        "lock_bytes": len(data),
        "entries": len(packages),
        "distinct_names": len(names),
        "distinct_versions": len(versions),
        "extra_entries_for_same_name": len(packages) - len(names),
    }, versions


def plot(file, values, label):
    if not values:
        return
    width, height = 720, 360
    left, right, top, bottom = 80, 700, 36, 310
    values = sorted(values)
    low = min(values[0], 0)
    high = max(values[-1], 0)
    if high == low:
        low, high = -1, 1

    def y(value):
        return bottom - (value - low) / (high - low) * (bottom - top)

    points = []
    for rank, value in enumerate(values):
        x = left + rank / max(1, len(values) - 1) * (right - left)
        points.append(f"{x:.2f},{y(value):.2f}")
    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<g font-family="system-ui,sans-serif" font-size="14" fill="#222">',
        f'<text x="{left}" y="22">{label}</text>',
        f'<path d="M{left},{top}V{bottom}H{right}" fill="none" stroke="#aaa"/>',
        f'<path d="M{left},{y(0)}H{right}" fill="none" stroke="#666" stroke-dasharray="4 4"/>',
        f'<text x="{left - 7}" y="{top + 5}" text-anchor="end">{high:.3g}</text>',
        f'<text x="{left - 7}" y="{bottom}" text-anchor="end">{low:.3g}</text>',
        f'<text x="{left}" y="338">1</text>',
        f'<text x="{right}" y="338" text-anchor="end">{len(values)}</text>',
        '<text x="390" y="338" text-anchor="middle">Paired successes, sorted by change</text>',
        f'<polyline points="{" ".join(points)}" fill="none" stroke="#147c96" stroke-width="2"/>',
        f'<circle cx="{left}" cy="{y(values[0])}" r="2.5" fill="#147c96"/>',
        "</g></svg>",
    ]
    file.write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if not Path("/.dockerenv").exists():
        parser.error(
            "read untrusted result files in a container with read-only input mounts"
        )
    baseline, left = read_results(args.baseline)
    candidate, right = read_results(args.candidate)
    if "continued_from" in baseline or "continued_from" in candidate:
        parser.error("continued online seed passes are cache population runs")
    if baseline.get("excluded_root") != candidate.get("excluded_root"):
        parser.error("incomparable excluded roots")
    for field in (
        "image",
        "input_sha256",
        "exclude_newer",
        "offline",
        "seed_sha256",
        "project",
        "harness",
        "workers",
        "python",
        "limit",
    ):
        if baseline[field] != candidate[field]:
            parser.error(f"incomparable {field}")
    args.output.mkdir(exist_ok=False, parents=True)
    counts = Counter()
    rows = []
    for name in sorted(left.keys() | right.keys()):
        first, second = left.get(name), right.get(name)
        a, b = cause(first), cause(second)
        row = {"package": name, "baseline_status": a, "candidate_status": b}
        counts[f"{a} / {b}"] += 1
        versions = {}
        for side, outcome, result in (("baseline", a, first), ("candidate", b, second)):
            if result:
                summary, directory = result
                row[f"{side}_seconds"] = summary["time"]
                if outcome == "success":
                    metrics, versions[side] = lock_metrics(directory)
                    row.update(
                        {f"{side}_{key}": value for key, value in metrics.items()}
                    )
        if a == b == "success":
            row["delta_seconds"] = row["candidate_seconds"] - row["baseline_seconds"]
            row["change_percent"] = 100 * row["delta_seconds"] / row["baseline_seconds"]
            added = versions["candidate"] - versions["baseline"]
            row["new_versions_to_audit"] = len(added)
            row["added_versions"] = " ".join(f"{p}=={v}" for p, v in sorted(added))
        rows.append(row)
    with (args.output / "packages.csv").open("w", newline="") as file:
        writer = csv.DictWriter(
            file, fieldnames=list(dict.fromkeys(k for row in rows for k in row))
        )
        writer.writeheader()
        writer.writerows(rows)
    (args.output / "outcomes.json").write_text(
        json.dumps(counts, indent=2, sort_keys=True) + "\n"
    )
    plot(
        args.output / "absolute.svg",
        [r["delta_seconds"] for r in rows if "delta_seconds" in r],
        "Candidate minus baseline wall time (seconds)",
    )
    plot(
        args.output / "relative.svg",
        [r["change_percent"] for r in rows if "change_percent" in r],
        "Change in wall time (%)",
    )


if __name__ == "__main__":
    main()
