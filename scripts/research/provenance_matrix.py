#!/usr/bin/env python3
"""Exercise evidence-cache and identity policy boundaries with a real signed wheel."""

from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
from pathlib import Path


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
        report = json.loads(
            process.stdout
            if process.returncode == 0
            else process.stderr.splitlines()[-1]
        )
        if (process.returncode == 0) != success or target.exists() != success:
            raise AssertionError(f"Unexpected {name}: {report}")
        if success:
            if not report["cryptographic_verification_ran"]:
                raise AssertionError(f"Verification skipped: {report}")
            if report["evidence_cache_status"] != cache_status:
                raise AssertionError(f"Unexpected cache state: {report}")
        elif report["error_type"] not in {"VerificationError", "ValueError"}:
            raise AssertionError(f"Infrastructure failure in {name}: {report}")
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
    run("wrong-identity", identity="untrusted-fixture-identity")
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
    with (
        subprocess.Popen(
            command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        ) as first,
        subprocess.Popen(
            command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        ) as second,
    ):
        first_output = first.communicate(timeout=120)
        second_output = second.communicate(timeout=120)
    concurrent = []
    for process, (stdout, stderr) in ((first, first_output), (second, second_output)):
        report = json.loads(
            stdout if process.returncode == 0 else stderr.splitlines()[-1]
        )
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
