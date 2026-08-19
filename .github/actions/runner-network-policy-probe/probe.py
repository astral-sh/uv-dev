"""Harmless hosted acceptance checks. Never prints response bodies or tokens."""

import http.client
import ipaddress
import json
import os
import socket
import ssl
import struct
import subprocess
import sys
from pathlib import Path

DENIED = "example.com"
ALLOWED = "api.github.com"
DIRECTORY = Path("/run/uv-network-policy")


def tls_request(address, name):
    context = ssl.create_default_context()
    with (
        socket.create_connection((address, 443), timeout=8) as raw,
        context.wrap_socket(raw, server_hostname=name) as stream,
    ):
        stream.sendall(
            f"HEAD / HTTP/1.1\r\nHost: {name}\r\nUser-Agent: uv-network-policy-probe\r\nConnection: close\r\n\r\n".encode()
        )
        return stream.recv(1024).startswith(b"HTTP/")


def baseline_path():
    return Path(os.environ["RUNNER_TEMP"]) / "network-policy-baseline.json"


def baseline():
    # Diagnostics only: restore communication if the experimental firewall
    # cuts off the runner. Never use this timer for acceptance evidence.
    subprocess.run(
        [
            "sudo",
            "-n",
            "systemd-run",
            "--unit=uv-network-probe-rescue",
            "--on-active=90s",
            "/bin/sh",
            "-c",
            "nft -a list table inet uv_network_policy > /run/uv-network-policy-diagnostics.txt; "
            "journalctl -u uv-network-policy.service --no-pager -n 50 >> /run/uv-network-policy-diagnostics.txt; "
            "nft delete table inet uv_network_policy",
        ],
        check=True,
    )
    addresses = sorted(
        {
            item[4][0]
            for item in socket.getaddrinfo(DENIED, 443, type=socket.SOCK_STREAM)
            if ipaddress.ip_address(item[4][0]).is_global
        }
    )
    reachable = []
    for address in addresses:
        try:
            if tls_request(address, DENIED):
                reachable.append(address)
        except OSError:
            continue
    if not any(ipaddress.ip_address(address).version == 4 for address in reachable):
        raise RuntimeError("denied-origin IPv4 baseline failed")
    baseline_path().write_text(json.dumps({"denied_addresses": reachable}) + "\n")
    print("Captured reachable denied-origin addresses before hardening.")


def early():
    if os.environ.get("UV_NETWORK_POLICY_ACTIVE") != "1":
        raise RuntimeError("policy did not run before the later pre hook")
    if not tls_request(ALLOWED, ALLOWED):
        raise RuntimeError("allowed transparent HTTPS failed in later pre hook")
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=5)
    connection.request("CONNECT", DENIED + ":443")
    if connection.getresponse().status != 403:
        raise RuntimeError("denied CONNECT succeeded in later pre hook")
    connection.close()
    print("Later action pre hook sees the active policy.")


def dns_query(name, tcp):
    packet = struct.pack("!6H", 2345, 0x100, 1, 0, 0, 0)
    packet += (
        b"".join(bytes([len(part)]) + part.encode() for part in name.split("."))
        + b"\0"
        + struct.pack("!HH", 1, 1)
    )
    with socket.socket(
        socket.AF_INET, socket.SOCK_STREAM if tcp else socket.SOCK_DGRAM
    ) as stream:
        stream.settimeout(5)
        stream.connect(("8.8.8.8", 53))
        if tcp:
            stream.sendall(struct.pack("!H", len(packet)) + packet)
            header = stream.recv(2)
            if len(header) != 2:
                raise RuntimeError("missing DNS TCP header")
            response = stream.recv(int.from_bytes(header, "big"))
        else:
            stream.sendall(packet)
            response = stream.recv(4096)
    if len(response) < 12 or response[:2] != packet[:2]:
        raise RuntimeError("invalid DNS response")
    return response[3] & 15


def main_probe():
    early()
    baseline_value = json.loads(baseline_path().read_text())
    results = {"early_hook": True, "direct": []}
    for address in baseline_value["denied_addresses"]:
        try:
            denied = not tls_request(address, DENIED)
        except OSError:
            denied = True
        results["direct"].append(
            {"family": ipaddress.ip_address(address).version, "denied": denied}
        )
        if not denied:
            raise RuntimeError("direct-IP TLS bypassed the policy")
    # The original destination does not authorize a tunnel; the proxy re-resolves SNI.
    results["allowed_sni"] = tls_request(baseline_value["denied_addresses"][0], ALLOWED)
    if not results["allowed_sni"]:
        raise RuntimeError("authorized SNI did not work through transparent routing")
    for tcp in (False, True):
        if dns_query(DENIED, tcp) != 5 or dns_query(ALLOWED, tcp) != 0:
            raise RuntimeError("external DNS bypass check failed")
    results["dns_udp_tcp"] = True
    connection = http.client.HTTPSConnection(
        "127.0.0.1", 18080, context=ssl.create_default_context(), timeout=10
    )
    connection.set_tunnel(ALLOWED, 443)
    connection.request("HEAD", "/", headers={"User-Agent": "uv-network-policy-probe"})
    results["explicit_https"] = connection.getresponse().status == 200
    connection.close()
    if not results["explicit_https"]:
        raise RuntimeError("explicit HTTPS proxy failed")
    settings = json.loads((DIRECTORY / "settings.json").read_text())
    sudo = (
        subprocess.run(
            ["sudo", "-n", "true"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )
    results["sudo_available"] = sudo
    if sudo != (settings["privileges"] == "retain"):
        raise RuntimeError("sudo access differs from the selected policy")
    if settings["privileges"] == "drop":
        for name in ("/run/docker.sock", "/run/containerd/containerd.sock"):
            with socket.socket(socket.AF_UNIX) as stream:
                if stream.connect_ex(name) == 0:
                    raise RuntimeError("privileged container socket is accessible")
        results["container_sockets_denied"] = True
    results["audit"] = json.loads((DIRECTORY / "audit/events.json").read_text())
    destination = Path(os.environ["RUNNER_TEMP"]) / "network-policy-evidence.json"
    destination.write_text(json.dumps(results, indent=2) + "\n")
    print("Network policy acceptance checks passed.")


if __name__ == "__main__":
    operation = os.environ["INPUT_OPERATION"]
    phase = sys.argv[1]
    if (operation, phase) == ("baseline", "pre"):
        baseline()
    elif (operation, phase) == ("probe", "pre"):
        early()
    elif (operation, phase) == ("probe", "main"):
        main_probe()
    elif (operation, phase) != ("baseline", "main"):
        raise SystemExit("unknown probe operation")
