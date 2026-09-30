"""Measure a known package lookup across independent Simple API indexes."""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import threading
import time
import urllib.request
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
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--package", required=True)
    parser.add_argument("--filename", required=True)
    parser.add_argument("--index-path", action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    if not profile.get("pep658", True):
        parser.error("the metadata oracle requires PEP 658")
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    index_body = fixtures.simple[bench.normalize(args.package)]
    metadata_path = f"/files/{args.filename}.metadata"
    metadata = fixtures.metadata[args.filename + ".metadata"]
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def read(path: str, expected: bytes) -> None:
        with loopback.open(server.url + path, timeout=60) as response:
            if response.read() != expected:
                raise ValueError(f"Oracle response differs: {path}")

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=len(args.index_path)
        ) as pool:
            futures = [pool.submit(read, path, index_body) for path in args.index_path]
            for future in futures:
                future.result()
        read(metadata_path, metadata)
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()

    def minimum_latency(path: str) -> float:
        return max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )

    required_bytes = len(index_body) * len(args.index_path) + len(metadata)
    required_latency = max(map(minimum_latency, args.index_path)) + minimum_latency(
        metadata_path
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "index_paths": args.index_path,
        "filename": args.filename,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": 2,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 2, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known selected package with unlimited concurrent index requests, followed by its metadata. The reference includes every configured index, as required by unsafe-best-match.",
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
