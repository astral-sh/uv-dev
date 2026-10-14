"""Measure native Linux release binaries and summarize changes against a baseline."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
BINARIES = {"uv": "release", "uvx": "release", "uv-build": "minimal-size"}


def command_output(command: list[str], source: Path) -> str:
    return subprocess.check_output(command, cwd=source, text=True).strip()


def measure(source: Path) -> dict:
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
    }


def report(current: dict, base: dict | None) -> str:
    if current["target"] != TARGET or (base and base["target"] != TARGET):
        raise ValueError(f"Expected measurements for {TARGET}")

    lines = [
        "## Binary size",
        "",
        f"Target: `{TARGET}`. Current revision: `{current['revision']}`.",
        "",
        (
            "Native Cargo builds with `release` for `uv` and `uvx`, and `minimal-size` "
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
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    measurement = commands.add_parser("measure")
    measurement.add_argument("--source", type=Path, required=True)
    measurement.add_argument("--output", type=Path, required=True)
    summary = commands.add_parser("report")
    summary.add_argument("current", type=Path)
    summary.add_argument("--base", type=Path)
    args = parser.parse_args()

    if args.command == "measure":
        args.output.write_text(json.dumps(measure(args.source), indent=2) + "\n")
    elif args.command == "report":
        current = json.loads(args.current.read_text())
        base = json.loads(args.base.read_text()) if args.base else None
        print(report(current, base), end="")


if __name__ == "__main__":
    main()
