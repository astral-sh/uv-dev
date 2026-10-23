#!/usr/bin/env python3
"""Interrupt a real wheel upgrade and compare retry/repair with a clean install.

    python scripts/research/installation_recovery.py --uv target/debug/uv \
        --work-dir work/recovery --boundary install

The experiment stops and kills uv after observing either removal of an old
payload or creation of a new payload. A missed interruption is an error. It copies
the interrupted target and cache before applying an ordinary retry and
`--reinstall` independently. It compares each result with a clean installation,
including `RECORD` and absence of old payload files.
Exit status 1 means the clean-tree oracle failed; the JSON report contains the
remaining differences. Use `--expect residue` to verify a known leftover-file case.

This is process interruption on the current filesystem, not power-loss testing.
No durability, environment-wide atomicity, or exhaustive schedule claim is made.
"""

import argparse
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time
import zipfile
from pathlib import Path


def environment() -> dict[str, str]:
    return {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }


def wheel(root: Path, generation: int, file_count: int) -> Path:
    version = f"{generation}.0.0"
    path = root / f"recovery_probe-{version}-py3-none-any.whl"
    info = f"recovery_probe-{version}.dist-info"
    files = {
        "recovery_probe/__init__.py": f"VERSION = '{version}'\n".encode(),
        f"{info}/METADATA": (
            f"Metadata-Version: 2.1\nName: recovery-probe\nVersion: {version}\n"
        ).encode(),
        f"{info}/WHEEL": (
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    for index in range(file_count):
        files[f"recovery_probe/generation_{generation}/{index:05}.dat"] = (
            f"generation={generation}, index={index}\n".encode() * 64
        )
    files[f"{info}/RECORD"] = (
        "".join(f"{name},,\n" for name in files) + f"{info}/RECORD,,\n"
    ).encode()
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, contents in files.items():
            archive.writestr(name, contents)
    return path


def command(uv: str, artifact: Path, target: Path, cache: Path) -> list[str]:
    return [
        uv,
        "--no-config",
        "--cache-dir",
        str(cache),
        "pip",
        "install",
        "--no-index",
        "--no-deps",
        "--no-compile-bytecode",
        "--python",
        sys.executable,
        "--link-mode",
        "copy",
        "--target",
        str(target),
        str(artifact),
    ]


def snapshot(target: Path) -> dict[str, str]:
    result = {}
    for root in [target / "recovery_probe", *target.glob("recovery_probe-*.dist-info")]:
        for path in [root, *root.rglob("*")]:
            if path.is_dir():
                result[path.relative_to(target).as_posix() + "/"] = "directory"
            elif path.is_file():
                result[path.relative_to(target).as_posix()] = hashlib.sha256(
                    path.read_bytes()
                ).hexdigest()
    return result


def difference(actual: dict[str, str], expected: dict[str, str]) -> dict:
    return {
        "missing": sorted(expected.keys() - actual.keys()),
        "unexpected": sorted(actual.keys() - expected.keys()),
        "changed": sorted(
            name
            for name in actual.keys() & expected.keys()
            if actual[name] != expected[name]
        ),
    }


def interrupt(arguments: list[str], target: Path, boundary: str, log: Path) -> dict:
    old = target / "recovery_probe/generation_1/00000.dat"
    new = target / "recovery_probe/generation_2/00000.dat"
    assert old.is_file() and not new.exists(), "unexpected initial installation"
    start = time.monotonic()
    observed = False
    with log.open("w") as output:
        process = subprocess.Popen(
            arguments, env=environment(), stdout=output, stderr=output, text=True
        )
        try:
            while process.poll() is None:
                observed = not old.exists() if boundary == "uninstall" else new.exists()
                if observed:
                    os.kill(process.pid, signal.SIGSTOP)
                    os.kill(process.pid, signal.SIGKILL)
                    break
                if time.monotonic() - start > 30:
                    raise TimeoutError("upgrade did not reach the selected boundary")
                time.sleep(0.0001)
        finally:
            if process.poll() is None:
                process.kill()
            returncode = process.wait()
    if not observed or returncode != -signal.SIGKILL:
        raise RuntimeError(
            "missed the selected interruption; increase --files and use a new work directory"
        )
    return {
        "boundary": boundary,
        "returncode": returncode,
        "elapsed_seconds": time.monotonic() - start,
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
    parser.add_argument("--boundary", choices=["uninstall", "install"], required=True)
    parser.add_argument("--files", type=int, default=3000)
    parser.add_argument("--expect", choices=["clean", "residue"], default="clean")
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("this experiment requires POSIX process signals")
    if args.files < 1:
        parser.error("--files must be positive")
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    old = wheel(root, 1, args.files)
    new = wheel(root, 2, args.files)
    target = root / "interrupted"
    cache = root / "shared-cache"
    subprocess.run(
        command(uv, old, target, cache),
        env=environment(),
        check=True,
        capture_output=True,
    )

    # Prepare the replacement in the same cache before selecting an installation boundary.
    subprocess.run(
        command(uv, new, root / "prepared", cache),
        env=environment(),
        check=True,
        capture_output=True,
    )
    interruption = interrupt(
        command(uv, new, target, cache), target, args.boundary, root / "interrupted.log"
    )
    interrupted = snapshot(target)
    retry_target = root / "ordinary-retry"
    repair_target = root / "explicit-reinstall"
    retry_cache = root / "retry-cache"
    repair_cache = root / "repair-cache"
    shutil.copytree(target, retry_target, symlinks=True)
    shutil.copytree(target, repair_target, symlinks=True)
    shutil.copytree(cache, retry_cache, symlinks=True)
    shutil.copytree(cache, repair_cache, symlinks=True)
    for copied_cache in (retry_cache, repair_cache):
        assert all(
            path.resolve().is_relative_to(copied_cache)
            for path in copied_cache.rglob("*")
            if path.is_symlink()
        ), "a copied cache link refers outside its independent cache"
    assert snapshot(retry_target) == interrupted, "retry starting state changed"
    assert snapshot(repair_target) == interrupted, "reinstall starting state changed"
    retry = subprocess.run(
        command(uv, new, retry_target, retry_cache),
        env=environment(),
        capture_output=True,
        text=True,
        check=False,
    )
    retried = snapshot(retry_target)
    repair = subprocess.run(
        [*command(uv, new, repair_target, repair_cache), "--reinstall"],
        env=environment(),
        capture_output=True,
        text=True,
        check=False,
    )
    repaired = snapshot(repair_target)
    assert snapshot(target) == interrupted, (
        "recovery modified the original interrupted state"
    )
    clean_target = root / "clean"
    subprocess.run(
        command(uv, new, clean_target, root / "fresh-cache"),
        env=environment(),
        check=True,
        capture_output=True,
    )
    clean = snapshot(clean_target)
    ghost_target = root / "empty-dist-info-control"
    shutil.copytree(clean_target, ghost_target)
    ghost = ghost_target / "recovery_probe-1.0.0.dist-info"
    ghost.mkdir()
    ghost_difference = difference(snapshot(ghost_target), clean)
    assert ghost_difference["unexpected"] == [ghost.name + "/"]
    ghost_listing = subprocess.run(
        [
            uv,
            "--no-config",
            "--cache-dir",
            str(root / "listing-cache"),
            "pip",
            "list",
            "--target",
            str(ghost_target),
            "--format",
            "json",
        ],
        env=environment(),
        check=True,
        capture_output=True,
        text=True,
    )
    retry_difference = difference(retried, clean)
    repair_difference = difference(repaired, clean)
    report = {
        "uv": binary_identity(uv),
        "oracle": "each recovery independently recreates the clean package tree, including RECORD",
        "fault_model": "process interruption only; no power-loss simulation",
        "interruption": interruption,
        "interrupted_entry_count": len(interrupted),
        "starting_state": {
            "identical_for_both_recovery_paths": True,
            "sha256": hashlib.sha256(
                json.dumps(interrupted, sort_keys=True).encode()
            ).hexdigest(),
            "independent_target_and_cache_copies": True,
        },
        "clean_entry_count": len(clean),
        "empty_dist_info_control": {
            "difference": ghost_difference,
            "installed_distributions": json.loads(ghost_listing.stdout),
        },
        "ordinary_retry": {
            "returncode": retry.returncode,
            "difference": retry_difference,
            "stderr": retry.stderr,
        },
        "explicit_reinstall": {
            "returncode": repair.returncode,
            "difference": repair_difference,
            "stderr": repair.stderr,
        },
    }
    report_path = root / "report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    for outcome in (report["ordinary_retry"], report["explicit_reinstall"]):
        outcome["difference"] = {
            name: {"count": len(paths), "sample": paths[:10]}
            for name, paths in outcome["difference"].items()
        }
    report["full_report"] = str(report_path)
    print(json.dumps(report, indent=2))
    clean = (
        retry.returncode == repair.returncode == 0
        and not any(retry_difference.values())
        and not any(repair_difference.values())
    )
    residue = retry.returncode == repair.returncode == 0 and all(
        not result["missing"] and not result["changed"] and result["unexpected"]
        for result in (retry_difference, repair_difference)
    )
    return 0 if (clean if args.expect == "clean" else residue) else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        if error.stderr:
            if isinstance(error.stderr, bytes):
                sys.stderr.buffer.write(error.stderr)
            else:
                sys.stderr.write(error.stderr)
        raise
