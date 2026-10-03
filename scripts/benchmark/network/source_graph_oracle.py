"""Fetch a known selected graph through sidecars or verified distribution archives."""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import struct
import threading
import time
import urllib.request
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def metadata_bytes(fixtures: bench.Fixtures, profile: dict, entry: dict) -> int:
    """Return an optimistic body-byte bound for the selected metadata."""
    filename = entry["filename"]
    if entry["core-metadata"]:
        return len(fixtures.metadata[filename + ".metadata"])
    path = fixtures.files[filename]
    if not filename.endswith(".whl"):
        return path.stat().st_size
    with zipfile.ZipFile(path) as archive:
        entries = [
            value
            for value in archive.infolist()
            if value.filename.endswith(".dist-info/METADATA")
        ]
        if len(entries) != 1:
            raise ValueError("Expected one wheel metadata entry")
        metadata = entries[0]
    if profile.get("ranges", True):
        return metadata.compress_size
    with path.open("rb") as source:
        source.seek(metadata.header_offset)
        header = source.read(30)
    if header[:4] != b"PK\x03\x04":
        raise ValueError("Invalid ZIP local header")
    name_size, extra_size = struct.unpack_from("<HH", header, 26)
    return metadata.header_offset + 30 + name_size + extra_size + metadata.compress_size


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    selected = {
        entry["filename"]: (name, entry)
        for name, entries in fixtures.packages.items()
        for entry in entries
        if entry["filename"] in args.filename
    }
    if set(selected) != set(args.filename):
        parser.error("every selected filename must appear in the manifest")
    if len({name for name, _ in selected.values()}) != len(selected):
        parser.error("select only one version per package")
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def read(path: str, expected: bytes) -> None:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(server.url + path, timeout=60) as response:
            if response.read() != expected:
                raise ValueError(f"Oracle response differs: {path}")

    def paths(filename: str, package: str, entry: dict) -> list[tuple[str, bytes]]:
        path = entry["url"]
        if entry["core-metadata"]:
            path += ".metadata"
            payload = fixtures.metadata[filename + ".metadata"]
        else:
            payload = fixtures.files[filename].read_bytes()
        return [(f"/simple/{package}/", fixtures.simple[package]), (path, payload)]

    requests = [paths(filename, *entry) for filename, entry in selected.items()]

    def fetch(items: list[tuple[str, bytes]]) -> None:
        for path, expected in items:
            read(path, expected)

    started = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(requests)) as pool:
            list(pool.map(fetch, requests))
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(len(body) for items in requests for _, body in items)
    required_metadata_bytes = sum(
        len(fixtures.simple[package]) + metadata_bytes(fixtures, profile, entry)
        for package, entry in selected.values()
    )
    required_latency = max(
        sum(
            max(
                0,
                profile.get("path_latency_ms", {}).get(
                    path, profile.get("latency_ms", 0)
                )
                - profile.get("jitter_ms", 0),
            )
            for path, _ in items
        )
        for items in requests
    )
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filenames": args.filename,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_metadata_bytes": required_metadata_bytes,
        "required_waves": 2,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 2, required_latency
        ),
        "optimistic_metadata_floor_seconds": bench.network_floor(
            profile, required_metadata_bytes, 2, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known selected versions with unlimited concurrency. Read each index, then its advertised metadata or complete distribution archive, verifying all response bytes. This is a realizable retrieval strategy; the full-archive byte count is not a universal lower bound for metadata discovery. The separate metadata bound charges a known compressed ZIP entry when ranges work, the required wheel prefix otherwise, and complete source archives. Both floors exclude headers, TCP/TLS, extraction, resolution, and CPU work.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in result.items() if key != "events"}, indent=2
        )
    )


if __name__ == "__main__":
    main()
