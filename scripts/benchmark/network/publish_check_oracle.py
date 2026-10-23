"""Check a known batch of local distributions against one current index snapshot."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def selected_distributions(
    manifest: Path, filenames: list[str]
) -> tuple[list[dict], list[str]]:
    entries = json.loads(manifest.read_text())
    selected = set(filenames) if filenames else {entry["filename"] for entry in entries}
    entries = [entry for entry in entries if entry["filename"] in selected]
    if {entry["filename"] for entry in entries} != selected or not entries:
        raise ValueError("every selected distribution must appear in the manifest")
    packages = sorted(
        {bench.normalize(entry["filename"].split("-")[0]) for entry in entries}
    )
    return entries, packages


def verify_distributions(directory: Path, entries: list[dict], indexes: dict) -> None:
    for entry in entries:
        package = bench.normalize(entry["filename"].split("-")[0])
        remote = indexes[package][entry["filename"]]["hashes"]["sha256"]
        if (
            remote != entry["sha256"]
            or bench.digest(directory / entry["filename"]) != remote
        ):
            raise ValueError(f"Distribution digest differs: {entry['filename']}")


def lower_bound(
    fixtures, packages: list[str], profile: dict, route: str, concurrency: int
) -> dict:
    required_bytes = (
        sum(len(fixtures.simple[package]) for package in packages)
        if route == "current"
        else 0
    )
    netem = bench.netem_profile()
    required_waves, required_latency = bench.concurrent_latency_floor(
        [
            max(
                0,
                profile.get("path_latency_ms", {}).get(
                    f"/simple/{package}/", profile.get("latency_ms", 0)
                )
                - profile.get("jitter_ms", 0),
            )
            for package in packages
        ],
        concurrency,
        netem.get("rtt_ms", 0),
    )
    return {
        "required_bytes": required_bytes,
        "required_waves": required_waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, required_waves, required_latency, netem=netem
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", action="append", default=[])
    parser.add_argument("--route", choices=["current", "revalidate"], default="current")
    parser.add_argument("--concurrency", type=int, default=50)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("concurrency must be positive")
    try:
        entries, packages = selected_distributions(args.manifest, args.filename)
    except ValueError as error:
        parser.error(str(error))
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def read(package: str) -> tuple[str, dict]:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        expected = fixtures.simple[package]
        etag = '"' + hashlib.sha256(expected).hexdigest() + '"'
        request = urllib.request.Request(
            server.url + f"/simple/{package}/",
            headers={"If-None-Match": etag} if args.route == "revalidate" else {},
        )
        for attempt in range(4):
            try:
                response = opener.open(request, timeout=60)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                status = response.status
                body = response.read()
                response_etag = response.headers.get("ETag")
            if args.route == "revalidate" and status == 304 and response_etag == etag:
                if body:
                    raise ValueError(
                        f"Conditional index response has a body: {package}"
                    )
                body = expected
                break
            if args.route == "current" and status == 200 and body == expected:
                break
            if attempt == 3 or status not in {408, 429, 500, 502, 503, 504}:
                raise ValueError(f"Unexpected index response: {package} ({status})")
        if body != expected:
            raise ValueError(f"Index response differs: {package}")
        return package, {
            entry["filename"]: entry for entry in json.loads(body)["files"]
        }

    started = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=min(args.concurrency, len(packages))
        ) as pool:
            indexes = dict(pool.map(read, packages))
        verify_distributions(args.directory, entries, indexes)
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filenames": sorted({entry["filename"] for entry in entries}),
        "route": args.route,
        "concurrency": args.concurrency,
        "seconds": seconds,
        **lower_bound(fixtures, packages, profile, args.route, args.concurrency),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "retry_scope": "Transient HTTP responses are retried up to three times per URL without oracle backoff. The optimistic floor excludes retries.",
        "scope": "Known local files and package names. Read each current index within the configured concurrency limit, or conditionally revalidate a cached index with its strong ETag, and verify every selected file's SHA-256. The floor excludes hashing, headers, TCP/TLS, and other CPU work.",
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
