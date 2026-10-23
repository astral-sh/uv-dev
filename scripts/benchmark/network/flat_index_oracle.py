"""Measure a known package graph split between Simple API and flat indexes."""

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
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", action="append", required=True)
    parser.add_argument("--flat-package", action="append", required=True)
    parser.add_argument("--flat-path", default="/flat/slow")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    if not profile.get("pep658", True):
        parser.error("the metadata oracle requires PEP 658")
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    names = {
        item["filename"]: name
        for name, files in fixtures.packages.items()
        for item in files
        if item["filename"] in args.filename
    }
    flat_packages = {bench.normalize(name) for name in args.flat_package}
    if set(names) != set(args.filename) or not flat_packages.issubset(names.values()):
        parser.error("every selected package must occur in the manifest")
    index_paths = {
        filename: args.flat_path if name in flat_packages else f"/simple/{name}/"
        for filename, name in names.items()
    }
    expected_indexes = {
        path: fixtures.flat
        if path == args.flat_path
        else fixtures.simple[path.split("/")[2]]
        for path in set(index_paths.values())
    }
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def read(path: str, expected: bytes) -> None:
        with loopback.open(server.url + path, timeout=60) as response:
            if response.read() != expected:
                raise ValueError(f"Oracle response differs: {path}")

    def fetch(filename: str, indexes: dict) -> None:
        indexes[index_paths[filename]].result()
        read(
            f"/files/{filename}.metadata",
            fixtures.metadata[filename + ".metadata"],
        )

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(names) + 1) as pool:
            indexes = {
                path: pool.submit(read, path, expected)
                for path, expected in expected_indexes.items()
            }
            futures = [pool.submit(fetch, filename, indexes) for filename in names]
            for future in futures:
                future.result()
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(map(len, expected_indexes.values())) + sum(
        len(fixtures.metadata[filename + ".metadata"]) for filename in names
    )

    def minimum_latency(path: str) -> float:
        return max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )

    critical_latency = max(
        minimum_latency(index_paths[filename])
        + minimum_latency(f"/files/{filename}.metadata")
        for filename in names
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filenames": args.filename,
        "flat_packages": sorted(flat_packages),
        "flat_path": args.flat_path,
        "seconds": seconds,
        "required_metadata_and_index_bytes": required_bytes,
        "required_latency_ms": critical_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, 2, critical_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Known selected versions and dependencies with unlimited request concurrency. Each index is fetched once before its metadata sidecars; excludes resolution and installation.",
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
