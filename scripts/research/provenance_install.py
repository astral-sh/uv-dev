#!/usr/bin/env python3
"""Verify local PEP 740 evidence before an isolated, hash-checked uv wheel install."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from importlib.metadata import version
from pathlib import Path

from pypi_attestations import Attestation, Distribution
from sigstore.verify import policy


def evidence_objects(data: bytes) -> list[dict]:
    """Accept a PEP 740 attestation or a PyPI provenance object's attestations."""
    document = json.loads(data)
    if "attestation_bundles" not in document:
        return [document]
    bundles = document["attestation_bundles"]
    if not isinstance(bundles, list) or not bundles:
        raise ValueError("Provenance must contain at least one attestation bundle")
    attestations = []
    for bundle in bundles:
        values = bundle.get("attestations")
        if not isinstance(values, list) or not values:
            raise ValueError("Each provenance bundle must contain attestations")
        attestations.extend(values)
    return attestations


def cache_evidence(cache: Path, key: dict, evidence: bytes) -> str:
    """Cache bytes, never a cryptographic acceptance decision."""
    identity = hashlib.sha256(json.dumps(key, sort_keys=True).encode()).hexdigest()
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / f"{identity}.json"
    if destination.is_file() and destination.read_bytes() == evidence:
        return "hit"
    status = "repaired" if destination.exists() else "miss"
    with tempfile.NamedTemporaryFile(dir=cache, delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(evidence)
    try:
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return status


def verify_and_install(args: argparse.Namespace) -> dict:
    if args.artifact.suffix != ".whl":
        raise ValueError(
            "This experiment accepts wheels only; source builds are outside its scope"
        )
    if args.target.exists():
        raise ValueError("Use a new target directory for each installation")
    with tempfile.TemporaryDirectory(
        prefix="verified-wheel-", dir=args.work_dir
    ) as directory:
        workspace = Path(directory).resolve()
        artifact = workspace / args.artifact.name
        hasher = hashlib.sha256()
        with args.artifact.open("rb") as source, artifact.open("wb") as snapshot:
            while chunk := source.read(1024 * 1024):
                snapshot.write(chunk)
                hasher.update(chunk)
        digest = hasher.hexdigest()
        evidence = args.attestation.read_bytes()
        identity_policy = {
            "identity": args.identity,
            "issuer": args.issuer,
            "staging": args.staging,
            "offline": args.offline,
        }
        verifier = {name: version(name) for name in ("pypi-attestations", "sigstore")}
        key = {
            "schema": 1,
            "filename": args.artifact.name,
            "artifact_sha256": digest,
            "evidence_sha256": hashlib.sha256(evidence).hexdigest(),
            "policy": identity_policy,
            "verifier": verifier,
        }
        attestations = [
            Attestation.model_validate(value) for value in evidence_objects(evidence)
        ]
        cache_status = cache_evidence(args.evidence_cache, key, evidence)
        args.evidence_cache_status = cache_status
        distribution = Distribution(name=args.artifact.name, digest=digest)
        identity = policy.Identity(identity=args.identity, issuer=args.issuer)
        started = time.perf_counter()
        predicates = []
        for attestation in attestations:
            predicate, _ = attestation.verify(
                identity, distribution, staging=args.staging, offline=args.offline
            )
            predicates.append(predicate)
        verification_seconds = time.perf_counter() - started

        # Verification always runs, even on evidence-cache hits. The verifier owns trust-root
        # refresh/expiry; a persisted acceptance decision must not bypass that lifecycle.
        requirements = workspace / "requirements.txt"
        requirements.write_text(f"{artifact.as_uri()} --hash=sha256:{digest}\n")
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("UV_")
        }
        command = [
            args.uv,
            "--no-config",
            "pip",
            "install",
            "--no-index",
            "--no-deps",
            "--no-build",
            "--require-hashes",
            "--requirements",
            str(requirements),
            "--target",
            str(args.target.resolve()),
            "--python",
            sys.executable,
            "--cache-dir",
            str(args.work_dir.resolve() / "uv-cache"),
        ]
        args.target.mkdir(parents=True, exist_ok=False)
        process = subprocess.run(
            command,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        if process.returncode:
            raise RuntimeError(
                f"Verified artifact installation failed: {process.stderr}"
            )
    with open(args.uv, "rb") as binary:
        uv_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    return {
        "uv_sha256": uv_sha256,
        "uv_executable": str(Path(args.uv).resolve()),
        "uv": subprocess.check_output([args.uv, "--version"], text=True).strip(),
        "artifact_sha256": digest,
        "evidence_cache_hit": cache_status == "hit",
        "evidence_cache_status": cache_status,
        "cryptographic_verification_ran": True,
        "verification_seconds": verification_seconds,
        "verified_attestations": len(attestations),
        "predicates": predicates,
        "policy": identity_policy,
        "verifier": verifier,
        "installed_to": str(args.target.resolve()),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument(
        "--identity",
        required=True,
        help="Expected certificate identity, supplied independently of evidence",
    )
    parser.add_argument("--issuer", required=True, help="Expected OIDC issuer")
    parser.add_argument("--uv", required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--evidence-cache", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Use verifier trust material without refreshing TUF",
    )
    parser.add_argument(
        "--staging",
        action="store_true",
        help="Use Sigstore staging roots (fixtures only)",
    )
    args = parser.parse_args()
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    args.uv = str(Path(uv).resolve())
    args.work_dir.mkdir(parents=True, exist_ok=True)
    try:
        report = verify_and_install(args)
    except Exception as error:
        print(
            json.dumps(
                {
                    "error": str(error),
                    "error_type": type(error).__name__,
                    "evidence_cache_status": getattr(
                        args, "evidence_cache_status", None
                    ),
                }
            ),
            file=sys.stderr,
        )
        raise SystemExit(1) from error
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
