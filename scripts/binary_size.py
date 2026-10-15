"""Measure native Linux release binaries and summarize changes against a baseline."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
BINARIES = {"uv": "release", "uvx": "release", "uv-build": "minimal-size"}


def command_output(command: list[str], source: Path) -> str:
    return subprocess.check_output(command, cwd=source, text=True).strip()


def train(source: Path, output: Path) -> dict:
    source = source.resolve()
    environment = os.environ | {"CARGO_INCREMENTAL": "0"}
    pgo_directory = source / "target-pgo"
    start = time.perf_counter()
    subprocess.run(
        [
            sys.executable,
            "scripts/build_uv_pgo.py",
            "--target",
            TARGET,
            "--target-dir",
            str(pgo_directory),
            "--train-only",
        ],
        cwd=source,
        env=environment,
        check=True,
    )
    duration = round(time.perf_counter() - start, 3)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(pgo_directory / "uv.profdata", output / "uv.profdata")
    return {
        "target": TARGET,
        "revision": command_output(["git", "rev-parse", "HEAD"], source),
        "rustc": command_output(["rustc", "--version"], source),
        "sha256": hashlib.sha256((output / "uv.profdata").read_bytes()).hexdigest(),
        "build_seconds": duration,
    }


def load_profile(directory: Path) -> dict:
    profile = json.loads((directory / "profile.json").read_text())
    if profile["target"] != TARGET:
        raise ValueError(f"Expected a PGO profile for {TARGET}")
    if (
        hashlib.sha256((directory / "uv.profdata").read_bytes()).hexdigest()
        != profile["sha256"]
    ):
        raise ValueError("PGO profile does not match its recorded checksum")
    return profile


def build(source: Path, pgo_directory: Path) -> dict[str, float]:
    source = source.resolve()
    pgo_directory = pgo_directory.resolve()
    profile = load_profile(pgo_directory)
    if profile["rustc"] != command_output(["rustc", "--version"], source):
        raise ValueError(
            "PGO profile and measured revision must use the same Rust toolchain; "
            "update the pinned training revision"
        )
    environment = os.environ | {"CARGO_INCREMENTAL": "0"}
    durations = {}
    commands = {
        "uv": (
            source,
            [
                "--release",
                "-p",
                "uv",
                "--bin",
                "uv",
                "--bin",
                "uvx",
                "--features",
                "self-update",
            ],
        ),
        "uv-build": (
            source / "crates" / "uv-build",
            ["--profile", "minimal-size", "-p", "uv-build", "--bin", "uv-build"],
        ),
    }
    for binary, (directory, arguments) in commands.items():
        build_environment = environment.copy()
        if binary == "uv":
            build_environment["RUSTFLAGS"] = (
                f"{environment.get('RUSTFLAGS', '')} "
                f"-Cprofile-use={pgo_directory / 'uv.profdata'}"
            ).strip()
        start = time.perf_counter()
        subprocess.run(
            [
                "cargo",
                "build",
                "--locked",
                "--target",
                TARGET,
                "-Z",
                "checksum-freshness",
                *arguments,
            ],
            cwd=directory,
            env=build_environment,
            check=True,
        )
        durations[binary] = round(time.perf_counter() - start, 3)
    return durations


def measure(
    source: Path,
    pgo_directory: Path,
    build_seconds: dict | None = None,
) -> dict:
    pgo_profile = load_profile(pgo_directory)
    sizes = {
        binary: (source / "target" / TARGET / profile / binary).stat().st_size
        for binary, profile in BINARIES.items()
    }
    if any(size <= 0 for size in sizes.values()):
        raise ValueError("Cannot measure an empty binary")
    return {
        "target": TARGET,
        "revision": command_output(["git", "rev-parse", "HEAD"], source),
        "rustc": {
            "uv": command_output(["rustc", "--version"], source),
            "uv-build": command_output(
                ["rustc", "--version"], source / "crates" / "uv-build"
            ),
        },
        "sizes": sizes,
        "pgo": True,
        "pgo_revision": pgo_profile["revision"],
        "pgo_profile_sha256": pgo_profile["sha256"],
        "build_seconds": build_seconds,
    }


def report(
    current: dict,
    base: dict | None,
    *,
    repository_url: str,
    sizes_only: bool = False,
) -> str:
    if current["target"] != TARGET or (base and base["target"] != TARGET):
        raise ValueError(f"Expected measurements for {TARGET}")
    if base and (
        current["pgo_profile_sha256"] != base["pgo_profile_sha256"]
        or current["pgo_revision"] != base["pgo_revision"]
    ):
        raise ValueError("Both revisions must use the same PGO profile")

    lines = ["## Binary size", ""]
    if base:
        lines += [
            "| Binary | Base (bytes) | Current (bytes) | Change (bytes) | Change (%) |",
            "| --- | ---: | ---: | ---: | ---: |",
        ]
    else:
        lines += ["| Binary | Size (bytes) |", "| --- | ---: |"]

    for binary in BINARIES:
        size = current["sizes"][binary]
        if type(size) is not int or size <= 0:
            raise ValueError(f"Invalid size for {binary}: {size!r}")
        if base:
            previous = base["sizes"][binary]
            if type(previous) is not int or previous <= 0:
                raise ValueError(f"Invalid baseline size for {binary}: {previous!r}")
            change = size - previous
            lines.append(
                f"| `{binary}` | {previous:,} | {size:,} | {change:+,} | "
                f"{change / previous:+.2%} |"
            )
        else:
            lines.append(f"| `{binary}` | {size:,} |")

    repository_url = repository_url.rstrip("/")
    lines += [
        "",
        "<details>",
        "<summary>Build details</summary>",
        "",
        f"Target: `{TARGET}`.",
        "",
        f"Current revision: [{current['revision'][:8]}]({repository_url}/commit/{current['revision']}).",
    ]
    if base:
        lines.append(
            f"Baseline revision: [{base['revision'][:8]}]({repository_url}/commit/{base['revision']})."
        )
    lines += [
        f"PGO training revision: [{current['pgo_revision'][:8]}]({repository_url}/commit/{current['pgo_revision']}).",
        "",
        (
            "Native Cargo builds with `release` for `uv` and `uvx`, and `minimal-size` "
            "for `uv-build`. `uv` and `uvx` share a PGO profile trained on a pinned "
            "`main` revision using the release training corpus and flags. "
            "Incremental compilation is disabled. Sizes are uncompressed bytes; "
            "SBOM embedding and manylinux packaging are excluded."
        ),
        "",
        "</details>",
    ]

    if not sizes_only and (durations := current.get("build_seconds")):
        lines += [
            "",
            "### Build time",
            "",
            (
                "Elapsed `cargo build` time for the current revision, excluding "
                "job setup, cache transfers, and artifact uploads. The `uv` build includes `uvx`. "
                "PGO profile preparation runs in a separate job."
            ),
            "",
            "| Build | Seconds |",
            "| --- | ---: |",
            f"| `uv` and `uvx` | {durations['uv']:.3f} |",
            f"| `uv-build` | {durations['uv-build']:.3f} |",
        ]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    training = commands.add_parser("train")
    training.add_argument("--source", type=Path, required=True)
    training.add_argument("--output", type=Path, required=True)
    build_command = commands.add_parser("build")
    build_command.add_argument("--source", type=Path, required=True)
    build_command.add_argument("--pgo-profile", type=Path, required=True)
    build_command.add_argument("--output", type=Path, required=True)
    measurement = commands.add_parser("measure")
    measurement.add_argument("--source", type=Path, required=True)
    measurement.add_argument("--pgo-profile", type=Path, required=True)
    measurement.add_argument("--output", type=Path, required=True)
    measurement.add_argument("--build-times", type=Path)
    summary = commands.add_parser("report")
    summary.add_argument("current", type=Path)
    summary.add_argument("--base", type=Path)
    summary.add_argument("--repository-url", required=True)
    summary.add_argument("--sizes-only", action="store_true")
    args = parser.parse_args()

    if args.command == "train":
        profile = train(args.source, args.output)
        (args.output / "profile.json").write_text(json.dumps(profile, indent=2) + "\n")
    elif args.command == "build":
        args.output.write_text(
            json.dumps(build(args.source, args.pgo_profile), indent=2) + "\n"
        )
    elif args.command == "measure":
        durations = (
            json.loads(args.build_times.read_text()) if args.build_times else None
        )
        result = measure(args.source, args.pgo_profile, durations)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    elif args.command == "report":
        current = json.loads(args.current.read_text())
        base = json.loads(args.base.read_text()) if args.base else None
        print(
            report(
                current,
                base,
                repository_url=args.repository_url,
                sizes_only=args.sizes_only,
            ),
            end="",
        )


if __name__ == "__main__":
    main()
