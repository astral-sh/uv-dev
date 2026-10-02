"""Fetch a known tool-upgrade request graph through a bounded HTTP/2 pool."""

from __future__ import annotations

import argparse
import importlib.util
import json
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "tool_upgrade_oracle", Path(__file__).with_name("tool_upgrade_oracle.py")
)
assert spec is not None and spec.loader is not None
upgrade = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upgrade)
bench = upgrade.bench


def fetch_wave(
    args, work: Path, base: str, tasks: list[dict], ready: list[int]
) -> list[dict]:
    command = [
        str(args.curl.resolve()),
        "--disable",
        "--parallel",
        "--parallel-max",
        str(args.concurrency),
    ]
    expected_transfers = {}
    for index in ready:
        if expected_transfers:
            command.append("--next")
        task = tasks[index]
        url = base + task["path"]
        body_path = work / f"body-{index}"
        expected_transfers[url] = (body_path, task["expected"])
        command.extend(
            [
                "--silent",
                "--show-error",
                "--http2",
                "--cacert",
                str(args.tls_certificate.resolve()),
                "--noproxy",
                "*",
                "--max-time",
                "300",
                "--retry",
                "3",
                "--retry-max-time",
                "300",
                "--write-out",
                "%{json}\n",
                "--output",
                str(body_path),
                url,
            ]
        )
    if len(expected_transfers) != len(ready):
        raise ValueError("Reference task paths must be distinct")
    completed = subprocess.run(
        command, capture_output=True, text=True, check=False, timeout=1200
    )
    if completed.returncode:
        raise RuntimeError(
            f"curl exited with {completed.returncode}: {completed.stderr}"
        )
    transfers = [json.loads(line) for line in completed.stdout.splitlines()]
    by_url = {transfer["url_effective"]: transfer for transfer in transfers}
    if len(by_url) != len(transfers) or by_url.keys() != expected_transfers.keys():
        raise ValueError("Oracle transfers differ from ready requests")
    for url, (body_path, expected) in expected_transfers.items():
        transfer = by_url[url]
        if str(transfer["http_version"]) != "2":
            raise ValueError("Oracle transfer did not use HTTP/2")
        if transfer["http_code"] != 200 or body_path.read_bytes() != expected:
            raise ValueError(f"Tool-upgrade reference differs: {url}")
    return transfers


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--concurrency", type=int, default=50)
    parser.add_argument("--curl", type=Path, default=shutil.which("curl"))
    parser.add_argument("--http2-proxy", type=Path, required=True)
    parser.add_argument("--tls-certificate", type=Path, required=True)
    parser.add_argument("--tls-key", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.concurrency <= 65535:
        parser.error("concurrency must be between 1 and 65535")
    if args.curl is None:
        parser.error("curl is required")
    manifest = args.directory / "tool-upgrade-fixtures.json"
    profile = json.loads((args.directory / "tool-upgrade-profiles.json").read_text())[
        args.profile
    ]
    scenario = json.loads(
        (args.directory / "tool-upgrade-descriptor.json").read_text()
    )["scenarios"][args.scenario]
    fixtures = bench.Fixtures(manifest, args.directory, profile.get("pep658", True))
    tasks = upgrade.requests(fixtures, profile, scenario)
    bounds = upgrade.bounds(tasks, profile, args.concurrency)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="tool-upgrade-h2-", dir=args.work_dir
    ) as temporary:
        work = Path(temporary).resolve()
        server = bench.Server(fixtures, profile, work / "origin.sock")
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        proxy = None
        transfers, waves = [], []
        try:
            proxy = bench.Http2Proxy(
                work,
                args.http2_proxy.resolve(),
                args.tls_certificate.resolve(),
                args.tls_key.resolve(),
            )
            server.public_url = proxy.url
            waiting, finished = set(range(len(tasks))), set()
            started = time.perf_counter()
            while waiting:
                ready = sorted(
                    index for index in waiting if tasks[index]["parents"] <= finished
                )
                if not ready:
                    raise ValueError("Reference request graph cannot make progress")
                transfers.extend(fetch_wave(args, work, server.url, tasks, ready))
                waves.append([tasks[index]["path"] for index in ready])
                finished.update(ready)
                waiting.difference_update(ready)
            seconds = time.perf_counter() - started
        finally:
            if proxy:
                proxy.stop()
            server.shutdown()
            server.wait_idle()
            server.server_close()
            thread.join()
        protocols = proxy.protocols() if tasks else {}
        if protocols.get("HTTP/2.0", 0) != len(server.events):
            raise ValueError("HTTP/2 and replay request counts differ")
        connections = {
            (event["request"]["remote_ip"], event["request"]["remote_port"])
            for line in proxy.log_path.read_text().splitlines()
            if (event := json.loads(line))
            .get("logger", "")
            .startswith("http.log.access")
        }
    result = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(manifest),
        "scenario": args.scenario,
        "concurrency": args.concurrency,
        "seconds": seconds,
        **bounds,
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "origin_connections": len(
            {event["origin_connection"] for event in server.events}
        ),
        "frontend_protocols": protocols,
        "frontend_connections": len(connections),
        "curl_connections_reported": sum(
            transfer["num_connects"] for transfer in transfers
        ),
        "curl": {
            "sha256": bench.digest(args.curl),
            "version": subprocess.check_output(
                [args.curl, "--version"], text=True
            ).splitlines()[0],
        },
        "proxy_sha256": bench.digest(args.http2_proxy),
        "certificate_sha256": bench.digest(args.tls_certificate),
        "transfer_waves": waves,
        "transfers": transfers,
        "events": sorted(server.events, key=lambda event: event["start"]),
        "retry_scope": "curl retries transient failures up to three times per URL with its default backoff. The optimistic floor excludes retries.",
        "scope": "Known dependency graph, distinct registry project/index pairs, and full selected wheels. Each dependency-ready wave uses a bounded curl HTTP/2 pool and completes before the next wave starts. The realizable reference includes each wave's connection setup, scheduling barrier, and retry waits. The optimistic floor allows pipeline overlap and excludes connection setup, failures, installation, and local processing; pinned and local-source scenarios have a conservative zero bound.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                key: value
                for key, value in result.items()
                if key not in {"events", "transfers"}
            }
        )
    )


if __name__ == "__main__":
    main()
