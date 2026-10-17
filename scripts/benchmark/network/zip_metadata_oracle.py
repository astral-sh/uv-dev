"""Measure a wheel metadata transfer with its ZIP entry location already known."""

from __future__ import annotations

import argparse
import hashlib
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=bench.HERE / "fixtures.json")
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=bench.HERE / "profiles.json")
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    wheel = fixtures.files[args.filename]
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
        start = entry.header_offset if profile.get("ranges", True) else 0
        source.seek(start)
        expected = source.read(end - start)
    required_bytes = entry.compress_size if profile.get("ranges", True) else end
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    headers = (
        {"Range": f"bytes={start}-{end - 1}"} if profile.get("ranges", True) else {}
    )
    request = urllib.request.Request(
        server.url + f"/files/{args.filename}", headers=headers
    )
    started = time.perf_counter()
    try:
        with loopback.open(request, timeout=120) as response:
            expected_status = 206 if profile.get("ranges", True) else 200
            if (
                response.status != expected_status
                or response.read(len(expected)) != expected
            ):
                raise ValueError("Oracle response differs from the pinned ZIP bytes")
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "metadata_sha256": hashlib.sha256(metadata).hexdigest(),
        "metadata_bytes": len(metadata),
        "compressed_metadata_bytes": entry.compress_size,
        "fetch_start": start,
        "fetch_end": end,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": 1,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 1
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known wheel and metadata ZIP offset. With range support, fetch and verify the local header and compressed metadata; the optimistic bound charges only the compressed metadata bytes. Without ranges, read and verify the required archive prefix. ZIP discovery and metadata parsing are excluded.",
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
