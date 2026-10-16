#!/usr/bin/env python3
"""Interrupt a real wheel upgrade and compare retry/repair with a clean install.

    python scripts/research/installation_recovery.py --uv target/debug/uv \
        --work-dir work/recovery --boundary install

The experiment stops and kills uv after observing either removal of an old
payload or creation of a new payload. A missed interruption is an error. It first
attempts an ordinary retry and then runs `--reinstall`. It compares each result
with an independent clean installation, including `RECORD` and absence of old payload files.
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
        for path in root.rglob("*"):
            if path.is_file():
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
        process = subprocess.Popen(arguments, stdout=output, stderr=output, text=True)
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
    subprocess.run(command(uv, old, target, cache), check=True, capture_output=True)

    # Prepare the replacement in the same cache before selecting an installation boundary.
    subprocess.run(
        command(uv, new, root / "prepared", cache), check=True, capture_output=True
    )
    interruption = interrupt(
        command(uv, new, target, cache), target, args.boundary, root / "interrupted.log"
    )
    interrupted = snapshot(target)
    retry = subprocess.run(
        command(uv, new, target, cache), capture_output=True, text=True, check=False
    )
    retried = snapshot(target)
    repair = subprocess.run(
        [*command(uv, new, target, cache), "--reinstall"],
        capture_output=True,
        text=True,
        check=False,
    )
    repaired = snapshot(target)
    clean_target = root / "clean"
    subprocess.run(
        command(uv, new, clean_target, root / "fresh-cache"),
        check=True,
        capture_output=True,
    )
    clean = snapshot(clean_target)
    retry_difference = difference(retried, clean)
    repair_difference = difference(repaired, clean)
    report = {
        "uv": binary_identity(uv),
        "oracle": "explicit reinstall recreates the clean package tree, including RECORD",
        "fault_model": "process interruption only; no power-loss simulation",
        "interruption": interruption,
        "interrupted_file_count": len(interrupted),
        "clean_file_count": len(clean),
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
    clean_repair = repair.returncode == 0 and not any(repair_difference.values())
    residue = (
        repair.returncode == 0
        and not repair_difference["missing"]
        and not repair_difference["changed"]
        and bool(repair_difference["unexpected"])
    )
    return 0 if (clean_repair if args.expect == "clean" else residue) else 1


if __name__ == "__main__":
    raise SystemExit(main())
