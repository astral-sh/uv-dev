"""Measure a verified, connection-reusing HTTP/2 transfer independently of uv."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path

from bench import (
    HERE,
    Fixtures,
    Http2Proxy,
    Server,
    digest,
    netem_profile,
    network_floor,
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=HERE / "fixtures.json")
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=HERE / "profiles.json")
    parser.add_argument("--profile", default="fast")
    parser.add_argument("--filename", required=True)
    parser.add_argument("--curl", type=Path, default=shutil.which("curl"))
    parser.add_argument("--http2-proxy", type=Path, required=True)
    parser.add_argument("--tls-certificate", type=Path, required=True)
    parser.add_argument("--tls-key", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.curl is None:
        parser.error("curl is required")
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = Fixtures(args.manifest, args.directory, profile.get("pep658", True))
    package = next(
        name
        for name, files in fixtures.packages.items()
        if any(file["filename"] == args.filename for file in files)
    )
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="http2-oracle-", dir=args.work_dir
    ) as trial:
        work = Path(trial).resolve()
        server = Server(fixtures, profile, work / "origin.sock")
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        proxy = None
        try:
            proxy = Http2Proxy(
                work,
                args.http2_proxy.resolve(),
                args.tls_certificate.resolve(),
                args.tls_key.resolve(),
            )
            server.public_url = proxy.url
            command = [
                str(args.curl.resolve()),
                "--fail",
                "--silent",
                "--show-error",
                "--http2",
                "--cacert",
                str(args.tls_certificate.resolve()),
                "--noproxy",
                "*",
                "--write-out",
                "%{json}\n",
                "--output",
                str(work / "index.json"),
                f"{server.url}/simple/{package}/",
                "--output",
                str(work / "artifact"),
                f"{server.url}/files/{args.filename}",
            ]
            start = time.perf_counter()
            completed = subprocess.run(
                command, capture_output=True, text=True, check=True, timeout=300
            )
            seconds = time.perf_counter() - start
            transfers = [json.loads(line) for line in completed.stdout.splitlines()]
            if len(transfers) != 2 or any(
                str(transfer["http_version"]) != "2" for transfer in transfers
            ):
                raise ValueError("Oracle transfers did not both use HTTP/2")
            if sum(transfer["num_connects"] for transfer in transfers) != 1:
                raise ValueError("Oracle did not reuse its connection")
            if (work / "index.json").read_bytes() != fixtures.simple[package]:
                raise ValueError("Oracle index bytes differ")
            if digest(work / "artifact") != fixtures.hashes[args.filename]:
                raise ValueError("Oracle artifact bytes differ")
        finally:
            if proxy:
                proxy.stop()
            server.shutdown()
            server.server_close()
            thread.join()
        required_bytes = (
            len(fixtures.simple[package]) + fixtures.files[args.filename].stat().st_size
        )
        data = {
            "profile": profile,
            "netem": netem_profile(),
            "filename": args.filename,
            "artifact_sha256": fixtures.hashes[args.filename],
            "manifest_sha256": digest(args.manifest),
            "seconds": seconds,
            "required_artifact_and_index_bytes": required_bytes,
            "optimistic_network_floor_seconds": network_floor(
                profile, required_bytes, 2
            ),
            "actual_bytes": sum(event["bytes"] for event in server.events),
            "requests": len(server.events),
            "frontend_protocols": proxy.protocols(),
            "curl": {
                "sha256": digest(args.curl),
                "version": subprocess.check_output(
                    [args.curl, "--version"], text=True
                ).splitlines()[0],
            },
            "proxy_sha256": digest(args.http2_proxy),
            "certificate_sha256": digest(args.tls_certificate),
            "transfers": transfers,
            "events": server.events,
            "scope": "Known artifact, sequential index and full-artifact GETs on one HTTP/2 connection. Includes connection startup and response bodies; excludes resolution and installation.",
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(data, indent=2) + "\n")
        print(
            json.dumps(
                {
                    key: value
                    for key, value in data.items()
                    if key not in {"events", "transfers"}
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
