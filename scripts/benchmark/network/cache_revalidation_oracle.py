"""Measure index revalidation for unchanged, hash-identified cached content."""

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
    parser.add_argument("--identity", choices=["source", "metadata"], default="source")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    package = next(
        name
        for name, files in fixtures.packages.items()
        if any(file["filename"] == args.filename for file in files)
    )
    index = fixtures.simple[package]
    if args.identity == "metadata" and not profile.get("pep658", True):
        parser.error("metadata identity requires advertised PEP 658 hashes")
    etag = '"' + hashlib.sha256(index).hexdigest() + '"'
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    path = f"/simple/{package}/"
    request = urllib.request.Request(server.url + path, headers={"If-None-Match": etag})
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
                raise ValueError("Oracle did not validate the unchanged index")
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    latency = max(
        0,
        profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
        - profile.get("jitter_ms", 0),
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "identity": args.identity,
        "identity_sha256": (
            hashlib.sha256(fixtures.metadata[args.filename + ".metadata"]).hexdigest()
            if args.identity == "metadata"
            else fixtures.hashes[args.filename]
        ),
        "seconds": seconds,
        "required_bytes": 0,
        "required_waves": 1,
        "required_latency_ms": latency,
        "optimistic_network_floor_seconds": bench.network_floor(profile, 0, 1, latency),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": (
            "An index requiring revalidation is unchanged, and the advertised strong PEP 658 hash matches a previously verified metadata sidecar. Only conditional index validation is required; fresh index caches can need no network requests."
            if args.identity == "metadata"
            else "An index requiring revalidation is unchanged, and the advertised strong archive hash matches a previously validated local source revision. Only conditional index validation is required; fresh index caches can need no network requests."
        ),
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
