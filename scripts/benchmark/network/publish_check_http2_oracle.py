"""Check known published distributions through a bounded HTTP/2 connection pool."""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import importlib.util
import json
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "publish_check_oracle", Path(__file__).with_name("publish_check_oracle.py")
)
assert spec is not None and spec.loader is not None
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)
bench = publish.bench


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", action="append", default=[])
    parser.add_argument("--route", choices=["current", "revalidate"], default="current")
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
    try:
        entries, packages = publish.selected_distributions(args.manifest, args.filename)
    except ValueError as error:
        parser.error(str(error))
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(
        args.manifest, args.directory, profile.get("pep658", True)
    )
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="publish-http2-oracle-", dir=args.work_dir
    ) as temporary:
        work = Path(temporary).resolve()
        server = bench.Server(fixtures, profile, work / "origin.sock")
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        proxy = None
        try:
            proxy = bench.Http2Proxy(
                work,
                args.http2_proxy.resolve(),
                args.tls_certificate.resolve(),
                args.tls_key.resolve(),
            )
            server.public_url = proxy.url
            command = [
                str(args.curl.resolve()),
                "--disable",
                "--parallel",
                "--parallel-max",
                str(args.concurrency),
            ]
            expected_transfers = {}
            for index, package in enumerate(packages):
                if index:
                    command.append("--next")
                url = server.url + f"/simple/{package}/"
                body_path = work / f"index-{index}.json"
                headers_path = work / f"headers-{index}.txt"
                expected = fixtures.simple[package]
                etag = '"' + hashlib.sha256(expected).hexdigest() + '"'
                expected_transfers[url] = (
                    package,
                    body_path,
                    headers_path,
                    expected,
                    etag,
                )
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
                        "60",
                        "--retry",
                        "3",
                        "--retry-max-time",
                        "60",
                        "--dump-header",
                        str(headers_path),
                        "--write-out",
                        "%{json}\n",
                        "--output",
                        str(body_path),
                    ]
                )
                if args.route == "revalidate":
                    command.extend(["--header", "If-None-Match: " + etag])
                command.append(url)
            started = time.perf_counter()
            completed = subprocess.run(
                command, capture_output=True, text=True, check=False, timeout=300
            )
            if completed.returncode:
                raise RuntimeError(
                    f"curl exited with {completed.returncode}: {completed.stderr}"
                )
            transfers = [json.loads(line) for line in completed.stdout.splitlines()]
            by_url = {transfer["url_effective"]: transfer for transfer in transfers}
            if (
                len(by_url) != len(transfers)
                or by_url.keys() != expected_transfers.keys()
            ):
                raise ValueError("Oracle transfers differ from selected projects")
            if any(str(transfer["http_version"]) != "2" for transfer in transfers):
                raise ValueError("Oracle transfers did not all use HTTP/2")
            indexes = {}
            for url, (
                package,
                body_path,
                headers_path,
                expected,
                etag,
            ) in expected_transfers.items():
                status = by_url[url]["http_code"]
                body = body_path.read_bytes() if body_path.exists() else b""
                if args.route == "revalidate":
                    blocks = headers_path.read_bytes().split(b"\r\n\r\n")
                    headers = next(
                        block
                        for block in reversed(blocks)
                        if block.startswith(b"HTTP/")
                    )
                    status_line, _, fields = headers.partition(b"\r\n")
                    if int(status_line.split()[1]) != status:
                        raise ValueError(f"Response header status differs: {package}")
                    response_etag = (
                        email.parser.BytesHeaderParser().parsebytes(fields).get("ETag")
                    )
                    if status != 304 or body or response_etag != etag:
                        raise ValueError(
                            f"Conditional index response differs: {package} "
                            f"(status={status}, bytes={len(body)}, etag={response_etag!r})"
                        )
                    body = expected
                elif status != 200 or body != expected:
                    raise ValueError(f"Index response differs: {package} ({status})")
                indexes[package] = {
                    entry["filename"]: entry for entry in json.loads(body)["files"]
                }
            publish.verify_distributions(args.directory, entries, indexes)
            seconds = time.perf_counter() - started
        finally:
            if proxy:
                proxy.stop()
            server.shutdown()
            server.wait_idle()
            server.server_close()
            thread.join()
        protocols = proxy.protocols()
        if protocols["HTTP/2.0"] != len(server.events):
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
        "manifest_sha256": bench.digest(args.manifest),
        "filenames": sorted({entry["filename"] for entry in entries}),
        "route": args.route,
        "concurrency": args.concurrency,
        "seconds": seconds,
        **publish.lower_bound(
            fixtures, packages, profile, args.route, args.concurrency
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
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
        "transfers": transfers,
        "events": server.events,
        "retry_scope": "curl retries transient failures up to three times per URL using its default backoff. The optimistic floor excludes retries.",
        "scope": "Known local files and package names. Retrieve each required current or conditionally revalidated index through verified TLS/HTTP2, within the configured transfer limit, and verify every selected file's SHA-256. Frontend connections count distinct client endpoints in the proxy access log; curl's per-transfer connection count may omit earlier retry attempts. The realizable reference includes connection startup and curl retry waits; the floor excludes hashing, headers, connection startup, and other CPU work.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                key: value
                for key, value in result.items()
                if key not in {"events", "transfers"}
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
