#!/usr/bin/env python3
"""Compare executable permissions for identical wheel payloads in shared caches.

Run with a development uv binary and a new work directory::

    python scripts/research/archive_executable_identity.py \
        --uv target/debug/uv --work-dir work/archive-identity

The experiment enables the `content-addressed-cache` preview. Only the ZIP
executable bit differs between the two wheel files. Each order uses
its own shared cache, with independent caches as controls. This probes archive
identity through the actual installer; it does not model cache behavior.
The payload uses a `.so` suffix to enter the shared file-object store regardless
of executable status. Reports include archive and object identities, preview-off
controls, and refresh versus package-cache-clean behavior. An optional
`--candidate-uv` must also read the baseline-populated cache correctly.
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


def run(command: list[str]) -> subprocess.CompletedProcess:
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    result = subprocess.run(
        command, env=environment, capture_output=True, text=True, check=False
    )
    if result.returncode:
        print(result.stderr, file=sys.stderr, end="")
        result.check_returncode()
    return result


def wheel(directory: Path, executable: bool) -> Path:
    directory.mkdir()
    path = directory / "mode_probe-1.0.0-py3-none-any.whl"
    files = {
        "mode_probe/probe.so": b"#!/bin/sh\nexit 0\n",
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
            mode = 0o755 if executable and name == "mode_probe/probe.so" else 0o644
            entry.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(entry, contents)
    return path


def install(
    uv: str,
    root: Path,
    name: str,
    artifact: Path,
    cache: str,
    *,
    preview: bool = True,
    refresh: bool = False,
) -> dict:
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
        "--link-mode",
        "copy",
        "--target",
        str(target),
        str(artifact),
    ]
    if preview:
        command.extend(["--preview-features", "content-addressed-cache"])
    if refresh:
        command.extend(["--refresh-package", "mode-probe"])
    run(command)
    installed = target / "mode_probe/probe.so"
    mode = stat.S_IMODE(installed.stat().st_mode)
    contents = installed.read_bytes()
    archive_ids = sorted(
        path.relative_to(root / cache).as_posix()
        for bucket in (root / cache).glob("archive-v*")
        for path in bucket.iterdir()
        if path.is_dir()
    )
    objects = []
    for bucket in (root / cache).glob("files-v*"):
        for path in bucket.rglob("*"):
            if path.is_file() and path.read_bytes() == contents:
                objects.append(
                    {
                        "id": path.relative_to(root / cache).as_posix(),
                        "executable": bool(path.stat().st_mode & 0o111),
                    }
                )
    return {
        "mode": oct(mode),
        "executable": bool(mode & 0o111),
        "sha256": hashlib.sha256(contents).hexdigest(),
        "archive_count": len(archive_ids),
        "archive_ids": archive_ids,
        "file_objects": sorted(objects, key=lambda item: item["id"]),
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
    parser.add_argument("--candidate-uv")
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
    candidate = shutil.which(args.candidate_uv) if args.candidate_uv else uv
    if candidate is None:
        parser.error(f"candidate executable not found: {args.candidate_uv}")
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
    preview_off = {
        "plain": install(uv, root, "off-plain", plain, "off-cache", preview=False),
        "executable": install(
            uv, root, "off-exec", executable, "off-cache", preview=False
        ),
    }
    refresh_seed = {
        "plain": install(uv, root, "refresh-seed-plain", plain, "refresh-cache"),
        "executable": install(
            uv, root, "refresh-seed-exec", executable, "refresh-cache"
        ),
    }
    plain_refresh_seed = {
        "executable": install(
            uv, root, "plain-refresh-seed-exec", executable, "plain-refresh-cache"
        ),
        "plain": install(
            uv, root, "plain-refresh-seed-plain", plain, "plain-refresh-cache"
        ),
    }
    existing_pointer = install(
        candidate, root, "existing-pointer", executable, "forward-cache"
    )
    refreshed_pointer = install(
        candidate, root, "refreshed-pointer", executable, "refresh-cache", refresh=True
    )
    existing_plain_pointer = install(
        candidate, root, "existing-plain-pointer", plain, "reverse-cache"
    )
    refreshed_plain_pointer = install(
        candidate,
        root,
        "refreshed-plain-pointer",
        plain,
        "plain-refresh-cache",
        refresh=True,
    )
    run(
        [
            candidate,
            "--no-config",
            "--cache-dir",
            str(root / "forward-cache"),
            "cache",
            "clean",
            "mode-probe",
        ],
    )
    after_clean = install(candidate, root, "after-clean", executable, "forward-cache")
    assert not controls["plain"]["executable"], "plain control is executable"
    assert controls["executable"]["executable"], "executable control lost its mode"
    assert controls["plain"]["sha256"] == controls["executable"]["sha256"]
    assert len(controls["plain"]["file_objects"]) == 1
    assert len(controls["executable"]["file_objects"]) == 1
    assert (
        controls["plain"]["file_objects"][0]["id"]
        != controls["executable"]["file_objects"][0]["id"]
    ), "file-object identity conflates executable status"
    assert not preview_off["plain"]["executable"]
    assert preview_off["executable"]["executable"]
    assert after_clean["executable"], (
        "fresh extraction after cache clean lost executable status"
    )
    conflation = (
        not forward["executable"]["executable"] and reverse["plain"]["executable"]
    )
    correct = forward["executable"]["executable"] and not reverse["plain"]["executable"]
    migration_correct = (
        existing_pointer["executable"]
        and refreshed_pointer["executable"]
        and not existing_plain_pointer["executable"]
        and not refreshed_plain_pointer["executable"]
    )
    report = {
        "uv": binary_identity(uv),
        "controls": controls,
        "plain_then_executable": forward,
        "executable_then_plain": reverse,
        "preview_disabled": preview_off,
        "candidate": binary_identity(candidate),
        "existing_pointer": existing_pointer,
        "refresh_cache_seed": refresh_seed,
        "refreshed_pointer": refreshed_pointer,
        "existing_plain_pointer": existing_plain_pointer,
        "plain_refresh_cache_seed": plain_refresh_seed,
        "refreshed_plain_pointer": refreshed_plain_pointer,
        "after_package_cache_clean": after_clean,
        "candidate_reuses_seeded_cache_correctly": migration_correct,
        "order_dependent_permissions": conflation,
    }
    print(json.dumps(report, indent=2))
    expected = conflation if args.expect == "conflation" else correct
    return 0 if expected and (not args.candidate_uv or migration_correct) else 1


if __name__ == "__main__":
    raise SystemExit(main())
