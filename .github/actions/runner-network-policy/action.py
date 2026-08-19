"""Run the disposable Linux runner's pre, main, and post hooks."""

import http.client
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

from policy import load, private_origins

DIRECTORY = Path("/run/uv-network-policy")


def health():
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=5)
    try:
        connection.request("GET", "/__uv_network_proxy_health")
        if connection.getresponse().status != 204:
            raise RuntimeError("network proxy health check failed")
    finally:
        connection.close()


def export(name, value):
    if any(character in name + value for character in "\r\n"):
        raise ValueError("invalid environment value")
    with Path(os.environ["GITHUB_ENV"]).open("a") as destination:
        destination.write(f"{name}={value}\n")


def pre():
    if (
        sys.platform != "linux"
        or os.environ.get("INPUT_DISPOSABLE") != "true"
        or os.environ.get("GITHUB_ACTIONS") != "true"
    ):
        raise RuntimeError("explicitly confirmed disposable Linux VM required")
    if Path("/.dockerenv").exists() or os.geteuid() == 0:
        raise RuntimeError("job containers and root jobs are not supported")
    profile = os.environ["INPUT_PROFILE"]
    privileges = os.environ.get("INPUT_PRIVILEGES", "drop")
    load(Path(__file__).with_name("policies.json"), profile)
    if privileges not in {"drop", "retain"}:
        raise ValueError("unknown privilege mode")
    if not shutil.which("nft"):
        # This is still the trusted bootstrap, before any later action hooks.
        subprocess.run(["sudo", "-n", "apt-get", "update"], check=True)
        subprocess.run(
            [
                "sudo",
                "-n",
                "apt-get",
                "install",
                "-y",
                "--no-install-recommends",
                "nftables",
            ],
            check=True,
        )
    subprocess.run(
        [
            "sudo",
            "-n",
            "/usr/bin/python3",
            "-E",
            "-s",
            str(Path(__file__).with_name("install.py")),
            profile,
            privileges,
            str(os.getuid()),
            json.dumps(private_origins(os.environ)),
        ],
        check=True,
    )
    health()
    for name in (
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "all_proxy",
    ):
        export(name, "http://127.0.0.1:18080")
    for name in ("NO_PROXY", "no_proxy"):
        export(name, "localhost,127.0.0.1,::1")
    export("NODE_USE_ENV_PROXY", "1")
    export("UV_NETWORK_POLICY_ACTIVE", "1")
    export("UV_NETWORK_POLICY_PROFILE", profile)
    export("UV_NETWORK_POLICY_AUDIT", str(DIRECTORY / "audit/events.json"))


def post():
    if not DIRECTORY.exists():
        return
    health()
    events = json.loads((DIRECTORY / "audit/events.json").read_text())
    destination = Path(os.environ["RUNNER_TEMP"]) / "network-policy-audit.json"
    destination.write_text(json.dumps(events, indent=2) + "\n")
    denied = sum(event["count"] for event in events if event["event"] == "denied")
    print(
        f"Network policy recorded {denied} denied requests; policy remains active until VM teardown."
    )


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in {"pre", "main", "post"}:
        raise ValueError("unknown action operation")
    {"pre": pre, "main": health, "post": post}[sys.argv[1]]()


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError):
        print("::error::Runner network policy failed", file=sys.stderr)
        raise SystemExit(1) from None
