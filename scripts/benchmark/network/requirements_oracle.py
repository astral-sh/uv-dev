"""Measure a remote requirements chain over one persistent HTTP/1.1 connection."""

from __future__ import annotations

import argparse
import http.client
import importlib.util
import json
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=bench.HERE / "profiles.json")
    parser.add_argument("--profile", default="fast")
    parser.add_argument("--root", default="/requirements/part0.txt")
    parser.add_argument("--filename", default="iniconfig-2.1.0-py3-none-any.whl")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    if not profile.get("pep658", True):
        parser.error("the oracle requires PEP 658")
    fixtures = bench.Fixtures(args.manifest, args.directory, True)
    package = next(
        name
        for name, files in fixtures.packages.items()
        if any(file["filename"] == args.filename for file in files)
    )
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=60)
    first_socket = None
    paths = []
    required_bytes = 0

    def read(path: str, expected: bytes) -> bytes:
        nonlocal first_socket, required_bytes
        connection.request("GET", path)
        if first_socket is None:
            first_socket = connection.sock
        elif connection.sock is not first_socket:
            raise ValueError("Oracle opened another connection")
        response = connection.getresponse()
        body = response.read()
        if response.status != 200 or body != expected:
            raise ValueError(f"Oracle response differs: {path}")
        paths.append(path)
        required_bytes += len(body)
        return body

    start = time.perf_counter()
    try:
        path = args.root
        seen = set()
        while True:
            if path in seen:
                raise ValueError("Requirements chain contains a cycle")
            seen.add(path)
            body = read(path, fixtures.routes[path].read_bytes())
            if not body.startswith(b"-r "):
                break
            path = urllib.parse.urljoin(path, body[3:].decode().strip())
        read(f"/simple/{package}/", fixtures.simple[package])
        read(
            f"/files/{args.filename}.metadata",
            fixtures.metadata[args.filename + ".metadata"],
        )
        seconds = time.perf_counter() - start
    finally:
        connection.close()
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_latency = sum(
        max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )
        for path in paths
    )
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "paths": paths,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": len(paths),
        "required_latency_ms": required_latency,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, len(paths), required_latency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "Sequential requirements-chain, index, and metadata GETs on one HTTP/1.1 connection. Includes connection startup; selected version is known, and resolution is excluded.",
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
