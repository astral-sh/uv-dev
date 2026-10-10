"""Measure bounded OSV query retrieval for a pinned set of audit dependencies."""

from __future__ import annotations

import argparse
import concurrent.futures
import http.client
import importlib.util
import json
import math
import threading
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def query(name: str, version: str, token: str | None = None) -> dict:
    result = {"package": {"name": name, "ecosystem": "PyPI"}, "version": version}
    if token is not None:
        result["page_token"] = token
    return result


def requests(configuration: dict, count: int) -> list[dict]:
    selected = sorted(configuration["dependencies"].items())[:count]
    if len(selected) != count:
        raise ValueError("Requested more dependencies than the fixture contains")
    pending = [(query(name, entry["version"]), None) for name, entry in selected]
    tasks = []
    while pending:
        following = []
        for offset in range(0, len(pending), 1000):
            batch = pending[offset : offset + 1000]
            body = json.dumps(
                {"queries": [request for request, _ in batch]}, separators=(",", ":")
            ).encode()
            expected, identity = bench.osv_query_response(configuration, body)
            task = len(tasks)
            tasks.append(
                {
                    "body": body,
                    "expected": expected,
                    "identity": identity,
                    "parents": {parent for _, parent in batch if parent is not None},
                }
            )
            for (request, _), response in zip(
                batch, json.loads(expected)["results"], strict=True
            ):
                if token := response.get("next_page_token"):
                    following.append((dict(request, page_token=token), task))
        pending = following
    return tasks


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--packages", type=int, default=3001)
    parser.add_argument("--concurrency", type=int, default=50)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.packages < 1 or args.concurrency < 1:
        parser.error("--packages and --concurrency must be positive")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    tasks = requests(fixtures.osv, args.packages)
    results = [
        result for task in tasks for result in json.loads(task["expected"])["results"]
    ]
    minimum_requests = math.ceil(len(results) / 1000)
    required_bytes = (
        sum(
            len(json.dumps(result, separators=(",", ":")).encode())
            for result in results
        )
        + len(results)
        - minimum_requests
        + len(b'{"results":[]}') * minimum_requests
    )
    maximum_pages = max(
        entry["pages"]
        for _, entry in sorted(fixtures.osv["dependencies"].items())[: args.packages]
    )
    waves = max(maximum_pages, math.ceil(minimum_requests / args.concurrency))
    minimum_latency = max(
        0,
        min(
            [
                profile.get("latency_ms", 0),
                *profile.get("osv_query_latency_ms", {}).values(),
            ]
        )
        - profile.get("jitter_ms", 0),
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
            connection.request(
                "POST",
                "/v1/querybatch",
                task["body"],
                {"Content-Type": "application/json"},
            )
            response = connection.getresponse()
            body = response.read()
            if response.status == 200 and body == task["expected"]:
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
                    index for index in waiting if tasks[index]["parents"] <= finished
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
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": waves * minimum_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, waves * minimum_latency
        ),
        "oracle_request_bytes": sum(len(task["body"]) for task in tasks),
        "expected_queries": [task["identity"] for task in tasks],
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "max_active": bench.maximum_active(server.events),
        "events": server.events,
        "scope": "Synthetic frozen audit with known direct-URL dependencies and no vulnerability findings. The optimistic floor assumes batches of at most 1000 queries, permits arbitrary batching and overlap once pagination tokens are available, and excludes retries, request-body transfer, TCP/TLS, headers, and CPU work. The oracle sends each prepared batch as soon as its prerequisite responses finish and retries transient HTTP failures without backoff.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                key: value
                for key, value in result.items()
                if key not in {"events", "expected_queries"}
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
