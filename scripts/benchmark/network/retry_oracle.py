"""Retrieve known metadata while following bounded HTTP Retry-After advice."""

from __future__ import annotations

import argparse
import email.utils
import http.client
import importlib.util
import json
import re
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


def retry_delay(value: str | None, maximum: float) -> float:
    if value is None:
        return 0
    value = value.strip()
    if re.fullmatch(r"[0-9]+", value):
        return min(int(value), maximum)
    try:
        return min(
            max(0, email.utils.parsedate_to_datetime(value).timestamp() - time.time()),
            maximum,
        )
    except (TypeError, ValueError, OverflowError):
        return 0


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", required=True)
    parser.add_argument("--flat-index-path")
    parser.add_argument("--max-delay", type=float, default=30)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.max_delay < 0:
        parser.error("--max-delay cannot be negative")
    if (
        args.flat_index_path is not None
        and re.fullmatch(r"/flat/[A-Za-z0-9_-]+", args.flat_index_path) is None
    ):
        parser.error("--flat-index-path must identify one replay find-links page")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    matches = [
        (name, entry)
        for name, entries in fixtures.packages.items()
        for entry in entries
        if entry["filename"] == args.filename
    ]
    if len(matches) != 1 or not matches[0][1]["core-metadata"]:
        parser.error("select a distribution with advertised metadata")
    package, entry = matches[0]
    paths = [
        (
            (args.flat_index_path, fixtures.flat)
            if args.flat_index_path
            else (f"/simple/{package}/", fixtures.simple[package])
        ),
        (entry["url"] + ".metadata", fixtures.metadata[args.filename + ".metadata"]),
    ]
    server = bench.Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    sleeps = []

    def read(path: str, expected: bytes) -> None:
        for attempt in range(4):
            try:
                with opener.open(server.url + path, timeout=60) as response:
                    if response.read() != expected:
                        raise ValueError(f"Oracle response differs: {path}")
                return
            except urllib.error.HTTPError as error:
                if attempt == 3 or error.code not in {408, 429, 500, 502, 503, 504}:
                    raise
                error.read()
                delay = retry_delay(error.headers.get("Retry-After"), args.max_delay)
                sleeps.append(delay)
                time.sleep(delay)
            except (
                urllib.error.URLError,
                http.client.RemoteDisconnected,
                http.client.IncompleteRead,
                ConnectionError,
            ):
                if attempt == 3:
                    raise
                sleeps.append(0)

    started = time.perf_counter()
    try:
        for path, expected in paths:
            read(path, expected)
        seconds = time.perf_counter() - started
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    required_bytes = sum(len(body) for _, body in paths)
    required_waves = len(paths)
    required_latency = 0
    required_wait = 0
    for path, body in paths:
        failure = profile.get("path_failures", {}).get(
            path,
            {
                "status": profile.get("fail_status"),
                "count": profile.get("fail_count", 0),
            },
        )
        cut = failure.get("cut_after_bytes", 0)
        truncated = 0 < cut < len(body)
        count = (
            failure.get("count", 0)
            if failure.get("status") or failure.get("disconnect") or truncated
            else 0
        )
        if truncated and not failure.get("status") and not failure.get("disconnect"):
            required_bytes += count * cut
        required_waves += count
        latency = max(
            0,
            profile.get("path_latency_ms", {}).get(path, profile.get("latency_ms", 0))
            - profile.get("jitter_ms", 0),
        )
        required_latency += (1 + count) * latency
        value = str(failure.get("retry_after", ""))
        if re.fullmatch(r"[0-9]+", value):
            required_wait += min(int(value), args.max_delay) * count * 1000
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "flat_index_path": args.flat_index_path,
        "max_delay_seconds": args.max_delay,
        "seconds": seconds,
        "required_bytes": required_bytes,
        "required_waves": required_waves,
        "required_latency_ms": required_latency,
        "required_wait_ms": required_wait,
        "optimistic_network_floor_seconds": bench.network_floor(
            profile, required_bytes, required_waves, required_latency, required_wait
        ),
        "observed_retry_delays_seconds": sleeps,
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "One known distribution. Fetch and verify its index and advertised metadata, retrying transient HTTP responses, disconnected requests, and truncated bodies up to three times per URL. Valid server delays are capped at the recorded maximum; missing or invalid advice and transport failures have zero oracle backoff. The bound includes required truncated prefixes and fixed numeric Retry-After waits on this serial request chain. HTTP-date waits have a conservative zero minimum because clock and whole-second rounding vary. It excludes headers, TCP/TLS, and CPU costs.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in result.items() if key != "events"}, indent=2
        )
    )


if __name__ == "__main__":
    main()
