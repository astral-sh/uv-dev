#!/usr/bin/env python3
"""Exercise evidence-cache and identity policy boundaries with a real signed wheel."""

from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
from pathlib import Path


def consumer_report(process: subprocess.CompletedProcess[str]) -> dict:
    lines = process.stderr.splitlines()
    payload = process.stdout if process.returncode == 0 else lines[-1] if lines else ""
    try:
        report = json.loads(payload)
    except json.JSONDecodeError:
        report = None
    if isinstance(report, dict):
        return report
    raise RuntimeError(
        f"Consumer exited with status {process.returncode}; the expected report was not a JSON object.\n"
        f"stdout:\n{process.stdout}\nstderr:\n{process.stderr}"
    )


def run_concurrently(
    command: list[str], timeout: float = 120
) -> list[subprocess.CompletedProcess[str]]:
    processes: list[subprocess.Popen[str]] = []
    try:
        for _ in range(2):
            processes.append(
                subprocess.Popen(
                    command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE
                )
            )
        results = []
        for process in processes:
            stdout, stderr = process.communicate(timeout=timeout)
            results.append(
                subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
            )
        return results
    except BaseException:
        for process in processes:
            if process.poll() is None:
                process.kill()
        for process in processes:
            process.communicate()
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument("--identity", required=True)
    parser.add_argument("--issuer", required=True)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--staging", action="store_true")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    args.work_dir.mkdir(parents=True, exist_ok=False)
    cache = args.work_dir / "evidence-cache"
    consumer = Path(__file__).with_name("provenance_install.py")
    common = [
        sys.executable,
        str(consumer),
        "--uv",
        args.uv,
        "--evidence-cache",
        str(cache),
        "--work-dir",
        str(args.work_dir),
    ]
    for flag in ("staging", "offline"):
        if getattr(args, flag):
            common.append("--" + flag)
    observations: list[dict] = []

    def run(
        name: str,
        *,
        artifact: Path = args.artifact,
        evidence: Path = args.attestation,
        identity: str = args.identity,
        issuer: str = args.issuer,
        success: bool = False,
        cache_status: str | None = None,
    ) -> None:
        target = args.work_dir / name
        command = common + [
            str(artifact),
            "--attestation",
            str(evidence),
            "--identity",
            identity,
            "--issuer",
            issuer,
            "--target",
            str(target),
        ]
        process = subprocess.run(
            command, text=True, capture_output=True, check=False, timeout=120
        )
        report = consumer_report(process)
        if (process.returncode == 0) != success or target.exists() != success:
            raise AssertionError(f"Unexpected {name}: {report}")
        if success:
            if not report["cryptographic_verification_ran"]:
                raise AssertionError(f"Verification skipped: {report}")
        elif report["error_type"] not in {"VerificationError", "ValueError"}:
            raise AssertionError(f"Infrastructure failure in {name}: {report}")
        if cache_status is not None and report["evidence_cache_status"] != cache_status:
            raise AssertionError(f"Unexpected cache state: {report}")
        observations.append(
            {
                "case": name,
                "returncode": process.returncode,
                "target_exists": target.exists(),
                "report": report,
            }
        )

    run("cold", success=True, cache_status="miss")
    run("warm", success=True, cache_status="hit")
    # Corrupt only the cached copy; the supplied signed evidence remains authoritative.
    cached = list(cache.glob("*.json"))
    if len(cached) != 1:
        raise AssertionError(f"Expected one evidence cache entry: {cached}")
    cached[0].write_bytes(b"invalid cached evidence")
    run("repaired-cache", success=True, cache_status="repaired")
    run("wrong-identity", identity="untrusted-fixture-identity", cache_status="miss")
    run(
        "wrong-identity-warm", identity="untrusted-fixture-identity", cache_status="hit"
    )
    run("wrong-issuer", issuer="https://invalid.example/issuer")

    artifact = args.work_dir / args.artifact.name
    artifact.write_bytes(args.artifact.read_bytes() + b"tampered")
    run("tampered-artifact", artifact=artifact)
    evidence = json.loads(args.attestation.read_bytes())
    if "attestation_bundles" in evidence:
        attestation = evidence["attestation_bundles"][0]["attestations"][0]
    else:
        attestation = evidence
    signature = bytearray(base64.b64decode(attestation["envelope"]["signature"]))
    signature[-1] ^= 1
    attestation["envelope"]["signature"] = base64.b64encode(signature).decode()
    altered = args.work_dir / "altered-attestation.json"
    altered.write_text(json.dumps(evidence))
    run("tampered-evidence", evidence=altered)
    empty = args.work_dir / "empty-provenance.json"
    empty.write_text('{"attestation_bundles": []}')
    run("empty-evidence", evidence=empty)
    run("warm-after-negatives", success=True, cache_status="hit")
    target = args.work_dir / "concurrent-target"
    command = common + [
        str(args.artifact),
        "--attestation",
        str(args.attestation),
        "--identity",
        args.identity,
        "--issuer",
        args.issuer,
        "--target",
        str(target),
    ]
    concurrent: list[dict] = []
    for process in run_concurrently(command):
        report = consumer_report(process)
        if process.returncode and report["error_type"] not in {
            "ValueError",
            "FileExistsError",
        }:
            raise AssertionError(
                f"Unexpected concurrent installation failure: {report}"
            )
        concurrent.append({"returncode": process.returncode, "report": report})
    if sorted(row["returncode"] for row in concurrent) != [0, 1] or not target.is_dir():
        raise AssertionError(f"Target was not reserved exclusively: {concurrent}")
    observations.append({"case": "concurrent-target", "results": concurrent})
    print(json.dumps({"observations": observations}, indent=2))


if __name__ == "__main__":
    main()
