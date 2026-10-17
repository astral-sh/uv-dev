#!/usr/bin/env python3
"""Compare module-conflict diagnostics on controlled wheel collisions."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path
from typing import TypedDict


class Observation(TypedDict):
    case: str
    binary: str
    repeat: int
    first_bytes: int
    second_bytes: int
    expected_conflict: bool
    warned: bool
    seconds: float


def wheel(path: Path, name: str, shared_path: str, payload: bytes) -> Path:
    normalized = name.replace("-", "_")
    target = path / f"{normalized}-1.0-py3-none-any.whl"
    prefix = f"{normalized}-1.0.dist-info/"
    with zipfile.ZipFile(target, "w") as archive:
        for filename, data in {
            shared_path: payload,
            prefix
            + "METADATA": f"Metadata-Version: 2.2\nName: {name}\nVersion: 1.0\n".encode(),
            prefix
            + "WHEEL": b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
            prefix + "RECORD": b"",
        }.items():
            archive.writestr(zipfile.ZipInfo(filename), data)
    return target


def measure(
    binaries: dict[str, str],
    workspace: Path,
    large_bytes: int,
    repeats: int,
    check: bool,
) -> dict:
    small = b"#" * 8192 + b"1"
    large = b"\0" * (large_bytes - 1) + b"1"
    cases = [
        ("empty-namespace", "namespace/__init__.py", b"", b""),
        ("identical-module", "shared.py", small, small),
        ("different-module-tail", "shared.py", small, small[:-1] + b"2"),
        ("identical-binary", "shared.abi3.so", large, large),
        ("different-binary-tail", "shared.abi3.so", large, large[:-1] + b"2"),
        ("different-binary-size", "shared.abi3.so", large, large + b"2"),
    ]
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    observations: list[Observation] = []
    identities = {}
    for label, binary in binaries.items():
        with open(binary, "rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        identities[label] = {
            "path": binary,
            "sha256": digest,
            "version": subprocess.check_output(
                [binary, "--version"], text=True
            ).strip(),
        }
    for case, shared_path, first, second in cases:
        directory = workspace / case
        directory.mkdir()
        first_wheel = wheel(directory, "collision-a", shared_path, first)
        second_wheel = wheel(directory, "collision-b", shared_path, second)
        for label, binary in binaries.items():
            for repeat in range(repeats):
                run_dir = directory / f"{label}-{repeat}"
                started = time.perf_counter()
                process = subprocess.run(
                    [
                        binary,
                        "--no-config",
                        "pip",
                        "install",
                        "--no-index",
                        "--no-deps",
                        "--preview-features",
                        "detect-module-conflicts",
                        "--python",
                        sys.executable,
                        "--target",
                        str(run_dir / "installed"),
                        "--cache-dir",
                        str(run_dir / "cache"),
                        str(first_wheel),
                        str(second_wheel),
                    ],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=60,
                )
                seconds = time.perf_counter() - started
                warned = (
                    f"The file `{shared_path}` is provided by more than one package"
                    in process.stderr
                )
                if process.returncode:
                    raise RuntimeError(process.stderr)
                installed = (run_dir / "installed" / shared_path).read_bytes()
                if installed not in (first, second):
                    raise AssertionError(
                        "Installed payload is not either complete input file"
                    )
                expected = first != second
                if check and label == "candidate" and warned != expected:
                    raise AssertionError(
                        f"{case}: expected warning={expected}, got {process.stderr}"
                    )
                observations.append(
                    {
                        "case": case,
                        "binary": label,
                        "repeat": repeat,
                        "first_bytes": len(first),
                        "second_bytes": len(second),
                        "expected_conflict": expected,
                        "warned": warned,
                        "seconds": seconds,
                    }
                )
    summaries = []
    for case, _, _, _ in cases:
        for label in binaries:
            rows = [
                row
                for row in observations
                if row["case"] == case and row["binary"] == label
            ]
            summaries.append(
                {
                    "case": case,
                    "binary": label,
                    "median_seconds": statistics.median(row["seconds"] for row in rows),
                    "missed_conflicts": sum(
                        row["expected_conflict"] and not row["warned"] for row in rows
                    ),
                    "false_positives": sum(
                        not row["expected_conflict"] and row["warned"] for row in rows
                    ),
                }
            )
    return {
        "binaries": identities,
        "python": sys.version,
        "observations": observations,
        "summaries": summaries,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--large-bytes", type=int, default=1024 * 1024 + 1)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument(
        "--check", action="store_true", help="Require correct candidate diagnostics"
    )
    args = parser.parse_args()
    if not 8193 <= args.large_bytes <= 64 * 1024 * 1024 or args.repeats < 1:
        parser.error("Use 8193..67108864 large bytes and at least one repeat")
    binaries = {}
    for label in ("baseline", "candidate"):
        executable = shutil.which(getattr(args, label))
        if executable is None:
            parser.error(f"{label} executable not found")
        binaries[label] = str(Path(executable).resolve())
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="module-conflicts-", dir=args.work_dir
    ) as directory:
        print(
            json.dumps(
                measure(
                    binaries,
                    Path(directory).resolve(),
                    args.large_bytes,
                    args.repeats,
                    args.check,
                ),
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
