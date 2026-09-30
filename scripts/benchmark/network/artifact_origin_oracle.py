"""Fetch a known dependency chain across independently configured artifact origins."""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import importlib.util
import json
import struct
import time
import urllib.request
import zipfile
from pathlib import Path
from urllib.parse import urlsplit

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def minimum_latency(profile: dict, path: str) -> float:
    return max(
        0,
        profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
        - profile.get("jitter_ms", 0),
    )


def metadata_transfer(wheel: Path, ranges: bool) -> tuple[int, bytes, int, bytes]:
    with zipfile.ZipFile(wheel) as archive:
        entries = [
            entry
            for entry in archive.infolist()
            if entry.filename.endswith(".dist-info/METADATA")
        ]
        if len(entries) != 1:
            raise ValueError("Expected one wheel metadata entry")
        entry = entries[0]
        metadata = archive.read(entry)
    with wheel.open("rb") as source:
        source.seek(entry.header_offset)
        header = source.read(30)
        if header[:4] != b"PK\x03\x04":
            raise ValueError("Invalid ZIP local header")
        name_size, extra_size = struct.unpack_from("<HH", header, 26)
        end = entry.header_offset + 30 + name_size + extra_size + entry.compress_size
        start = entry.header_offset if ranges else 0
        source.seek(start)
        expected = source.read(end - start)
    return start, expected, entry.compress_size if ranges else end, metadata


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument(
        "--filename",
        action="append",
        required=True,
        help="Wheel filename, in dependency-discovery order",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    replay = bench.Replay(fixtures, profile)
    replay.start()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    required_bytes = 0
    required_latency = 0
    metadata_hashes = {}
    started = time.perf_counter()
    try:
        for filename in args.filename:
            wheel = fixtures.files[filename]
            metadata = fixtures.metadata[filename + ".metadata"]
            headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
            package = bench.normalize(headers["Name"])
            index_path = f"/simple/{package}/"
            expected_index = replay.main.fixtures.simple[package]
            with opener.open(replay.main.url + index_path, timeout=120) as response:
                if response.read() != expected_index:
                    raise ValueError("Oracle index differs from the pinned fixture")
            required_bytes += len(expected_index)
            required_latency += minimum_latency(replay.main.profile, index_path)
            url = replay.file_urls.get(filename, replay.main.url + f"/files/{filename}")
            origin = next(
                server
                for server in replay.servers.values()
                if url.startswith(server.url + "/")
            )
            artifact_path = urlsplit(url).path
            if profile.get("pep658", True):
                url += ".metadata"
                artifact_path += ".metadata"
                request = urllib.request.Request(url)
                expected = metadata
                required = len(metadata)
                expected_status = 200
            else:
                ranges = origin.profile.get("ranges", True)
                start, expected, required, _ = metadata_transfer(wheel, ranges)
                request = urllib.request.Request(
                    url,
                    headers={"Range": f"bytes={start}-{start + len(expected) - 1}"}
                    if ranges
                    else {},
                )
                expected_status = 206 if ranges else 200
            with opener.open(request, timeout=120) as response:
                if (
                    response.status != expected_status
                    or response.read(len(expected)) != expected
                ):
                    raise ValueError("Oracle artifact differs from the pinned fixture")
            required_bytes += required
            required_latency += minimum_latency(origin.profile, artifact_path)
            metadata_hashes[filename] = hashlib.sha256(metadata).hexdigest()
        seconds = time.perf_counter() - started
    finally:
        replay.stop()
    events = replay.events
    waves = len(args.filename) * 2
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filenames": args.filename,
        "metadata_sha256": metadata_hashes,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in events),
        "requests": len(events),
        "events": events,
        "scope": "Known selected versions, artifact capabilities, and metadata ZIP offsets. Read each package index and then its metadata in dependency-discovery order. Ranged-wheel bounds charge compressed metadata only; non-range hosts require the archive prefix. ZIP discovery, parsing, TCP setup, and CPU work are excluded.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in data.items() if key != "events"}, indent=2
        )
    )


if __name__ == "__main__":
    main()
