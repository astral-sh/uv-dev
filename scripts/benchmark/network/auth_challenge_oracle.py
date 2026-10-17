"""Fetch one known distribution while cancelling unneeded authentication bodies."""

from __future__ import annotations

import argparse
import http.client
import importlib.util
import json
import threading
import time
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
    parser.add_argument("--filename", required=True)
    parser.add_argument("--route", choices=["metadata", "wheel"], default="metadata")
    parser.add_argument("--requirements-path")
    parser.add_argument("--preauthenticated", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    if any(profile.get(key) for key in ("path_failures", "fail_count", "cut_count")):
        parser.error("the authentication oracle does not model transient failures")
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    matches = [
        (name, entry)
        for name, entries in fixtures.packages.items()
        for entry in entries
        if entry["filename"] == args.filename
    ]
    if len(matches) != 1 or (
        args.route == "metadata" and not matches[0][1]["core-metadata"]
    ):
        parser.error("select a distribution with an available retrieval route")
    package, entry = matches[0]
    paths = [(f"/simple/{package}/", fixtures.simple[package])]
    if args.route == "metadata":
        paths.append(
            (entry["url"] + ".metadata", fixtures.metadata[args.filename + ".metadata"])
        )
    else:
        paths.append((entry["url"], fixtures.files[args.filename].read_bytes()))
    if args.requirements_path:
        paths.insert(
            0,
            (
                args.requirements_path,
                fixtures.routes[args.requirements_path].read_bytes(),
            ),
        )
    authorization = None
    if args.preauthenticated:
        credentials = {
            challenge["authorization"]
            for challenge in profile.get("auth_challenges", {}).values()
        }
        if len(credentials) != 1:
            parser.error("preauthentication requires one replay credential")
        authorization = credentials.pop()
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=60)
    challenges = []
    started = time.perf_counter()
    try:
        for path, expected in paths:
            headers = {"Authorization": authorization} if authorization else {}
            connection.request("GET", path, headers=headers)
            response = connection.getresponse()
            challenge = profile.get("auth_challenges", {}).get(path)
            if challenge is not None and authorization != challenge["authorization"]:
                if response.status != challenge.get("status", 401):
                    raise ValueError(f"Oracle challenge differs: {path}")
                response.close()
                connection.close()
                challenges.append(path)
                authorization = challenge["authorization"]
                connection = http.client.HTTPConnection(
                    "127.0.0.1", server.server_port, timeout=60
                )
                connection.request(
                    "GET", path, headers={"Authorization": authorization}
                )
                response = connection.getresponse()
            if response.status != 200 or response.read() != expected:
                raise ValueError(f"Oracle response differs: {path}")
        seconds = time.perf_counter() - started
    finally:
        connection.close()
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()

    def latency(path: str) -> float:
        return max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )

    required_bytes = sum(len(body) for _, body in paths)
    required_waves = len(paths) + len(challenges)
    preauthenticated_latency = profile.get("connection_latency_ms", 0) + sum(
        latency(path) for path, _ in paths
    )
    required_latency = preauthenticated_latency + sum(map(latency, challenges))
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "route": args.route,
        "requirements_path": args.requirements_path,
        "preauthenticated": args.preauthenticated,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": required_waves,
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, required_waves, required_latency
        ),
        "preauthenticated_network_floor_seconds": bench.network_floor(
            profile, required_bytes, len(paths), preauthenticated_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "challenge_paths": challenges,
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "One known distribution over HTTP/1.1. Unless preauthentication is selected, the reference starts without credentials, cancels each unfamiliar authentication challenge after its headers, and reuses the resulting credentials within the replay origin. It verifies all selected response bodies. The primary bound includes the required challenge waves but no error-body bytes; the additional preauthenticated bound permits credentials on the first request. Both omit HTTP headers, TCP/TLS startup, and CPU work.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "events"}))


if __name__ == "__main__":
    main()
