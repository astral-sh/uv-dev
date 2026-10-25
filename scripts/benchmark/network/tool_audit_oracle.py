"""Retrieve known registry project statuses and batched OSV audit responses."""

from __future__ import annotations

import argparse
import concurrent.futures
import http.client
import importlib.util
import json
import math
import threading
import time
from collections import Counter
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "osv_oracle", Path(__file__).with_name("osv_oracle.py")
)
assert spec is not None and spec.loader is not None
osv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(osv)
bench = osv.bench


def prepare(fixtures: object, profile: dict, count: int, cached: bool) -> list[dict]:
    configuration = fixtures.osv
    if "tool_dependencies" in configuration:
        tools = sorted(configuration["tool_dependencies"])[:count]
        if len(tools) != count:
            raise ValueError("Requested more tools than the fixture contains")
        names = {
            name for tool in tools for name in configuration["tool_dependencies"][tool]
        }
        selected = {
            "dependencies": {
                name: configuration["dependencies"][name] for name in sorted(names)
            }
        }
        projects = sorted(
            {
                (project["index"], project["name"])
                for tool in tools
                for project in configuration["tool_registry_projects"][tool]
            }
        )
    else:
        selected = configuration
        names = sorted(configuration["dependencies"])[:count]
        projects = [(number % 2, name) for number, name in enumerate(names)]
    tasks = [
        dict(task, group="osv", method="POST", path="/v1/querybatch")
        for task in osv.requests(selected, len(names))
    ]
    if not cached:
        for index, name in projects:
            path = f"/audit-index-{index}/{name}/"
            if profile["path_aliases"][path] != f"/simple/{name}/":
                raise ValueError("Registry audit fixture alias differs")
            tasks.append(
                {
                    "group": "registry",
                    "method": "GET",
                    "path": path,
                    "body": None,
                    "expected": fixtures.simple[name],
                    "parents": set(),
                }
            )
    return tasks


def bounds(
    configuration: dict, profile: dict, tasks: list[dict], concurrency: int
) -> dict:
    results = [
        result
        for task in tasks
        if task["group"] == "osv"
        for result in json.loads(task["expected"])["results"]
    ]
    minimum_batches = math.ceil(len(results) / 1000)
    required_bytes = (
        sum(
            len(json.dumps(result, separators=(",", ":")).encode())
            for result in results
        )
        + len(results)
        - minimum_batches
        + len(b'{"results":[]}') * minimum_batches
        + sum(len(task["expected"]) for task in tasks if task["group"] == "registry")
    )
    names = {
        query["package"]["name"]
        for task in tasks
        if task["group"] == "osv"
        for query in json.loads(task["body"])["queries"]
    }
    osv_waves = max(
        max(configuration["dependencies"][name]["pages"] for name in names),
        math.ceil(minimum_batches / concurrency),
    )
    jitter = profile.get("jitter_ms", 0)
    osv_latency = max(
        0,
        min(
            [
                profile.get("latency_ms", 0),
                *profile.get("osv_query_latency_ms", {}).values(),
            ]
        )
        - jitter,
    )
    rtt = bench.netem_profile().get("rtt_ms", 0)
    registry_latencies = [
        max(
            0,
            profile.get("path_latency_ms", {}).get(
                task["path"], profile.get("latency_ms", 0)
            )
            - jitter,
        )
        for task in tasks
        if task["group"] == "registry"
    ]
    registry_waves, registry_latency = (
        bench.concurrent_latency_floor(registry_latencies, concurrency, rtt)
        if registry_latencies
        else (0, 0)
    )
    waves = max(osv_waves, registry_waves)
    latency = (
        max(osv_waves * (osv_latency + rtt), registry_latency + registry_waves * rtt)
        - waves * rtt
    )
    return {
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, latency
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--packages", type=int, required=True)
    parser.add_argument("--concurrency", type=int, default=50)
    parser.add_argument("--cached-registry", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.packages < 1 or args.concurrency < 1:
        parser.error("package count and concurrency must be positive")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    tasks = prepare(fixtures, profile, args.packages, args.cached_registry)
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    local = threading.local()
    connections = []
    connection_lock = threading.Lock()

    def read(task: dict) -> None:
        if not hasattr(local, "connection"):
            local.connection = http.client.HTTPConnection(
                "127.0.0.1", server.server_port, timeout=60
            )
            with connection_lock:
                connections.append(local.connection)
        connection = local.connection
        for attempt in range(4):
            headers = {"Content-Type": "application/json"} if task["body"] else {}
            connection.request(task["method"], task["path"], task["body"], headers)
            response = connection.getresponse()
            body = response.read()
            if response.status == 200 and body == task["expected"]:
                return
            if attempt == 3 or response.status not in {408, 429, 500, 502, 503, 504}:
                raise ValueError("Audit response differs from the pinned fixture")
        raise AssertionError("Retry loop exhausted")

    started = time.perf_counter()
    try:
        waiting = set(range(len(tasks)))
        finished = set()
        active = {}
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=min(len(tasks), 2 * args.concurrency)
        ) as pool:
            while waiting or active:
                counts = Counter(tasks[index]["group"] for index in active.values())
                for index in sorted(waiting):
                    task = tasks[index]
                    if (
                        task["parents"] <= finished
                        and counts[task["group"]] < args.concurrency
                    ):
                        waiting.remove(index)
                        counts[task["group"]] += 1
                        active[pool.submit(read, task)] = index
                if not active:
                    raise ValueError("Audit request graph cannot make progress")
                completed, _ = concurrent.futures.wait(
                    active, return_when=concurrent.futures.FIRST_COMPLETED
                )
                for future in completed:
                    future.result()
                    finished.add(active.pop(future))
        seconds = time.perf_counter() - started
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
        "packages": args.packages,
        "concurrency_per_service": args.concurrency,
        "cached_registry": args.cached_registry,
        "seconds": seconds,
        **bounds(fixtures.osv, profile, tasks, args.concurrency),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "max_active": bench.maximum_active(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "Synthetic audit with known registry dependencies. The reference retrieves every uncached project's complete Simple API page while batching OSV queries across tools, using independent service concurrency limits and retrying transient failures immediately. The optimistic floor permits both services to overlap, arbitrary OSV batching after pagination tokens become available, and excludes retries, request-body transfer, connection establishment, headers, and CPU work.",
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
