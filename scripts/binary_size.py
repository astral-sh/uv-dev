"""Measure native Linux release binaries and summarize changes against a baseline."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
BINARIES = {"uv": "release", "uvx": "release", "uv-build": "minimal-size"}


def command_output(command: list[str], source: Path) -> str:
    return subprocess.check_output(command, cwd=source, text=True).strip()


def build(source: Path) -> dict[str, float]:
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
    durations = {}
    for binary, (directory, arguments) in commands.items():
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
            check=True,
        )
        durations[binary] = round(time.perf_counter() - start, 3)
    return durations


def measure(
    source: Path,
    build_seconds: dict | None = None,
    cache_hit: bool = False,
    incremental_cache: str = "miss",
) -> dict:
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
        "build_seconds": build_seconds,
        "rust_cache_hit": cache_hit,
        "incremental_cache": incremental_cache,
    }


def report(current: dict, base: dict | None, *, sizes_only: bool = False) -> str:
    if current["target"] != TARGET or (base and base["target"] != TARGET):
        raise ValueError(f"Expected measurements for {TARGET}")

    lines = [
        "## Binary size",
        "",
        f"Target: `{TARGET}`. Current revision: `{current['revision']}`.",
        "",
        (
            "Incremental native Cargo builds with `release` for `uv` and `uvx`, and `minimal-size` "
            "for `uv-build`. Sizes are uncompressed bytes. These builds exclude the "
            "release pipeline's PGO, SBOM embedding, and manylinux packaging."
        ),
        "",
    ]
    if base:
        lines += [
            f"Baseline revision: `{base['revision']}`.",
            "",
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

    lines += ["", "Compiler versions:", ""]
    for binary in ("uv", "uv-build"):
        compiler = current["rustc"][binary]
        line = f"- `{binary}`: `{compiler}`"
        if base and base["rustc"][binary] != compiler:
            line += f" (baseline: `{base['rustc'][binary]}`)"
        lines.append(line)

    if not sizes_only and (durations := current.get("build_seconds")):
        cache_match = "yes" if current["rust_cache_hit"] else "no"
        lines += [
            "",
            "### Build time",
            "",
            f"Exact Rust cache match: {cache_match}.",
            f"Incremental cache: {current['incremental_cache']}.",
            "",
            (
                "Elapsed `cargo build` time for the current revision, excluding setup, "
                "cache transfers, and artifact uploads. The `uv` build includes `uvx`. "
                "An exact cache match can still require recompiling changed crates."
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
    build_command = commands.add_parser("build")
    build_command.add_argument("--source", type=Path, required=True)
    build_command.add_argument("--output", type=Path, required=True)
    measurement = commands.add_parser("measure")
    measurement.add_argument("--source", type=Path, required=True)
    measurement.add_argument("--output", type=Path, required=True)
    measurement.add_argument("--build-times", type=Path)
    measurement.add_argument("--cache-hit", choices=["true", "false"], default="false")
    measurement.add_argument(
        "--incremental-cache", choices=["hit", "partial", "miss"], default="miss"
    )
    summary = commands.add_parser("report")
    summary.add_argument("current", type=Path)
    summary.add_argument("--base", type=Path)
    summary.add_argument("--sizes-only", action="store_true")
    args = parser.parse_args()

    if args.command == "build":
        args.output.write_text(json.dumps(build(args.source), indent=2) + "\n")
    elif args.command == "measure":
        durations = (
            json.loads(args.build_times.read_text()) if args.build_times else None
        )
        result = measure(
            args.source, durations, args.cache_hit == "true", args.incremental_cache
        )
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    elif args.command == "report":
        current = json.loads(args.current.read_text())
        base = json.loads(args.base.read_text()) if args.base else None
        print(report(current, base, sizes_only=args.sizes_only), end="")


if __name__ == "__main__":
    main()
