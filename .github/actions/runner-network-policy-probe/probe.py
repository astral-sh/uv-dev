"""Harmless hosted acceptance checks. Never prints response bodies or tokens."""

import base64
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
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

DENIED = "example.com"
ALLOWED = "api.github.com"
DIRECTORY = Path("/run/uv-network-policy")
IMAGE = "ubuntu@sha256:561618e2c15bf2397621dd04f96926663a3b5616c189cf7e38db7e82f5c538ea"


def ipv4(name, port):
    return socket.getaddrinfo(
        name, port, family=socket.AF_INET, type=socket.SOCK_STREAM
    )[0][4][0]


def connects(address, port):
    try:
        with socket.create_connection((address, port), timeout=5):
            return True
    except OSError:
        return False


def container_connects(address):
    return (
        subprocess.run(
            [
                "docker",
                "run",
                "--rm",
                "--network",
                "bridge",
                IMAGE,
                "timeout",
                "5",
                "bash",
                "-c",
                'exec 3<>"/dev/tcp/$1/443"',
                "probe",
                address,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )


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
    value = {"denied_addresses": reachable}
    ssh_address = ipv4("ssh.github.com", 443)
    with socket.create_connection((ssh_address, 443), timeout=5) as stream:
        value["ssh_on_https"] = stream.recv(128).startswith(b"SSH-")
    if not value["ssh_on_https"]:
        raise RuntimeError("non-TLS baseline failed")
    value["ssh_address"] = ssh_address
    alternate_address = ipv4("github.com", 22)
    value["alternate_address"] = alternate_address
    value["alternate_port_reachable"] = connects(alternate_address, 22)
    if os.environ.get("INPUT_CONTAINERS") == "true":
        subprocess.run(["docker", "pull", IMAGE], check=True, stdout=subprocess.DEVNULL)
        value["container_address"] = ipv4(ALLOWED, 443)
        if not container_connects(value["container_address"]):
            raise RuntimeError("container network baseline failed")
    baseline_path().write_text(json.dumps(value) + "\n")
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
    try:
        with socket.create_connection(
            (baseline_value["ssh_address"], 443), timeout=3
        ) as stream:
            stream.sendall(b"SSH-2.0-uv-network-policy-probe\r\n")
            raw_ssh = stream.recv(128).startswith(b"SSH-")
    except OSError:
        raw_ssh = False
    results["non_tls_denied"] = not raw_ssh
    if raw_ssh:
        raise RuntimeError("non-TLS traffic bypassed HTTPS policy")
    results["alternate_port_baseline"] = baseline_value["alternate_port_reachable"]
    if baseline_value["alternate_port_reachable"]:
        results["alternate_port_denied"] = not connects(
            baseline_value["alternate_address"], 22
        )
        if not results["alternate_port_denied"]:
            raise RuntimeError("alternate TCP port bypassed policy")
    if "container_address" in baseline_value:
        results["container_forwarding_denied"] = not container_connects(
            baseline_value["container_address"]
        )
        if not results["container_forwarding_denied"]:
            raise RuntimeError("container forwarding bypassed policy")
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
    if os.environ.get("INPUT_OIDC") == "true":
        url = urlsplit(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"])
        if (
            url.scheme != "https"
            or not url.hostname.endswith(".actions.githubusercontent.com")
            or url.port not in {None, 443}
        ):
            raise RuntimeError("unexpected OIDC endpoint")
        audience = "urn:uv-network-policy-probe"
        query = urlencode([*parse_qsl(url.query), ("audience", audience)])
        connection = http.client.HTTPSConnection(url.hostname, timeout=10)
        connection.request(
            "GET",
            urlunsplit(("", "", url.path, query, "")),
            headers={
                "Authorization": "Bearer "
                + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
            },
        )
        response = connection.getresponse()
        payload = json.loads(response.read(1024 * 1024))["value"].split(".")[1]
        claims = json.loads(
            base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4))
        )
        results["oidc"] = (
            response.status == 200
            and claims["aud"] == audience
            and claims["repository"] == "astral-sh/uv-dev"
            and str(claims["run_id"]) == os.environ["GITHUB_RUN_ID"]
        )
        connection.close()
        if not results["oidc"]:
            raise RuntimeError("OIDC claims did not match this probe")
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
