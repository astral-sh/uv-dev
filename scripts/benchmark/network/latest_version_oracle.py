"""Read known package versions from Simple API and find-links sources concurrently."""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import math
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
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--package", required=True)
    parser.add_argument("--simple-path", action="append", default=[])
    parser.add_argument("--flat-path", action="append", default=[])
    parser.add_argument("--concurrency", type=int)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.simple_path and not args.flat_path:
        parser.error("at least one source path is required")
    if args.concurrency is not None and args.concurrency < 1:
        parser.error("--concurrency must be positive")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    paths = [
        (path, fixtures.simple[bench.normalize(args.package)])
        for path in args.simple_path
    ]
    paths.extend((path, fixtures.flat) for path in args.flat_path)
    concurrency = min(args.concurrency or len(paths), len(paths))
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def read(item: tuple[str, bytes]) -> None:
        path, expected = item
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(server.url + path, timeout=60) as response:
            if response.read() != expected:
                raise ValueError(f"Oracle response differs: {path}")

    started = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
            list(pool.map(read, paths))
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(len(body) for _, body in paths)
    latencies = [
        max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )
        for path, _ in paths
    ]
    required_latency = max(max(latencies), sum(latencies) / concurrency)
    required_waves = math.ceil(len(paths) / concurrency)
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "package": args.package,
        "paths": [path for path, _ in paths],
        "concurrency": concurrency,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": required_waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, required_waves, required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known package and configured sources. Fetch and verify every required version listing at the recorded concurrency. The latency bound is the larger of the slowest response and total response latency divided by concurrency. It excludes TCP setup, response headers, parsing, and CPU work.",
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
