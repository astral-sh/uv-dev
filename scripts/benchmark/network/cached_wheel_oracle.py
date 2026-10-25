"""Measure local metadata access for a previously verified direct wheel."""

from __future__ import annotations

import argparse
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=bench.HERE / "fixtures.json")
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", required=True)
    parser.add_argument(
        "--cache-state",
        choices=("content-addressed", "fresh", "revalidate"),
        default="content-addressed",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, False)
    wheel = fixtures.files[args.filename]
    if wheel.suffix != ".whl":
        parser.error("select a wheel archive")
    expected = fixtures.metadata[args.filename + ".metadata"]
    events = []
    required_waves = 0
    latency = 0
    # Fixture construction verifies the complete archive before the timed cache read.
    started = time.perf_counter()
    if args.cache_state == "revalidate":
        server = bench.Server(fixtures, profile)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        path = f"/files/{args.filename}"
        etag = '"' + fixtures.hashes[args.filename] + '"'
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        request = urllib.request.Request(
            server.url + path, headers={"If-None-Match": etag}
        )
        started = time.perf_counter()
        try:
            try:
                response = opener.open(request, timeout=120)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                if (
                    response.status != 304
                    or response.read()
                    or response.headers["ETag"] != etag
                ):
                    raise ValueError("Oracle did not revalidate the cached wheel")
            metadata = bench.distribution_metadata(wheel)
            seconds = time.perf_counter() - started
        finally:
            server.shutdown()
            server.wait_idle()
            server.server_close()
            thread.join()
        events = server.events
        required_waves = 1
        latency = max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )
    else:
        metadata = bench.distribution_metadata(wheel)
        seconds = time.perf_counter() - started
    if metadata != expected:
        raise ValueError("Cached wheel metadata differs")
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "cache_state": args.cache_state,
        "identity_sha256": fixtures.hashes[args.filename],
        "metadata_sha256": hashlib.sha256(metadata).hexdigest(),
        "seconds": seconds,
        "required_bytes": 0,
        "required_waves": required_waves,
        "required_latency_ms": latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, 0, required_waves, latency
        ),
        "actual_bytes": sum(event["bytes"] for event in events),
        "requests": len(events),
        "events": events,
        "scope": {
            "content-addressed": "A direct wheel has already been downloaded completely and verified against the required content hash. Its unchanged local metadata requires no network transfer.",
            "fresh": "A complete cached direct wheel remains fresh under its HTTP policy. Its unchanged local metadata requires no network transfer.",
            "revalidate": "An unchanged complete cached direct wheel requires HTTP revalidation. One conditional artifact request establishes freshness before reading metadata locally.",
        }[args.cache_state]
        + " The reference times ZIP metadata access after fixture verification; its optimistic network floor excludes process startup, filesystem, and CPU costs.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps(data, indent=2))


if __name__ == "__main__":
    main()
