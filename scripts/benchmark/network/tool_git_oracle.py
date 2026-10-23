"""Measure the distinct Git reference lookups required by an installed tool set."""

from __future__ import annotations

import argparse
import concurrent.futures
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
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--selection", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    descriptor = json.loads((args.directory / "git-descriptor.json").read_text())
    profile = json.loads((args.directory / "tool-git-profiles.json").read_text())[
        args.profile
    ]
    paths = descriptor["api_paths"][args.selection]
    lookup_paths = [] if args.selection in {"pinned", "short"} else paths
    fixtures = bench.Fixtures(
        args.directory / "tool-git-fixtures.json", args.directory, True
    )
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def lookup(path: str) -> str:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        for attempt in range(4):
            try:
                with opener.open(server.url + path, timeout=120) as response:
                    commit = response.read().decode()
                    if commit != descriptor["commit"]:
                        raise ValueError("Reference lookup differs from the fixture")
                    return commit
            except urllib.error.HTTPError as error:
                if error.code != 503 or attempt == 3:
                    raise
        raise AssertionError("unreachable")

    try:
        start = time.perf_counter()
        if lookup_paths:
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=len(lookup_paths)
            ) as pool:
                assert list(pool.map(lookup, lookup_paths)) == [
                    descriptor["commit"]
                ] * len(lookup_paths)
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    # The installed tools already have the resolved commit. A conditional lookup
    # can return no body, and immutable revisions need no network revalidation.
    # Named refs can also be discovered through Git's advertised refs endpoint,
    # so the API-specific delay alone is not a global lower bound.
    required_bytes = 0
    waves = int(bool(paths) and args.selection not in {"pinned", "short"})
    required_latency = min(
        profile.get("latency_ms", 0),
        min(
            (
                profile.get("path_latency_ms", {}).get(
                    path, profile.get("latency_ms", 0)
                )
                for path in paths
            ),
            default=0,
        ),
    ) - profile.get("jitter_ms", 0)
    required_latency = max(0, required_latency) if waves else 0
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.directory / "tool-git-fixtures.json"),
        "commit": descriptor["commit"],
        "selection": args.selection,
        "paths": paths,
        "lookup_paths": lookup_paths,
        "seconds": seconds,
        "requests": len(server.events),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, required_latency
        ),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "Fetch each distinct GitHub reference once and verify its full commit as a realizable reference. The optimistic bound permits lookups to overlap, allows conditional responses with no body, and uses the smaller of Git ref-advertisement and API latency. Installed immutable revisions have a zero network floor. Repository fetching, environment inspection, HTTP setup, local work, and transient-error waits are excluded.",
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
