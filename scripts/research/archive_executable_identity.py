#!/usr/bin/env python3
"""Compare executable permissions for identical wheel payloads in shared caches.

Run with a development uv binary and a new work directory::

    python scripts/research/archive_executable_identity.py \
        --uv target/debug/uv --work-dir work/archive-identity

The experiment enables the `content-addressed-cache` preview. Only the ZIP
executable bit differs between the two wheel files. Each order uses
its own shared cache, with independent caches as controls. This probes archive
identity through the actual installer; it does not model cache behavior.
"""

import argparse
import csv
import hashlib
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import zipfile
from pathlib import Path


def wheel(directory: Path, executable: bool) -> Path:
    directory.mkdir()
    path = directory / "mode_probe-1.0.0-py3-none-any.whl"
    files = {
        "mode_probe/probe": b"#!/bin/sh\nexit 0\n",
        "mode_probe-1.0.0.dist-info/METADATA": (
            b"Metadata-Version: 2.1\nName: mode-probe\nVersion: 1.0.0\n"
        ),
        "mode_probe-1.0.0.dist-info/WHEEL": (
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    record = io.StringIO()
    writer = csv.writer(record, lineterminator="\n")
    for name in files:
        writer.writerow((name, "", ""))
    writer.writerow(("mode_probe-1.0.0.dist-info/RECORD", "", ""))
    files["mode_probe-1.0.0.dist-info/RECORD"] = record.getvalue().encode()
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, contents in files.items():
            entry = zipfile.ZipInfo(name)
            entry.create_system = 3
            mode = 0o755 if executable and name == "mode_probe/probe" else 0o644
            entry.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(entry, contents)
    return path


def install(uv: str, root: Path, name: str, artifact: Path, cache: str) -> dict:
    target = root / name
    command = [
        uv,
        "--no-config",
        "--cache-dir",
        str(root / cache),
        "pip",
        "install",
        "--python",
        sys.executable,
        "--no-index",
        "--no-deps",
        "--preview-features",
        "content-addressed-cache",
        "--link-mode",
        "copy",
        "--target",
        str(target),
        str(artifact),
    ]
    subprocess.run(command, check=True, capture_output=True, text=True)
    installed = target / "mode_probe/probe"
    mode = stat.S_IMODE(installed.stat().st_mode)
    return {
        "mode": oct(mode),
        "executable": bool(mode & 0o111),
        "sha256": hashlib.sha256(installed.read_bytes()).hexdigest(),
        "archive_count": sum(
            path.is_dir()
            for bucket in (root / cache).glob("archive-v*")
            for path in bucket.iterdir()
        ),
    }


def binary_identity(executable: str) -> dict:
    with Path(executable).open("rb") as binary:
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    return {
        "path": str(Path(executable).resolve()),
        "sha256": digest,
        "version": subprocess.check_output(
            [executable, "--version"], text=True
        ).strip(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument(
        "--expect", choices=["correct", "conflation"], default="correct"
    )
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("this experiment requires POSIX executable permissions")
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    plain = wheel(root / "plain-wheel", False)
    executable = wheel(root / "executable-wheel", True)
    controls = {
        "plain": install(uv, root, "plain-control", plain, "plain-cache"),
        "executable": install(uv, root, "executable-control", executable, "exec-cache"),
    }
    forward = {
        "plain": install(uv, root, "forward-plain", plain, "forward-cache"),
        "executable": install(uv, root, "forward-exec", executable, "forward-cache"),
    }
    reverse = {
        "executable": install(uv, root, "reverse-exec", executable, "reverse-cache"),
        "plain": install(uv, root, "reverse-plain", plain, "reverse-cache"),
    }
    assert not controls["plain"]["executable"], "plain control is executable"
    assert controls["executable"]["executable"], "executable control lost its mode"
    assert controls["plain"]["sha256"] == controls["executable"]["sha256"]
    conflation = (
        not forward["executable"]["executable"] and reverse["plain"]["executable"]
    )
    correct = forward["executable"]["executable"] and not reverse["plain"]["executable"]
    report = {
        "uv": binary_identity(uv),
        "controls": controls,
        "plain_then_executable": forward,
        "executable_then_plain": reverse,
        "order_dependent_permissions": conflation,
    }
    print(json.dumps(report, indent=2))
    return 0 if (conflation if args.expect == "conflation" else correct) else 1


if __name__ == "__main__":
    raise SystemExit(main())
