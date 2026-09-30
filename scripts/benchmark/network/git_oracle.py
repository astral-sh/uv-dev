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
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    descriptor = json.loads((args.directory / "git-descriptor.json").read_text())
    fixtures = bench.Fixtures(
        args.directory / "git-fixtures.json", args.directory, True
    )
    server = bench.Server(fixtures, profile, git_root=args.directory / "git")
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
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
    try:
        with tempfile.TemporaryDirectory(
            prefix="git-oracle-", dir=args.work_dir
        ) as temporary:
            repository = Path(temporary) / "checkout.git"
            subprocess.run(
                ["git", "init", "--bare", str(repository)],
                env=env,
                capture_output=True,
                check=True,
            )
            command = [
                "git",
                "--git-dir",
                str(repository),
                "fetch",
                "--no-tags",
                "--force",
                server.url + "/git/monorepo.git",
                "refs/heads/main:refs/heads/main",
            ]
            if args.warm:
                subprocess.run(
                    command, env=env, capture_output=True, check=True, timeout=60
                )
                server.reset()
            start = time.perf_counter()
            subprocess.run(
                command, env=env, capture_output=True, check=True, timeout=60
            )
            seconds = time.perf_counter() - start
            commit = subprocess.check_output(
                ["git", "--git-dir", str(repository), "rev-parse", "refs/heads/main"],
                env=env,
                text=True,
            ).strip()
            if commit != descriptor["commit"]:
                raise ValueError("Fetched Git commit differs from the pinned fixture")
    finally:
        server.shutdown()
        server.wait_idle()
        server.server_close()
        thread.join()
    waves = 1 if args.warm else 2
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "commit": commit,
        "bundle_sha256": fixtures.hashes["monorepo.bundle"],
        "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
        "warm": args.warm,
        "seconds": seconds,
        "requests": len(server.events),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "required_bytes": 0,
        "required_waves": waves,
        "optimistic_network_floor_seconds": bench.network_floor(profile, 0, waves),
        "events": sorted(server.events, key=lambda event: event["start"]),
        "scope": "A single Git CLI fetch of the known branch, verified against the pinned commit. The optimistic floor charges reference discovery and, for a cold fetch, one dependent pack response. Git pack compression and negotiation prevent treating the observed bytes as a strict minimum, so the byte floor is zero. The single-fetch bytes and time are a realizable reference.",
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
