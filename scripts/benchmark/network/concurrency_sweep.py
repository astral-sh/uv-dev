"""Calibrate download limits with one pinned uv binary and isolated paired runs."""

from __future__ import annotations

import argparse
import copy
import email.parser
import importlib.util
import json
import math
import random
import re
import subprocess
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def selections(fixtures) -> list[dict]:
    """Require exactly one wheel for each explicitly selected package."""
    selected = []
    names = set()
    for filename, path in sorted(fixtures.files.items()):
        if not filename.endswith(".whl"):
            raise ValueError("A concurrency calibration manifest must contain wheels")
        metadata = fixtures.metadata[filename + ".metadata"]
        headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
        name = bench.normalize(headers["Name"])
        if name in names:
            raise ValueError(f"More than one wheel selected for {name}")
        names.add(name)
        selected.append(
            {
                "name": name,
                "requirement": f"{name}=={headers['Version']}",
                "filename": filename,
                "index_bytes": len(fixtures.simple[name]),
                "metadata_bytes": len(metadata),
                "wheel_bytes": path.stat().st_size,
            }
        )
    if not selected:
        raise ValueError("Select at least one wheel")
    return selected


def check_equivalence(reference: dict, candidate: dict) -> None:
    for key in ("stdout_sha256", "verified_tree", "verified_files"):
        if reference[key] != candidate[key]:
            raise ValueError(f"Concurrency settings produced different {key}")


def paired_summary(observations: list[dict]) -> dict:
    measured = bench.summary(
        [
            {"parent": pair["reference"], "head": pair["candidate"]}
            for pair in observations
        ]
    )
    return {
        "pairs": measured["pairs"],
        "reference_median_seconds": measured["parent_median_seconds"],
        "candidate_median_seconds": measured["head_median_seconds"],
        "median_paired_ratio": measured["median_paired_ratio"],
        "ratio_95ci": measured["ratio_95ci"],
        "bootstrap_resamples": measured["bootstrap_resamples"],
    }


def strategy_floor(profile, netem, selected, workload, limit):
    required_bytes = sum(
        item["index_bytes"]
        + item["metadata_bytes" if workload == "resolve" else "wheel_bytes"]
        for item in selected
    )
    waves = max(2, math.ceil(2 * len(selected) / limit))
    return {
        "required_bytes": required_bytes,
        "required_waves": waves,
        "seconds": bench.network_floor(profile, required_bytes, waves, netem=netem),
    }


def verify_calibration(data: dict) -> None:
    if data.get("kind") != "download-concurrency-calibration" or not data.get(
        "complete"
    ):
        raise ValueError("Calibration is incomplete")
    if (
        not re.fullmatch(r"[0-9a-f]{40}", data["revision"])
        or not re.fullmatch(r"[0-9a-f]{64}", data["binary"]["sha256"])
        or data["revision"][:9] not in data["binary"]["version"]
        or data["pairs_per_limit"] < 2
        or len(set(data["limits"])) != len(data["limits"])
        or min([data["reference_limit"], *data["limits"]]) < 1
        or set(data["results"]) != {str(limit) for limit in data["limits"]}
    ):
        raise ValueError("Calibration identities or limits differ")
    for limit, result in data["results"].items():
        if (
            len(result["pairs"]) != data["pairs_per_limit"]
            or result["summary"] != paired_summary(result["pairs"])
            or result["strategy_floor"]
            != strategy_floor(
                data["profile"],
                data["netem"],
                data["selected"],
                data["workload"],
                int(limit),
            )
        ):
            raise ValueError(f"Calibration measurements differ for limit {limit}")
        for pair in result["pairs"]:
            for side in ("reference", "candidate"):
                check_equivalence(data["equivalence"], pair[side])
                protocols = pair[side]["frontend_protocols"]
                if data["http2_proxy"] and set(protocols or []) != {"HTTP/2.0"}:
                    raise ValueError("HTTP/2 calibration used a different protocol")
                if not data["http2_proxy"] and protocols is not None:
                    raise ValueError("Unexpected HTTP/2 proxy observations")


def run_pairs(binary, fixtures, profile, trial, data, output) -> None:
    order = random.Random(data["order_seed"])
    for round_number in range(data["warmups"] + data["pairs_per_limit"]):
        limits = data["limits"].copy()
        order.shuffle(limits)
        for position, limit in enumerate(limits):
            sides = ["reference", "candidate"]
            if (round_number + position) % 2:
                sides.reverse()
            observation = {}
            for side in sides:
                settings = copy.copy(trial)
                settings.env = {
                    "UV_CONCURRENT_DOWNLOADS": str(
                        data["reference_limit"] if side == "reference" else limit
                    )
                }
                observation[side] = bench.run_one(binary, fixtures, profile, settings)
            check_equivalence(observation["reference"], observation["candidate"])
            if "equivalence" not in data:
                data["equivalence"] = {
                    key: observation["reference"][key]
                    for key in ("stdout_sha256", "verified_tree", "verified_files")
                }
            check_equivalence(data["equivalence"], observation["reference"])
            if round_number >= data["warmups"]:
                data["results"][str(limit)]["pairs"].append(observation)
                output.write_text(json.dumps(data, indent=2) + "\n")
                print(
                    f"limit {limit}, pair {round_number - data['warmups'] + 1}/{data['pairs_per_limit']}: "
                    f"reference={observation['reference']['seconds']:.4f}s "
                    f"candidate={observation['candidate']['seconds']:.4f}s",
                    flush=True,
                )
    for result in data["results"].values():
        result["summary"] = paired_summary(result["pairs"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--workload", choices=["resolve", "install"], required=True)
    parser.add_argument(
        "--limits", type=int, nargs="+", default=[1, 2, 4, 8, 16, 32, 50]
    )
    parser.add_argument("--reference", type=int, default=50)
    parser.add_argument("--pairs", type=int, default=4)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--order-seed", type=int, default=42)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--http2-proxy", type=Path)
    parser.add_argument("--tls-certificate", type=Path)
    parser.add_argument("--tls-key", type=Path)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if (
        not re.fullmatch(r"[0-9a-f]{40}", args.revision)
        or not re.fullmatch(r"[0-9a-f]{64}", args.binary_sha256)
        or args.pairs < 2
        or args.warmups < 0
        or args.timeout <= 0
        or min([args.reference, *args.limits]) < 1
        or len(set(args.limits)) != len(args.limits)
    ):
        parser.error(
            "use full identities, positive unique limits, and at least two pairs"
        )
    if args.output.exists():
        parser.error("output already exists; retain the existing calibration")
    binary = args.uv.resolve()
    version = subprocess.check_output([binary, "--version"], text=True).strip()
    if bench.digest(binary) != args.binary_sha256 or args.revision[:9] not in version:
        parser.error("the uv binary does not match its recorded identity")
    python = args.python.resolve()
    if not python.is_file():
        parser.error("select a concrete Python executable")
    python_digest = bench.digest(python)
    manifest_digest = bench.digest(args.manifest)
    profile = json.loads(args.profiles.read_text())[args.profile]
    if not profile.get("pep658", True):
        parser.error("the calibration reference requires advertised metadata sidecars")
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    selected = selections(fixtures)
    proxy = None
    if args.http2_proxy:
        if not args.tls_certificate or not args.tls_key or not args.tls_key.is_file():
            parser.error("HTTP/2 calibration requires its TLS certificate and key")
        proxy = {
            "binary": str(args.http2_proxy.resolve()),
            "sha256": bench.digest(args.http2_proxy),
            "version": subprocess.check_output(
                [args.http2_proxy, "version"], text=True
            ).strip(),
            "certificate_sha256": bench.digest(args.tls_certificate),
        }
    elif args.tls_certificate or args.tls_key:
        parser.error("TLS inputs require --http2-proxy")
    command = (
        ["pip", "compile", "--no-header", "--no-annotate"]
        if args.workload == "resolve"
        else ["pip", "install", "--no-deps", "--target", "{work}/site"]
    ) + [
        "--python",
        "{python}",
        "--only-binary",
        ":all:",
        "--default-index",
        "{index}",
    ]
    if args.workload == "install":
        command.append("-r")
    command.append("{work}/requirements.in")
    trial = argparse.Namespace(
        directory=args.directory,
        work_dir=args.work_dir,
        python=str(python),
        command=command,
        requirement=[item["requirement"] for item in selected],
        templates={},
        setup_commands=[],
        env={},
        cache_mode="cold",
        timeout=args.timeout,
        git_root=None,
        http2_proxy=args.http2_proxy.resolve() if args.http2_proxy else None,
        tls_certificate=args.tls_certificate.resolve()
        if args.tls_certificate
        else None,
        tls_key=args.tls_key.resolve() if args.tls_key else None,
        verify_tree="{work}/site" if args.workload == "install" else None,
        normalize_tree_file=[],
        normalize_tree_symlink=[],
        verify_file=[],
    )
    data = {
        "kind": "download-concurrency-calibration",
        "revision": args.revision,
        "binary": {
            "path": str(binary),
            "sha256": args.binary_sha256,
            "version": version,
        },
        "manifest_sha256": manifest_digest,
        "python": {"path": str(python), "sha256": python_digest},
        "profile": profile,
        "netem": bench.netem_profile(),
        "http2_proxy": proxy,
        "workload": args.workload,
        "command": command,
        "selected": selected,
        "reference_limit": args.reference,
        "limits": args.limits,
        "order_seed": args.order_seed,
        "pairs_per_limit": args.pairs,
        "warmups": args.warmups,
        "timeout_seconds": args.timeout,
        "scope": "Exploratory configuration comparison using one source and binary. Each candidate run is paired with a fresh reference run. The 95% intervals are per-comparison and are not adjusted for selecting the best limit. A selected setting needs a separate confirmation study; this calibration cannot qualify a source change.",
        "floor_scope": "Optimistic response-body and request-wave bound for reading each selected package through its index and then its advertised sidecar or wheel. It excludes TCP/TLS setup, headers, compression alternatives, CPU work, and protocol strategies that already know artifact URLs.",
        "results": {},
    }
    for limit in args.limits:
        data["results"][str(limit)] = {
            "strategy_floor": strategy_floor(
                profile, data["netem"], selected, args.workload, limit
            ),
            "pairs": [],
        }
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    run_pairs(binary, fixtures, profile, trial, data, args.output)
    if (
        bench.digest(binary) != args.binary_sha256
        or bench.digest(args.manifest) != manifest_digest
        or bench.digest(python) != python_digest
    ):
        raise ValueError("A pinned calibration input changed during the run")
    data["complete"] = True
    verify_calibration(data)
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(
        json.dumps(
            {key: value["summary"] for key, value in data["results"].items()}, indent=2
        )
    )


if __name__ == "__main__":
    main()
