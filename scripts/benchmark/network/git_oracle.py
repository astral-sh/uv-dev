"""Measure one known Git fetch against the smart-HTTP replay origin."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import tempfile
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
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--warm", action="store_true")
    parser.add_argument("--revision", help="Fetch the fixture's known commit directly")
    parser.add_argument("--http2-proxy", type=Path, help="Path to the Caddy binary")
    parser.add_argument("--tls-certificate", type=Path)
    parser.add_argument("--tls-key", type=Path)
    args = parser.parse_args()
    transport = (args.http2_proxy, args.tls_certificate, args.tls_key)
    if any(transport) and not all(transport):
        parser.error("HTTP/2 requires the proxy binary, certificate, and key")
    if args.http2_proxy:
        args.http2_proxy = args.http2_proxy.resolve()
        args.tls_certificate = args.tls_certificate.resolve()
        args.tls_key = args.tls_key.resolve()
    profile = json.loads(args.profiles.read_text())[args.profile]
    descriptor = json.loads((args.directory / "git-descriptor.json").read_text())
    if args.revision is not None and args.revision != descriptor["commit"]:
        parser.error("--revision must equal the fixture's full commit")
    fixtures = bench.Fixtures(
        args.directory / "git-fixtures.json", args.directory, True
    )
    env = {
        key: value for key, value in os.environ.items() if not key.startswith("GIT_")
    }
    env.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        NO_PROXY="127.0.0.1,localhost",
        no_proxy="127.0.0.1,localhost",
    )
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="git-oracle-", dir=args.work_dir
    ) as temporary:
        work = Path(temporary)
        server = bench.Server(
            fixtures,
            profile,
            work / "origin.sock" if args.http2_proxy else None,
            git_root=args.directory / "git",
        )
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        proxy = None
        warm_requests = 0
        protocols = None
        try:
            if args.http2_proxy:
                proxy = bench.Http2Proxy(
                    work, args.http2_proxy, args.tls_certificate, args.tls_key
                )
                server.public_url = proxy.url
                env.update(
                    GIT_CONFIG_COUNT="1",
                    GIT_CONFIG_KEY_0="http.version",
                    GIT_CONFIG_VALUE_0="HTTP/2",
                    GIT_SSL_CAINFO=str(args.tls_certificate),
                )
            repository = work / "checkout.git"
            subprocess.run(
                ["git", "init", "--bare", str(repository)],
                env=env,
                capture_output=True,
                check=True,
            )
            source_ref = args.revision or "refs/heads/main"
            target_ref = "refs/benchmark/commit" if args.revision else "refs/heads/main"
            command = [
                "git",
                "--git-dir",
                str(repository),
                "fetch",
                "--no-tags",
                "--force",
                server.url + "/git/monorepo.git",
                f"{source_ref}:{target_ref}",
            ]
            if args.warm:
                subprocess.run(
                    command, env=env, capture_output=True, check=True, timeout=60
                )
                server.wait_idle()
                warm_requests = len(server.events)
                server.reset()
            start = time.perf_counter()
            subprocess.run(
                command, env=env, capture_output=True, check=True, timeout=60
            )
            seconds = time.perf_counter() - start
            commit = subprocess.check_output(
                ["git", "--git-dir", str(repository), "rev-parse", target_ref],
                env=env,
                text=True,
            ).strip()
            if commit != descriptor["commit"]:
                raise ValueError("Fetched Git commit differs from the pinned fixture")
        finally:
            if proxy:
                proxy.stop()
            server.shutdown()
            server.wait_idle()
            server.server_close()
            thread.join()
        if proxy:
            protocols = proxy.protocols()
            if protocols["HTTP/2.0"] != warm_requests + len(server.events):
                raise ValueError("Git HTTP/2 and replay request counts differ")
            protocols["HTTP/2.0"] -= warm_requests
    waves = (0 if args.revision else 1) if args.warm else 2
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "commit": commit,
        "bundle_sha256": fixtures.hashes["monorepo.bundle"],
        "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
        "warm": args.warm,
        "revision": args.revision,
        "http2_proxy": (
            {
                "binary": str(args.http2_proxy),
                "version": subprocess.check_output(
                    [args.http2_proxy, "version"], text=True
                ).strip(),
                "sha256": bench.digest(args.http2_proxy),
                "certificate_sha256": bench.digest(args.tls_certificate),
            }
            if args.http2_proxy
            else None
        ),
        "frontend_protocols": protocols,
        "seconds": seconds,
        "requests": len(server.events),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "required_bytes": 0,
        "required_waves": waves,
        "optimistic_network_floor_seconds": bench.network_floor(profile, 0, waves),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "A single Git CLI fetch of the known reference, verified against the pinned commit. The optimistic floor charges reference discovery and, for a cold fetch, one dependent pack response. A cached exact commit needs no network request; a named reference must be checked for updates. Git pack compression and negotiation prevent treating the observed bytes as a strict minimum, so the byte floor is zero. The single-fetch bytes and time are a realizable reference.",
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
