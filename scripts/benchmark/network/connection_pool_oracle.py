"""Fetch known package metadata using one persistent HTTP connection per package."""

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
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    if any(len(files) != 1 for files in fixtures.packages.values()):
        parser.error("the connection oracle requires one pinned file per package")
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def fetch(name: str) -> None:
        connection = http.client.HTTPConnection(
            "127.0.0.1", server.server_port, timeout=60
        )
        filename = fixtures.packages[name][0]["filename"]
        try:
            for path, expected in (
                (f"/simple/{name}/", fixtures.simple[name]),
                (
                    f"/files/{filename}.metadata",
                    fixtures.metadata[filename + ".metadata"],
                ),
            ):
                connection.request("GET", path)
                response = connection.getresponse()
                if response.status != 200 or response.read() != expected:
                    raise ValueError(f"Oracle response differs: {path}")
        finally:
            connection.close()

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=len(fixtures.packages)
        ) as pool:
            list(pool.map(fetch, fixtures.packages))
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(map(len, fixtures.simple.values())) + sum(
        map(len, fixtures.metadata.values())
    )

    def latency(path: str) -> float:
        return max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )

    required_latency = profile.get("connection_latency_ms", 0) + max(
        latency(f"/simple/{name}/") + latency(f"/files/{files[0]['filename']}.metadata")
        for name, files in fixtures.packages.items()
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "seconds": seconds,
        "packages": len(fixtures.packages),
        "required_bytes": required_bytes,
        "required_waves": 2,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 2, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "Known selected versions and dependency graph, with all index-to-metadata pairs fetched concurrently on persistent HTTP/1.1 connections. The bound includes one configured application connection delay and omits TCP startup and dependency discovery.",
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
