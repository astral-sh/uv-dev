"""Verify an archived paired study against exact local source revisions."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import inspect
import json
import math
import os
import statistics
import subprocess
from pathlib import Path


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def is_study_spec(path: Path, study: dict) -> bool:
    """Recognize the exact study manifest or its pre-build hash template."""
    if path.name == "verification-spec.json" or path.name.endswith(
        "-verification-spec.json"
    ):
        return json.loads(path.read_text()) == study
    if path.name.endswith("-verification-template.json"):
        template = json.loads(path.read_text())
        for side, value in template.get("binary_sha256", {}).items():
            if value is None:
                template["binary_sha256"][side] = study["binary_sha256"].get(side)
        return template == study
    return False


def verify(evidence: Path, repository: Path, study: dict) -> dict:
    parent, head = study["parent"], study["head"]
    for revision in (parent, head):
        resolved = subprocess.check_output(
            ["git", "rev-parse", "--verify", f"{revision}^{{commit}}"],
            cwd=repository,
            text=True,
        ).strip()
        require(resolved == revision, f"Expected a full commit ID: {revision}")
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", parent, head],
        cwd=repository,
        check=True,
    )
    change = subprocess.check_output(
        [
            "git",
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--abbrev=7",
            parent,
            head,
            "--",
        ],
        cwd=repository,
    )
    require((evidence / "change.patch").read_bytes() == change, "Source diff differs")
    require(head in (evidence / "source.txt").read_text(), "Archived source ID differs")
    binary_hashes = study["binary_sha256"]
    require(set(binary_hashes) == {"parent", "head"}, "Both binary hashes are required")
    recorded_hashes = {
        line.split(maxsplit=1)[0]
        for line in (evidence / "binaries.sha256").read_text().splitlines()
        if line.strip()
    }
    require(
        set(binary_hashes.values()) <= recorded_hashes, "Archived binary hashes differ"
    )

    module_spec = importlib.util.spec_from_file_location(
        "archived_network_bench", evidence / "bench.py"
    )
    require(
        module_spec is not None and module_spec.loader is not None,
        "Missing archived harness",
    )
    bench = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(bench)
    expected_files = {case["file"] for case in study["cases"]}
    require(len(expected_files) == len(study["cases"]), "Duplicate study cases")
    observed_files = {
        path.name
        for pattern in study["result_globs"]
        for path in evidence.glob(pattern)
        if path.is_file() and not is_study_spec(path, study)
    }
    require(
        observed_files == expected_files,
        "Case manifest does not cover every study result",
    )
    require(
        any(case["role"] == "fast" for case in study["cases"]),
        "Missing fast-network control",
    )
    require(
        any(case.get("qualifying") for case in study["cases"]),
        "Missing qualifying workload",
    )

    results = {}
    for case in study["cases"]:
        path = evidence / case["file"]
        data = json.loads(path.read_text())
        label = path.name
        require(
            (data["parent_sha"], data["head_sha"]) == (parent, head),
            f"{label}: source IDs",
        )
        require(
            {side: binary["sha256"] for side, binary in data["binaries"].items()}
            == binary_hashes,
            f"{label}: binary hashes",
        )
        for side, revision in (("parent", parent), ("head", head)):
            require(
                revision[:9] in data["binaries"][side]["version"],
                f"{label}: binary version",
            )
        pairs = data["pairs"]
        require(len(pairs) == case["pairs"] >= 20, f"{label}: sample count")
        if case.get("requires_stderr"):
            require(data.get("compare_stderr"), f"{label}: missing stderr comparison")
        require(
            bench.summary(pairs) == data["summary"],
            f"{label}: paired bootstrap differs",
        )
        for pair in pairs:
            if data.get("compare_stderr"):
                require(
                    bool(pair["parent"].get("stderr_sha256"))
                    and pair["parent"]["stderr_sha256"]
                    == pair["head"].get("stderr_sha256"),
                    f"{label}: stderr differs",
                )
            for key in ("stdout_sha256", "verified_tree", "verified_files"):
                require(
                    pair["parent"].get(key) == pair["head"].get(key),
                    f"{label}: {key} differs",
                )
            for side in ("parent", "head"):
                value = pair[side]
                require(
                    math.isfinite(value["seconds"]) and value["seconds"] > 0,
                    f"{label}: duration",
                )
                require(
                    value["requests"] == len(value["events"]), f"{label}: request count"
                )
                require(
                    value["bytes"] == sum(event["bytes"] for event in value["events"]),
                    f"{label}: byte count",
                )
                if case.get("requires_tree"):
                    require(
                        bool(value.get("verified_tree")),
                        f"{label}: missing tree comparison",
                    )
                for filename in case.get("required_files", []):
                    require(
                        filename in value.get("verified_files", {}),
                        f"{label}: missing {filename}",
                    )
        if case.get("qualifying"):
            require(
                case["role"] == "primary", f"{label}: qualifying case must be primary"
            )
            require(
                data["summary"]["ratio_95ci"][1] <= 0.95,
                f"{label}: improvement below 5%",
            )
        lower_bound = data["lower_bound"]
        old_netem = os.environ.get("UV_BENCH_NETEM")
        os.environ["UV_BENCH_NETEM"] = json.dumps(data.get("netem", {}))
        try:
            arguments = [
                data["profile"],
                lower_bound["required_bytes"],
                lower_bound["required_waves"],
            ]
            if (
                "required_latency_ms"
                in inspect.signature(bench.network_floor).parameters
            ):
                arguments.append(lower_bound.get("required_latency_ms"))
            if "required_wait_ms" in inspect.signature(bench.network_floor).parameters:
                arguments.append(lower_bound.get("required_wait_ms", 0))
            require(
                bench.network_floor(*arguments) == lower_bound["seconds"],
                f"{label}: lower-bound arithmetic",
            )
        finally:
            if old_netem is None:
                os.environ.pop("UV_BENCH_NETEM", None)
            else:
                os.environ["UV_BENCH_NETEM"] = old_netem
        results[label] = {
            "role": case["role"],
            "sha256": sha256(path),
            "summary": data["summary"],
            "profile": data["profile"],
            "netem": data.get("netem", {}),
            "lower_bound": lower_bound,
            "traffic": {
                side: {
                    key: statistics.median(pair[side][key] for pair in pairs)
                    for key in ("bytes", "requests")
                }
                for side in ("parent", "head")
            },
        }
    return {
        "parent": parent,
        "head": head,
        "scope": study["scope"],
        "binary_sha256": binary_hashes,
        "change_sha256": sha256(evidence / "change.patch"),
        "harness_sha256": sha256(evidence / "bench.py"),
        "verified_pairs": sum(case["pairs"] for case in study["cases"]),
        "results": results,
        "limitation": "This checks source, recorded binaries, paired statistics, result completeness, output equivalence, traffic totals, and bound arithmetic. The workload and oracle assumptions still require review.",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    result = verify(args.evidence, args.repository, json.loads(args.spec.read_text()))
    if args.archive:
        result["evidence_sha256"] = sha256(args.archive)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                "head": result["head"],
                "scenarios": len(result["results"]),
                "pairs": result["verified_pairs"],
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
