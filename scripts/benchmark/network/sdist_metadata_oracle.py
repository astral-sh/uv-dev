"""Measure the gzip prefix needed to reach known static source metadata."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import tarfile
import threading
import time
import urllib.request
import zlib
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def metadata_prefix(path: Path) -> tuple[bytes, int, bytes]:
    """Return a compressed prefix, the PKG-INFO offset, and its verified bytes."""
    with tarfile.open(path) as archive:
        entries = [
            entry
            for entry in archive
            if entry.isfile()
            and entry.name.count("/") == 1
            and entry.name.endswith("/PKG-INFO")
        ]
        if len(entries) != 1:
            raise ValueError("Expected one source metadata entry")
        entry = entries[0]
        with archive.extractfile(entry) as source:
            metadata = source.read()
    target = entry.offset_data + entry.size
    compressed = path.read_bytes()
    decoder = zlib.decompressobj(wbits=31)
    produced = consumed = 0
    while produced < target:
        chunk = compressed[consumed : consumed + 65536]
        if not chunk:
            raise ValueError("Gzip ended before the source metadata")
        produced += len(decoder.decompress(chunk, target - produced))
        used = len(chunk) - len(decoder.unconsumed_tail) - len(decoder.unused_data)
        if used <= 0:
            raise ValueError("Gzip decoder made no progress")
        consumed += used
    prefix = compressed[:consumed]
    decoded = zlib.decompressobj(wbits=31).decompress(prefix)
    if decoded[entry.offset_data : target] != metadata:
        raise ValueError("Compressed prefix does not contain the expected metadata")
    return prefix, entry.offset_data, metadata


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
    if profile.get("pep658", True):
        raise ValueError("Use the metadata-sidecar oracle when PEP 658 is advertised")
    fixtures = bench.Fixtures(args.manifest, args.directory, False)
    prefix, offset, metadata = metadata_prefix(fixtures.files[args.filename])
    package = next(
        name
        for name, files in fixtures.packages.items()
        if any(file["filename"] == args.filename for file in files)
    )
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    started = time.perf_counter()
    try:
        with loopback.open(server.url + f"/simple/{package}/", timeout=120) as response:
            if response.read() != fixtures.simple[package]:
                raise ValueError("Oracle index bytes differ")
        headers = (
            {"Range": f"bytes=0-{len(prefix) - 1}"}
            if profile.get("ranges", True)
            else {}
        )
        request = urllib.request.Request(
            server.url + f"/files/{args.filename}", headers=headers
        )
        with loopback.open(request, timeout=120) as response:
            expected_status = 206 if profile.get("ranges", True) else 200
            if (
                response.status != expected_status
                or response.read(len(prefix)) != prefix
            ):
                raise ValueError("Oracle source prefix differs")
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = len(fixtures.simple[package]) + len(prefix)
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "metadata_sha256": hashlib.sha256(metadata).hexdigest(),
        "metadata_bytes": len(metadata),
        "metadata_tar_offset": offset,
        "compressed_prefix_bytes": len(prefix),
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": 2,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 2
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known gzip archive and static PKG-INFO offset. Fetch the index and enough compressed bytes to reach that metadata. This optimistic reference excludes full-archive hash validation, gzip trailer validation, source extraction, and discovery of alternative metadata sources.",
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
