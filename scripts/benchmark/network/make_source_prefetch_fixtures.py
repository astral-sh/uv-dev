"""Create a source-only candidate discarded after a direct dependency resolves."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import io
import json
import random
import tarfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", Path(__file__).with_name("make_scheduling_fixtures.py")
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def source(directory: Path, version: int, payload_size: int) -> dict:
    name = "uv-bench-source-choice"
    stem = name.replace("-", "_")
    metadata = (
        f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}.0\n"
        "Requires-Python: >=3.8\n"
    ).encode()
    pyproject = (
        f'[project]\nname = "{name}"\nversion = "{version}.0"\n'
        'requires-python = ">=3.8"\ndependencies = []\n'
    ).encode()
    contents = {
        "PKG-INFO": metadata,
        "pyproject.toml": pyproject,
        "payload.bin": random.Random(version).randbytes(payload_size),
    }
    path = directory / f"{stem}-{version}.0.tar.gz"
    with (
        path.open("wb") as output,
        gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as gzip_file,
        tarfile.open(
            fileobj=gzip_file, mode="w", format=tarfile.USTAR_FORMAT
        ) as archive,
    ):
        for name, data in sorted(contents.items()):
            member = tarfile.TarInfo(f"{stem}-{version}.0/{name}")
            member.size = len(data)
            member.mode = 0o644
            archive.addfile(member, io.BytesIO(data))
    return {
        "filename": path.name,
        "url": path.resolve().as_uri(),
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "size": path.stat().st_size,
        "pep658": False,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--payload-bytes", type=int, default=4 * 1024 * 1024)
    args = parser.parse_args()
    if args.payload_bytes < 0:
        parser.error("--payload-bytes must be nonnegative")
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = [
        scheduling.wheel(
            args.directory, "uv-bench-source-gate", 1, "uv-bench-source-choice==1.0"
        ),
        source(args.directory, 1, 0),
        source(args.directory, 2, args.payload_bytes),
    ]
    gate_path = "/files/uv_bench_source_gate-1.0-py3-none-any.whl.metadata"
    unused_path = "/files/uv_bench_source_choice-2.0.tar.gz"
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {gate_path: 750},
        },
        "constrained": {
            "latency_ms": 250,
            "bytes_per_second": 125000,
            "path_latency_ms": {gate_path: 750},
        },
        "uniform": {"latency_ms": 150, "bytes_per_second": 1250000},
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {gate_path: 750},
            "path_failures": {unused_path: {"status": 503, "count": 2}},
        },
    }
    sidecars = [
        {key: value for key, value in item.items() if key != "pep658"}
        for item in manifest
    ]
    for name, value in (
        ("source-prefetch-fixtures.json", manifest),
        ("source-prefetch-sidecars.json", sidecars),
        ("source-prefetch-profiles.json", profiles),
    ):
        (args.directory / name).write_text(json.dumps(value, indent=2) + "\n")
    print(
        json.dumps({"distributions": len(manifest), "directory": str(args.directory)})
    )


if __name__ == "__main__":
    main()
