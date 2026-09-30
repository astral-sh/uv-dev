"""Measure bounded retrieval of a pinned remote requirements graph."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import heapq
import http.client
import importlib.util
import json
import math
import threading
import time
import urllib.parse
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def parse(path: str, body: bytes) -> tuple[list[str], bool]:
    """Accept only the deliberately small grammar emitted by the fixture builder."""
    includes = []
    requirement = False
    for line in body.decode().splitlines():
        if line.startswith(("-r ", "-c ")):
            includes.append(urllib.parse.urljoin(path, line[3:]))
        elif line == "iniconfig==2.1.0":
            requirement = True
        elif line.strip():
            raise ValueError(f"Unexpected oracle requirement: {line!r}")
    return includes, requirement


def graph_bounds(
    root: str,
    graph: dict[str, list[str]],
    package_paths: set[str],
    metadata_paths: tuple[str, ...],
    profile: dict,
    concurrency: int,
    rtt_ms: float,
) -> dict:
    def application_latency(path: str) -> float:
        return max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )

    def distances(weight) -> dict[str, float]:
        # A shared include becomes available when any referring parent finishes.
        result = {root: weight(root)}
        queue = [(result[root], root)]
        while queue:
            cost, path = heapq.heappop(queue)
            if cost != result[path]:
                continue
            for child in graph[path]:
                candidate = cost + weight(child)
                if candidate < result.get(child, math.inf):
                    result[child] = candidate
                    heapq.heappush(queue, (candidate, child))
        return result

    depths = distances(lambda _: 1)
    waves = int(
        max(
            max(depths.values()),
            min(depths[path] for path in package_paths) + len(metadata_paths),
            math.ceil((len(graph) + len(metadata_paths)) / concurrency),
        )
    )
    paths = [*graph, *metadata_paths]
    costs = {path: application_latency(path) + rtt_ms for path in paths}
    arrivals = distances(costs.__getitem__)
    latency = max(
        max(arrivals.values()),
        min(arrivals[path] for path in package_paths)
        + sum(costs[path] for path in metadata_paths),
        sum(costs.values()) / concurrency,
        waves * min(costs.values()),
        waves * rtt_ms,
    )
    return {
        "required_waves": waves,
        # bench.network_floor adds the RTT charge separately. Keeping only the
        # remainder here avoids summing maxima from different critical paths.
        "required_latency_ms": latency - waves * rtt_ms,
        "optimistic_latency_bound_ms": latency,
        "requirements_files": len(graph),
        "requirement_discovery_paths": sorted(package_paths),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=bench.HERE / "profiles.json")
    parser.add_argument("--profile", default="fast")
    parser.add_argument("--root", default="/requirements/prefetch-root.txt")
    parser.add_argument(
        "--route",
        choices=["metadata", "requirements", "revalidate"],
        default="metadata",
    )
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("--concurrency must be positive")
    profile = json.loads(args.profiles.read_text())[args.profile]
    if not profile.get("pep658", True):
        parser.error("the oracle requires PEP 658")
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    filename = "iniconfig-2.1.0-py3-none-any.whl"
    metadata_paths = {
        "metadata": ("/simple/iniconfig/", f"/files/{filename}.metadata"),
        "requirements": (),
        "revalidate": ("/simple/iniconfig/",),
    }[args.route]
    graph = {}
    bodies = {}
    package_paths = set()
    unseen = [args.root]
    while unseen:
        path = unseen.pop()
        if path in graph:
            continue
        body = fixtures.routes[path].read_bytes()
        children, contains_package = parse(path, body)
        graph[path] = children
        bodies[path] = body
        if contains_package:
            package_paths.add(path)
        unseen.extend(children)
    if not package_paths:
        raise ValueError("Requirements graph has no pinned package")
    conditional = {}
    if args.route == "metadata":
        bodies[metadata_paths[0]] = fixtures.simple["iniconfig"]
        bodies[metadata_paths[1]] = fixtures.metadata[filename + ".metadata"]
    elif args.route == "revalidate":
        index = fixtures.simple["iniconfig"]
        sidecar_hash = hashlib.sha256(
            fixtures.metadata[filename + ".metadata"]
        ).hexdigest()
        if sidecar_hash.encode() not in index:
            raise ValueError("Index does not identify the cached metadata sidecar")
        conditional[metadata_paths[0]] = '"' + hashlib.sha256(index).hexdigest() + '"'
        bodies[metadata_paths[0]] = b""
    bounds = graph_bounds(
        args.root,
        graph,
        package_paths,
        metadata_paths,
        profile,
        args.concurrency,
        bench.netem_profile().get("rtt_ms", 0),
    )
    required_bytes = sum(map(len, bodies.values()))
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    local = threading.local()
    connections = []
    connection_lock = threading.Lock()

    def read(path: str) -> bytes:
        if not hasattr(local, "connection"):
            local.connection = http.client.HTTPConnection(
                "127.0.0.1", server.server_port, timeout=60
            )
            with connection_lock:
                connections.append(local.connection)
        connection = local.connection
        for attempt in range(4):
            headers = (
                {"If-None-Match": conditional[path]} if path in conditional else {}
            )
            connection.request("GET", path, headers=headers)
            response = connection.getresponse()
            body = response.read()
            expected_status = 304 if path in conditional else 200
            if (
                response.status == expected_status
                and body == bodies[path]
                and (
                    path not in conditional
                    or response.getheader("ETag") == conditional[path]
                )
            ):
                return body
            if attempt == 3 or response.status not in {408, 429, 500, 502, 503, 504}:
                raise ValueError(f"Oracle response differs: {path}")
        raise AssertionError("Retry loop exhausted")

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=args.concurrency
        ) as pool:
            seen = {args.root}
            pending = {pool.submit(read, args.root): args.root}
            while pending:
                completed, _ = concurrent.futures.wait(
                    pending, return_when=concurrent.futures.FIRST_COMPLETED
                )
                for future in completed:
                    path = pending.pop(future)
                    children, _ = parse(path, future.result())
                    for child in children:
                        if child not in seen:
                            seen.add(child)
                            pending[pool.submit(read, child)] = child
            if seen != graph.keys():
                raise ValueError("Oracle did not retrieve the complete graph")
            for path in metadata_paths:
                pool.submit(read, path).result()
        seconds = time.perf_counter() - start
    finally:
        for connection in connections:
            connection.close()
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "root": args.root,
        "route": args.route,
        "concurrency": args.concurrency,
        "seconds": seconds,
        "required_bytes": required_bytes,
        **bounds,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile,
            required_bytes,
            bounds["required_waves"],
            bounds["required_latency_ms"],
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "retry_scope": "Transient HTTP responses are retried up to three times per URL without oracle backoff. The optimistic floor excludes retries.",
        "bound_scope": "All unique include bodies and the selected route's metadata are required. The bound combines shortest discovery paths, the download limit, and body serialization. It permits metadata lookup to overlap remaining includes and excludes connection startup, headers, parsing, and CPU work.",
        "scope": "Discover and fetch remote includes at the recorded concurrency, reusing one HTTP/1.1 connection per worker. The metadata route then fetches the known package's index and metadata. The requirements route assumes those responses are fresh in cache. The revalidate route conditionally validates the unchanged index, whose strong PEP 658 hash identifies the cached sidecar. This is a realizable retrieval strategy; resolution is excluded.",
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
