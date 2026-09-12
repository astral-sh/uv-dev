"""Compare wheel-metadata coordination, warm-cache cost, and cancellation."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import importlib.util
import io
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "uv_concurrent_download_qualification",
    Path(__file__).with_name("qualify-concurrent-downloads.py"),
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Could not load the concurrent-download qualification")
concurrent = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = concurrent
SPEC.loader.exec_module(concurrent)

SCENARIOS = (
    "cost-pep658",
    "cost-range",
    "interrupted-pep658",
    "interrupted-range",
    "warm-lock-pep658",
    "warm-lock-range",
)
VERSION = "0.1.0"
ROOT_NAME = "uv-metadata-root"
WARM_LOCK_PROBE_SECONDS = 2
WARM_LOCK_BODY_DELAY_MS = 5000


def binary_record(path: Path) -> dict:
    return {
        "path": str(path),
        "sha256": concurrent.sha256(path),
        "version": subprocess.run(
            [path, "--version"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip(),
    }


def qualification(
    args: argparse.Namespace, binary: Path, directory: Path
) -> concurrent.Qualification:
    options = argparse.Namespace(**vars(args))
    options.uv = binary
    return concurrent.Qualification(options, directory)


def request_counts(counts: dict[str, int]) -> dict[str, int]:
    return {
        key: value
        for key, value in counts.items()
        if key.startswith(("GET /", "HEAD /")) and " [" not in key
    }


def require_no_requests(counts: dict[str, int], context: str) -> None:
    if requests := request_counts(counts):
        raise AssertionError(f"{context} made HTTP requests: {requests}")


def metadata_key(filename: str, *, pep658: bool) -> str:
    return (
        f"GET /files/{filename}.metadata"
        if pep658
        else f"GET /files/{filename} [range]"
    )


def write_wheel(directory: Path, name: str, requires: list[str]) -> dict:
    stem = name.replace("-", "_")
    dist_info = f"{stem}-{VERSION}.dist-info"
    metadata = (
        f"Metadata-Version: 2.3\nName: {name}\nVersion: {VERSION}\n"
        "Requires-Python: >=3.11\n"
        + "".join(f"Requires-Dist: {requirement}\n" for requirement in requires)
    ).encode()
    files = {
        f"{stem}.py": b"VALUE = 42\n",
        f"{dist_info}/METADATA": metadata,
        f"{dist_info}/WHEEL": (
            b"Wheel-Version: 1.0\nGenerator: uv-bench-fixture\n"
            b"Root-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    record = io.StringIO()
    writer = csv.writer(record, lineterminator="\n")
    for path, contents in files.items():
        digest = (
            base64.urlsafe_b64encode(hashlib.sha256(contents).digest())
            .rstrip(b"=")
            .decode()
        )
        writer.writerow([path, "sha256=" + digest, len(contents)])
    writer.writerow([f"{dist_info}/RECORD", "", ""])
    files[f"{dist_info}/RECORD"] = record.getvalue().encode()
    filename = f"{stem}-{VERSION}-py3-none-any.whl"
    path = directory / filename
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        for member, contents in files.items():
            info = zipfile.ZipInfo(member, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            wheel.writestr(info, contents)
    return {
        "filename": filename,
        "sha256": concurrent.sha256(path),
        "size": path.stat().st_size,
    }


def write_graph(directory: Path, packages: int) -> tuple[Path, list[dict]]:
    directory.mkdir(parents=True)
    leaves = [f"uv-metadata-leaf-{number:04}" for number in range(packages - 1)]
    entries = [write_wheel(directory, name, []) for name in leaves]
    entries.append(
        write_wheel(directory, ROOT_NAME, [f"{name}=={VERSION}" for name in leaves])
    )
    manifest = directory / "fixtures.json"
    manifest.write_text(json.dumps(entries, indent=2) + "\n")
    return manifest, entries


def summary(samples: list[float]) -> dict[str, float]:
    quartiles = statistics.quantiles(samples, n=4, method="inclusive")
    return {
        "minimum": min(samples),
        "p25": quartiles[0],
        "median": statistics.median(samples),
        "p75": quartiles[2],
        "maximum": max(samples),
        "mean": statistics.mean(samples),
    }


def paired_phase(
    args: argparse.Namespace,
    server: concurrent.FixtureServer,
    participants: dict[str, concurrent.Qualification],
    directory: Path,
    *,
    offline: bool,
) -> dict:
    commands = {
        label: participant.command(
            directory / label / "cache",
            directory / label / "target",
            [f"{ROOT_NAME}=={VERSION}"],
            index=server.url + "/simple",
            dry_run=True,
            offline=offline,
        )
        for label, participant in participants.items()
    }
    for _ in range(args.warmups):
        for label, participant in participants.items():
            participant.run([commands[label]])
    server.reset()
    samples: dict[str, list[float]] = {"base": [], "candidate": []}
    pairs = []
    for number in range(args.iterations):
        order = ["base", "candidate"] if number % 2 == 0 else ["candidate", "base"]
        pair = {"order": order}
        for label in order:
            elapsed = participants[label].run([commands[label]])
            samples[label].append(elapsed)
            pair[label + "_seconds"] = elapsed
        pair["candidate_over_base"] = pair["candidate_seconds"] / pair["base_seconds"]
        pair["candidate_minus_base_seconds"] = (
            pair["candidate_seconds"] - pair["base_seconds"]
        )
        pairs.append(pair)
    counts = server.stats()
    require_no_requests(counts, "Offline metadata" if offline else "Fresh metadata")
    return {
        "mode": "offline" if offline else "warm",
        "samples_seconds": samples,
        "summary_seconds": {
            label: summary(values) for label, values in samples.items()
        },
        "paired_ratio": summary([pair["candidate_over_base"] for pair in pairs]),
        "paired_difference_seconds": summary(
            [pair["candidate_minus_base_seconds"] for pair in pairs]
        ),
        "pairs": pairs,
        "counts": counts,
    }


def warm_cost(
    args: argparse.Namespace,
    directory: Path,
    fixture_directory: Path,
    manifest: Path,
    entries: list[dict],
    *,
    pep658: bool,
) -> dict:
    name = "cost-pep658" if pep658 else "cost-range"
    directory = directory / name
    participants = {
        label: qualification(args, binary, directory / label)
        for label, binary in [("base", args.base), ("candidate", args.candidate)]
    }
    with concurrent.FixtureServer(
        args.python,
        fixture_directory,
        manifest,
        0,
        core_metadata=pep658,
    ) as server:
        cold = {}
        for label, participant in participants.items():
            target = directory / label / "target"
            target.mkdir(parents=True)
            server.reset()
            elapsed = participant.run(
                [
                    participant.command(
                        directory / label / "cache",
                        target,
                        [f"{ROOT_NAME}=={VERSION}"],
                        index=server.url + "/simple",
                        dry_run=True,
                    )
                ]
            )
            counts = server.stats()
            for entry in entries:
                key = metadata_key(entry["filename"], pep658=pep658)
                if counts.get(key, 0) != 1:
                    raise AssertionError(
                        f"Expected one cold metadata request for {key}: {counts}"
                    )
            if any(value for key, value in counts.items() if key.endswith(" [body]")):
                raise AssertionError(f"Cold metadata fetched a wheel body: {counts}")
            cold[label] = {"elapsed_seconds": elapsed, "counts": counts}
        phases = [
            paired_phase(args, server, participants, directory, offline=offline)
            for offline in [False, True]
        ]
    return {"name": name, "cold": cold, "phases": phases}


def wait_for(
    server: concurrent.FixtureServer,
    process: subprocess.Popen[bytes],
    predicate,
    timeout: float,
) -> dict[str, int]:
    deadline = time.monotonic() + timeout
    while True:
        counts = server.stats(idle=False)
        if predicate(counts):
            return counts
        if process.poll() is not None or time.monotonic() >= deadline:
            raise AssertionError(f"Expected an active fixture request: {counts}")
        time.sleep(0.005)


def terminate(process: subprocess.Popen[bytes]) -> int:
    if process.poll() is None:
        process.terminate()
    try:
        process.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate(timeout=10)
    if process.returncode == 0:
        raise AssertionError("Interrupted command completed successfully")
    return process.returncode


def close_process(process: subprocess.Popen[bytes] | None) -> None:
    if process is not None:
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=10)


def cancelled_metadata(
    args: argparse.Namespace,
    directory: Path,
    manifest: Path,
    wheel: concurrent.Wheel,
    *,
    pep658: bool,
) -> dict:
    name = "interrupted-pep658" if pep658 else "interrupted-range"
    directory = directory / name
    key = metadata_key(wheel.path.name, pep658=pep658)
    active = "GET file metadata [active]" if pep658 else "GET file ranges [active]"
    observations = {}
    with concurrent.FixtureServer(
        args.python,
        args.directory,
        manifest,
        args.metadata_delay_ms,
        core_metadata=pep658,
    ) as server:
        for label, binary in [("base", args.base), ("candidate", args.candidate)]:
            participant = qualification(args, binary, directory / label)
            cache = directory / label / "cache"
            requirements = [f"{wheel.name}=={wheel.version}"]
            index = server.url + "/simple"
            server.reset()
            leader = participant.spawn(
                participant.command(
                    cache,
                    directory / label / "leader",
                    requirements,
                    index=index,
                    dry_run=True,
                )
            )
            try:
                active_counts = wait_for(
                    server,
                    leader,
                    lambda counts: counts.get(active, 0) > 0,
                    args.timeout,
                )
                leader_code = terminate(leader)
            finally:
                close_process(leader)
            interrupted_counts = server.stats()
            server.reset()
            elapsed = participant.run(
                [
                    participant.command(
                        cache,
                        directory / label / f"recovered-{number}",
                        requirements,
                        index=index,
                        dry_run=True,
                    )
                    for number in range(args.processes)
                ]
            )
            counts = server.stats()
            if counts.get(key, 0) < 1:
                raise AssertionError(
                    f"No metadata was fetched after cancellation: {counts}"
                )
            if (
                label == "candidate"
                and args.expect_coalesced_candidate
                and counts[key] != 1
            ):
                raise AssertionError(f"Recovery metadata was not coalesced: {counts}")
            if counts.get(wheel.count_key("body"), 0):
                raise AssertionError(f"Metadata recovery downloaded a wheel: {counts}")
            server.reset()
            participant.run(
                [
                    participant.command(
                        cache,
                        directory / label / "offline",
                        requirements,
                        index=index,
                        dry_run=True,
                        offline=True,
                    )
                ]
            )
            offline_counts = server.stats()
            require_no_requests(offline_counts, "Recovered offline metadata")
            observations[label] = {
                "leader_exit_code": leader_code,
                "active_counts": active_counts,
                "interrupted_counts": interrupted_counts,
                "recovery_elapsed_seconds": elapsed,
                "recovery_counts": counts,
                "offline_counts": offline_counts,
            }
    return {"name": name, "observations": observations}


def warm_lock(
    args: argparse.Namespace,
    directory: Path,
    manifest: Path,
    wheel: concurrent.Wheel,
    *,
    pep658: bool,
) -> dict:
    name = "warm-lock-pep658" if pep658 else "warm-lock-range"
    directory = directory / name
    observations = {}
    with concurrent.FixtureServer(
        args.python,
        args.directory,
        manifest,
        20,
        WARM_LOCK_BODY_DELAY_MS,
        core_metadata=pep658,
    ) as server:
        for label, binary in [("base", args.base), ("candidate", args.candidate)]:
            participant = qualification(args, binary, directory / label)
            cache = directory / label / "cache"
            requirements = [f"{wheel.name}=={wheel.version}"]
            index = server.url + "/simple"
            participant.run(
                [
                    participant.command(
                        cache,
                        directory / label / "seed",
                        requirements,
                        index=index,
                        dry_run=True,
                    )
                ]
            )
            server.reset()
            leader = participant.spawn(
                participant.command(
                    cache,
                    directory / label / "leader",
                    requirements,
                    index=index,
                )
            )
            warm = None
            try:
                before = wait_for(
                    server,
                    leader,
                    lambda counts: (
                        0
                        < counts.get(wheel.count_key("body-bytes"), 0)
                        < wheel.path.stat().st_size
                    ),
                    args.timeout,
                )
                started = time.perf_counter()
                warm = participant.spawn(
                    participant.command(
                        cache,
                        directory / label / "warm",
                        requirements,
                        index=index,
                        dry_run=True,
                        offline=True,
                    )
                )
                try:
                    _, stderr = warm.communicate(timeout=WARM_LOCK_PROBE_SECONDS)
                    blocked = False
                except subprocess.TimeoutExpired:
                    blocked = True
                    stderr = b""
                before_release = time.perf_counter() - started
                leader_code = terminate(leader)
                if blocked:
                    _, stderr = warm.communicate(timeout=args.timeout)
                elapsed = time.perf_counter() - started
                if warm.returncode != 0:
                    raise RuntimeError(
                        f"Warm metadata failed: {stderr.decode(errors='replace')}"
                    )
            finally:
                close_process(warm)
                close_process(leader)
            after = server.stats()
            if request_counts(after) != request_counts(before):
                raise AssertionError(
                    f"Warm offline metadata made requests: {before} -> {after}"
                )
            if label == "base" and args.expect_nonblocking_base and blocked:
                raise AssertionError("The base warm-cache control was blocked")
            if label == "candidate":
                expected = args.expected_candidate_warm_lock
                if expected != "either" and blocked != (expected == "blocked"):
                    raise AssertionError(
                        f"Expected {expected} candidate warm-cache read; blocked={blocked}"
                    )
            observations[label] = {
                "leader_exit_code": leader_code,
                "body_bytes_before_probe": before[wheel.count_key("body-bytes")],
                "blocked_for_probe_window": blocked,
                "elapsed_before_leader_release_seconds": before_release,
                "total_warm_elapsed_seconds": elapsed,
                "counts_before_probe": before,
                "counts_after_recovery": after,
            }
    return {
        "name": name,
        "probe_seconds": WARM_LOCK_PROBE_SECONDS,
        "body_delay_ms": WARM_LOCK_BODY_DELAY_MS,
        "observations": observations,
    }


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-fixtures"
    )
    parser.add_argument(
        "--manifest", type=Path, default=Path(__file__).with_name("fixtures.json")
    )
    parser.add_argument(
        "--work-directory", type=Path, default=root / ".cache/bench-wheel-metadata"
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--packages", type=int, default=128)
    parser.add_argument("--iterations", type=int, default=30)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--downloads", type=int, default=50)
    parser.add_argument("--processes", type=int, default=4)
    parser.add_argument("--fd-limit", type=int)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--metadata-delay-ms", type=float, default=1000)
    parser.add_argument("--preview-feature", action="append", default=[])
    parser.add_argument("--scenario", choices=SCENARIOS, action="append", default=[])
    parser.add_argument("--expect-coalesced-candidate", action="store_true")
    parser.add_argument("--expect-nonblocking-base", action="store_true")
    parser.add_argument(
        "--expected-candidate-warm-lock",
        choices=["either", "blocked", "unblocked"],
        default="either",
    )
    args = parser.parse_args()
    if not 2 <= args.packages <= 4096 or args.iterations < 2 or args.warmups < 0:
        parser.error(
            "use 2-4096 packages, at least two iterations, and nonnegative warmups"
        )
    if args.downloads < 1 or args.processes < 2 or args.timeout <= 0:
        parser.error(
            "downloads and timeout must be positive; processes must be at least two"
        )
    if args.metadata_delay_ms < 100:
        parser.error(
            "metadata delay must be at least 100 ms for observable cancellation"
        )
    if args.fd_limit is not None and (os.name != "posix" or args.fd_limit < 16):
        parser.error("--fd-limit requires POSIX and a limit of at least 16")
    args.base = args.base.resolve(strict=True)
    args.candidate = args.candidate.resolve(strict=True)
    args.python = args.python.resolve(strict=True)
    args.directory = args.directory.resolve(strict=True)
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    scenarios = [
        name for name in SCENARIOS if not args.scenario or name in args.scenario
    ]
    selected, wheels = concurrent.load_wheels(args.directory, args.manifest)
    click = next(wheel for wheel in wheels if wheel.name.lower() == "click")
    report = {
        "base": binary_record(args.base),
        "candidate": binary_record(args.candidate),
        "python": str(args.python),
        "python_version": subprocess.run(
            [args.python, "--version"], check=True, capture_output=True, text=True
        ).stdout.strip(),
        "platform": sys.platform,
        "harness_sha256": concurrent.sha256(Path(__file__)),
        "concurrent_harness_sha256": concurrent.sha256(
            Path(__file__).with_name("qualify-concurrent-downloads.py")
        ),
        "fixture_server_sha256": concurrent.sha256(
            Path(__file__).with_name("serve-fixtures.py")
        ),
        "fixtures": selected,
        "packages": args.packages,
        "iterations": args.iterations,
        "warmups": args.warmups,
        "concurrent_builds": 1,
        "concurrent_downloads": args.downloads,
        "processes": args.processes,
        "fd_limit": args.fd_limit,
        "timeout_seconds": args.timeout,
        "preview_features": args.preview_feature,
        "metadata_delay_ms": args.metadata_delay_ms,
        "expect_coalesced_candidate": args.expect_coalesced_candidate,
        "expect_nonblocking_base": args.expect_nonblocking_base,
        "expected_candidate_warm_lock": args.expected_candidate_warm_lock,
        "scenarios": scenarios,
        "measurement_scope": (
            "Paired end-to-end CLI latency on a generated, valid multi-wheel dependency "
            "graph. This is not an optimized in-process CodSpeed result."
        ),
        "results": [],
    }
    try:
        with tempfile.TemporaryDirectory(
            prefix="wheel-metadata-", dir=args.work_directory
        ) as temporary:
            directory = Path(temporary)
            real_manifest = directory / "real-fixtures.json"
            real_manifest.write_text(json.dumps(selected, indent=2) + "\n")
            graph_directory = directory / "graph"
            graph_manifest, entries = write_graph(graph_directory, args.packages)
            report["graph_manifest_sha256"] = concurrent.sha256(graph_manifest)
            report["graph_wheels"] = entries
            for scenario in scenarios:
                pep658 = scenario.endswith("pep658")
                if scenario.startswith("cost-"):
                    result = warm_cost(
                        args,
                        directory,
                        graph_directory,
                        graph_manifest,
                        entries,
                        pep658=pep658,
                    )
                elif scenario.startswith("interrupted-"):
                    result = cancelled_metadata(
                        args, directory, real_manifest, click, pep658=pep658
                    )
                else:
                    result = warm_lock(
                        args, directory, real_manifest, click, pep658=pep658
                    )
                report["results"].append(result)
        report["success"] = True
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        output = json.dumps(report, indent=2) + "\n"
        if args.output is not None:
            args.output.write_text(output)
        else:
            print(output, end="")


if __name__ == "__main__":
    main()
