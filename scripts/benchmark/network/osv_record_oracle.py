"""Measure bounded OSV page and full-record retrieval for a pinned audit."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import http.client
import importlib.util
import json
import math
import threading
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "osv_oracle", Path(__file__).with_name("osv_oracle.py")
)
assert spec is not None and spec.loader is not None
osv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(osv)
bench = osv.bench


def compact(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


def requests(
    configuration: dict, count: int, cached: bool, revalidate: bool = False
) -> tuple[list, list]:
    if cached and revalidate:
        raise ValueError("Cached records do not need revalidation")
    queries = osv.requests(configuration, count)
    identifiers: dict[str, set[int]] = {}
    for index, task in enumerate(queries):
        for result in json.loads(task["expected"])["results"]:
            for vulnerability in result["vulns"]:
                identifiers.setdefault(vulnerability["id"], set()).add(index)
    records = []
    if not cached:
        for identifier, parents in sorted(identifiers.items()):
            record = configuration["vulnerabilities"][identifier]
            if record["id"] != identifier:
                raise ValueError("OSV record ID differs from its lookup key")
            payload = compact(record)
            task = {
                "path": "/v1/vulns/" + identifier,
                "expected": payload,
                "any_parent": parents,
            }
            if revalidate:
                task.update(
                    expected=b"",
                    status=304,
                    headers={
                        "If-None-Match": '"' + hashlib.sha256(payload).hexdigest() + '"'
                    },
                )
            records.append(task)
    return queries, records


def lower_bound(
    configuration: dict,
    count: int,
    concurrency: int,
    profile: dict,
    cached: bool,
    revalidate: bool = False,
) -> tuple[int, int, float]:
    selected = sorted(configuration["dependencies"].items())[:count]
    if len(selected) != count or concurrency < 1:
        raise ValueError("Invalid package count or concurrency")
    queries, records = requests(configuration, count, cached, revalidate)
    results = [
        result for task in queries for result in json.loads(task["expected"])["results"]
    ]
    minimum_queries = math.ceil(len(results) / 1000)
    required_bytes = (
        sum(len(compact(result)) for result in results)
        + len(results)
        - minimum_queries
        + len(b'{"results":[]}') * minimum_queries
        + sum(len(record["expected"]) for record in records)
    )
    earliest: dict[str, int] = {}
    if not cached:
        for _, dependency in selected:
            for page, identifiers in enumerate(dependency["vulns_by_page"]):
                for identifier in identifiers:
                    earliest[identifier] = min(earliest.get(identifier, page), page)
    waves = max(
        max(dependency["pages"] for _, dependency in selected),
        max((page + 2 for page in earliest.values()), default=0),
        math.ceil((minimum_queries + len(records)) / concurrency),
    )
    latencies = [
        profile.get("latency_ms", 0),
        *profile.get("osv_query_latency_ms", {}).values(),
        *(
            profile.get("path_latency_ms", {}).get(
                record["path"], profile.get("latency_ms", 0)
            )
            for record in records
        ),
    ]
    latency = waves * max(0, min(latencies) - profile.get("jitter_ms", 0))
    return required_bytes, waves, latency


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--packages", type=int, default=3001)
    parser.add_argument("--concurrency", type=int, default=50)
    record_mode = parser.add_mutually_exclusive_group()
    record_mode.add_argument("--records-cached", action="store_true")
    record_mode.add_argument("--records-revalidate", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    queries, records = requests(
        fixtures.osv, args.packages, args.records_cached, args.records_revalidate
    )
    tasks = [*queries, *records]
    required_bytes, waves, latency = lower_bound(
        fixtures.osv,
        args.packages,
        args.concurrency,
        profile,
        args.records_cached,
        args.records_revalidate,
    )
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
            if "body" in task:
                connection.request(
                    "POST",
                    "/v1/querybatch",
                    task["body"],
                    {"Content-Type": "application/json"},
                )
            else:
                connection.request("GET", task["path"], headers=task.get("headers", {}))
            response = connection.getresponse()
            body = response.read()
            if response.status == task.get("status", 200) and body == task["expected"]:
                return
            if attempt == 3 or response.status not in {408, 429, 500, 502, 503, 504}:
                raise ValueError("OSV response differs from the pinned fixture")
        raise AssertionError("Retry loop exhausted")

    started = time.perf_counter()
    try:
        waiting = set(range(len(tasks)))
        finished = set()
        active = {}
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=args.concurrency
        ) as pool:
            while waiting or active:
                ready = sorted(
                    index
                    for index in waiting
                    if (
                        tasks[index]["parents"] <= finished
                        if "parents" in tasks[index]
                        else bool(tasks[index]["any_parent"] & finished)
                    )
                )
                for index in ready[: args.concurrency - len(active)]:
                    waiting.remove(index)
                    active[pool.submit(read, tasks[index])] = index
                if not active:
                    raise ValueError("OSV request graph cannot make progress")
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
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "packages": args.packages,
        "concurrency": args.concurrency,
        "records_cached": args.records_cached,
        "records_revalidate": args.records_revalidate,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, latency
        ),
        "oracle_request_bytes": sum(len(task["body"]) for task in queries),
        "expected_queries": [task["identity"] for task in queries],
        "expected_records": [task["path"] for task in records],
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "max_active": bench.maximum_active(server.events),
        "events": server.events,
        "scope": "Synthetic frozen audit with known direct-URL dependencies and pinned vulnerability records. The optimistic floor permits arbitrary valid batches, overlap, global record deduplication, and full-record cache hits or body-free conditional revalidation when selected. It excludes retries, request-body transfer, TCP/TLS, headers, and CPU. The realizable reference schedules a bounded dependency graph and retries transient HTTP failures immediately.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                key: value
                for key, value in result.items()
                if key not in {"events", "expected_queries", "expected_records"}
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
