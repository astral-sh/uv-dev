"""Fetch known Simple API pages through a bounded pool of HTTP/1.1 connections."""

from __future__ import annotations

import argparse
import concurrent.futures
import http.client
import importlib.util
import json
import threading
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--concurrency", type=int, required=True)
    parser.add_argument("--packages", type=int)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.concurrency < 1 or (args.packages is not None and args.packages < 1):
        parser.error("concurrency and package count must be positive")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    packages = sorted(fixtures.packages)
    if args.packages is not None:
        if args.packages > len(packages):
            parser.error("package count exceeds the manifest")
        packages = packages[: args.packages]
    if not packages:
        parser.error("the manifest has no packages")
    concurrency = min(args.concurrency, len(packages))
    paths = [f"/simple/{name}/" for name in packages]
    expected = dict(zip(paths, (fixtures.simple[name] for name in packages), strict=True))
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def fetch(shard: list[str]) -> None:
        connection = http.client.HTTPConnection(
            "127.0.0.1", server.server_port, timeout=60
        )
        try:
            for path in shard:
                for attempt in range(5):
                    connection.request("GET", path)
                    response = connection.getresponse()
                    body = response.read()
                    if response.status == 200 and body == expected[path]:
                        break
                    if response.status not in {429, 500, 502, 503, 504} or attempt == 4:
                        raise ValueError(f"Oracle response differs: {path}")
        finally:
            connection.close()

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
            list(
                pool.map(
                    fetch, (paths[offset::concurrency] for offset in range(concurrency))
                )
            )
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(map(len, expected.values()))
    waves, required_latency = bench.concurrent_latency_floor(
        [
            max(
                0,
                profile.get("path_latency_ms", {}).get(
                    path, profile.get("latency_ms", 0)
                )
                - profile.get("jitter_ms", 0),
            )
            for path in paths
        ],
        concurrency,
        bench.netem_profile().get("rtt_ms", 0),
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "seconds": seconds,
        "packages": len(packages),
        "concurrency": concurrency,
        "paths": paths,
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "Known package names, fetching their complete Simple API pages in fixed shards on persistent HTTP/1.1 connections. Transient failures retry immediately. The optimistic bound omits connection setup, failures, interpreter discovery, and local work.",
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
