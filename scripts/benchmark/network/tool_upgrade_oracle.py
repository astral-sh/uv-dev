"""Fetch the known registry pages and full wheels needed by a tool upgrade."""

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
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def requests(fixtures, profile: dict, scenario: dict) -> list[dict]:
    if scenario["source"] in {"local", "pinned"}:
        return []
    tasks = []
    indexes = {}
    for project in scenario["projects"]:
        path = f"/upgrade-index-{project['index']}/{project['name']}/"
        if profile["path_aliases"][path] != f"/simple/{project['name']}/":
            raise ValueError("Tool-upgrade registry alias differs")
        indexes.setdefault(project["name"], []).append(len(tasks))
        tasks.append(
            {
                "path": path,
                "expected": fixtures.simple[project["name"]],
                "parents": set(),
            }
        )
    if scenario["initial_version"] != scenario["final_version"]:
        for filename in scenario["selected_wheels"]:
            name = next(
                name
                for name, files in fixtures.packages.items()
                if any(file["filename"] == filename for file in files)
            )
            tasks.append(
                {
                    "path": "/files/" + filename,
                    "expected": fixtures.files[filename].read_bytes(),
                    "parents": {indexes[name][0]},
                }
            )
    return tasks


def bounds(tasks: list[dict], profile: dict, concurrency: int, *, netem=None) -> dict:
    """Bound the generated two-stage page-to-wheel request graph."""
    if concurrency < 1:
        raise ValueError("concurrency must be positive")
    if netem is None:
        netem = bench.netem_profile()
    rtt_ms = netem.get("rtt_ms", 0)
    required_bytes = sum(len(task["expected"]) for task in tasks)
    costs, depths, arrivals = [], [], []
    for index, task in enumerate(tasks):
        if any(parent < 0 or parent >= index for parent in task["parents"]):
            raise ValueError("reference tasks must be in dependency order")
        cost = rtt_ms + max(
            0,
            profile.get("path_latency_ms", {}).get(
                task["path"], profile.get("latency_ms", 0)
            )
            - profile.get("jitter_ms", 0),
        )
        costs.append(cost)
        depths.append(
            1 + max((depths[parent] for parent in task["parents"]), default=0)
        )
        arrivals.append(
            cost + max((arrivals[parent] for parent in task["parents"]), default=0)
        )
    waves = max(max(depths, default=0), math.ceil(len(tasks) / concurrency))
    latency = max(
        max(arrivals, default=0),
        sum(costs) / concurrency,
        waves * min(costs, default=0),
        waves * rtt_ms,
    )
    # network_floor charges RTTs separately. Subtract that charge so independent
    # capacity and dependency-path bounds are not added together.
    required_latency = latency - waves * rtt_ms
    return {
        "required_bytes": required_bytes,
        "required_waves": waves,
        "required_latency_ms": required_latency,
        "optimistic_latency_bound_ms": latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, waves, required_latency, netem=netem
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--concurrency", type=int, default=50)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("concurrency must be positive")
    manifest = args.directory / "tool-upgrade-fixtures.json"
    profile = json.loads((args.directory / "tool-upgrade-profiles.json").read_text())[
        args.profile
    ]
    scenario = json.loads(
        (args.directory / "tool-upgrade-descriptor.json").read_text()
    )["scenarios"][args.scenario]
    fixtures = bench.Fixtures(manifest, args.directory, profile.get("pep658", True))
    tasks = requests(fixtures, profile, scenario)
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    local = threading.local()
    connections = []
    connection_lock = threading.Lock()

    def fetch(task: dict) -> dict:
        if not hasattr(local, "connection"):
            local.connection = http.client.HTTPConnection(
                "127.0.0.1", server.server_port, timeout=60
            )
            with connection_lock:
                connections.append(local.connection)
        connection = local.connection
        started = time.perf_counter() - server.epoch
        for attempt in range(5):
            connection.request("GET", task["path"])
            response = connection.getresponse()
            body = response.read()
            if response.status == 200 and body == task["expected"]:
                return {
                    "started": started,
                    "completed": time.perf_counter() - server.epoch,
                    "bytes": len(body),
                    "sha256": hashlib.sha256(body).hexdigest(),
                }
            if response.status not in {408, 429, 500, 502, 503, 504} or attempt == 4:
                raise ValueError(f"Tool-upgrade reference differs: {task['path']}")

    client_transfers = {}
    started = time.perf_counter()
    try:
        waiting, finished, active = set(range(len(tasks))), set(), {}
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=args.concurrency
        ) as pool:
            while waiting or active:
                for index in sorted(waiting):
                    if (
                        len(active) < args.concurrency
                        and tasks[index]["parents"] <= finished
                    ):
                        waiting.remove(index)
                        active[pool.submit(fetch, tasks[index])] = index
                if not active:
                    raise ValueError("Reference request graph cannot make progress")
                completed, _ = concurrent.futures.wait(
                    active, return_when=concurrent.futures.FIRST_COMPLETED
                )
                for future in completed:
                    transfer = future.result()
                    index = active.pop(future)
                    client_transfers[tasks[index]["path"]] = transfer
                    finished.add(index)
        seconds = time.perf_counter() - started
    finally:
        for connection in connections:
            connection.close()
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    report = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(manifest),
        "scenario": args.scenario,
        "concurrency": args.concurrency,
        "seconds": seconds,
        **bounds(tasks, profile, args.concurrency),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "client_transfers": dict(sorted(client_transfers.items())),
        "client_timing_reference": "Seconds from the replay origin's monotonic epoch; completion follows full response-body verification.",
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "Known dependency graph, distinct registry project/index pairs, and full selected wheels. Artifact bodies are fetched once after a referring project page. Metadata sidecars are unnecessary when the full wheel is required. The bound omits connection setup, failures, installation, and local processing; pinned and local-source scenarios use a conservative zero bound.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "events"}))


if __name__ == "__main__":
    main()
