"""Create a deterministic backtracking graph for request-scheduling measurements."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import json
import random
import zipfile
from pathlib import Path


def wheel(
    directory: Path,
    name: str,
    version: int,
    requires: str | list[str] | None,
    *,
    build_tag: int | None = None,
    payload_bytes: int = 0,
) -> dict:
    stem = name.replace("-", "_")
    dist_info = f"{stem}-{version}.0.dist-info"
    metadata = (
        f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}.0\n"
        "Requires-Python: >=3.8\n"
    )
    for requirement in [requires] if isinstance(requires, str) else requires or []:
        metadata += f"Requires-Dist: {requirement}\n"
    contents = {
        f"{stem}/__init__.py": f'__version__ = "{version}.0"\n'.encode(),
        f"{dist_info}/METADATA": metadata.encode(),
        f"{dist_info}/WHEEL": (
            b"Wheel-Version: 1.0\nGenerator: uv-network-bench\n"
            b"Root-Is-Purelib: true\nTag: py3-none-any\n"
            + (f"Build: {build_tag}\n".encode() if build_tag is not None else b"")
        ),
    }
    payload_path = f"{stem}/payload.bin"
    if payload_bytes:
        contents[payload_path] = random.Random(version).randbytes(payload_bytes)
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for path, data in sorted(contents.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
        writer.writerow((path, f"sha256={digest.decode()}", len(data)))
    writer.writerow((f"{dist_info}/RECORD", "", ""))
    contents[f"{dist_info}/RECORD"] = record.getvalue().encode()
    build = f"-{build_tag}" if build_tag is not None else ""
    path = directory / f"{stem}-{version}.0{build}-py3-none-any.whl"
    with zipfile.ZipFile(path, "w") as archive:
        for member_name, data in sorted(
            contents.items(), key=lambda item: (item[0] != payload_path, item[0])
        ):
            info = zipfile.ZipInfo(member_name, date_time=(1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data)
    return {
        "filename": path.name,
        "url": path.resolve().as_uri(),
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "size": path.stat().st_size,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [wheel(args.directory, "uv-bench-pin", 1, None)]
    manifest.extend(
        wheel(
            args.directory,
            "uv-bench-choice",
            version,
            f"uv-bench-pin=={2 if version > 15 else 1}.0",
        )
        for version in range(1, 31)
    )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0, "pep658": True},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000, "pep658": True},
        "slow-unused": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "pep658": True,
            "path_latency_ms": {
                f"/files/uv_bench_choice-{version}.0-py3-none-any.whl.metadata": 3000
                for version in range(1, 15)
            },
        },
    }
    for path, value in ((args.manifest, manifest), (args.profiles, profiles)):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps({"wheels": len(manifest), "manifest": str(args.manifest)}))


if __name__ == "__main__":
    main()
