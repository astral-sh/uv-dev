#!/usr/bin/env -S uv run --no-cache --no-managed-python --no-python-downloads --python python3 --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Small allowlisting HTTP CONNECT proxy for Apple container sandboxes."""

from __future__ import annotations

import argparse
import fnmatch
import ipaddress
import json
import select
import socket
import socketserver
import sys
import threading
from dataclasses import dataclass
from urllib.parse import urlsplit

DEFAULT_ALLOWED_DOMAINS = [
    "pypi.org",
    "files.pythonhosted.org",
    "releases.astral.sh",
]

DEFAULT_DENIED_DOMAINS = [
    "openai.com",
    "*.openai.com",
    "oaiusercontent.com",
    "*.oaiusercontent.com",
]

DEFAULT_ALLOWED_PORTS = [80, 443]
MAX_HEADER_BYTES = 64 * 1024
BUFFER_SIZE = 64 * 1024


@dataclass(frozen=True)
class ProxyPolicy:
    allowed_domains: tuple[str, ...]
    denied_domains: tuple[str, ...]
    allowed_ports: tuple[int, ...]


class PolicyError(Exception):
    """Raised when a request is denied by policy."""


def normalize_host(host: str) -> str:
    host = host.strip().rstrip(".").lower()
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]

    try:
        return host.encode("idna").decode("ascii")
    except UnicodeError:
        return host


def split_host_port(value: str, default_port: int | None = None) -> tuple[str, int]:
    if value.startswith("["):
        host, _, rest = value[1:].partition("]")
        if rest.startswith(":"):
            return normalize_host(host), int(rest[1:])
        if default_port is not None:
            return normalize_host(host), default_port
        raise ValueError("missing port")

    if value.count(":") == 1:
        host, port = value.rsplit(":", 1)
        return normalize_host(host), int(port)

    if default_port is not None:
        return normalize_host(value), default_port

    raise ValueError("missing port")


def parse_list(value: str | None, defaults: list[str]) -> tuple[str, ...]:
    if value is None:
        items = defaults
    else:
        items = [item.strip() for item in value.replace("\n", ",").split(",")]
    return tuple(normalize_host(item) for item in items if item.strip())


def parse_ports(value: str) -> tuple[int, ...]:
    return tuple(int(item.strip()) for item in value.split(",") if item.strip())


def domain_matches(pattern: str, host: str) -> bool:
    if pattern.startswith("*."):
        suffix = pattern[1:]
        return host.endswith(suffix) and host != pattern[2:]
    return fnmatch.fnmatchcase(host, pattern)


def is_ip_literal(host: str) -> bool:
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return True


def is_forbidden_ip(address: str) -> bool:
    ip_address = ipaddress.ip_address(address)
    return (
        ip_address.is_private
        or ip_address.is_loopback
        or ip_address.is_link_local
        or ip_address.is_multicast
        or ip_address.is_reserved
        or ip_address.is_unspecified
    )


def resolve_public_addresses(host: str, port: int) -> list[tuple]:
    results = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    public_results = []
    seen = set()

    for result in results:
        address = result[4][0]
        key = (result[0], result[1], result[2], address, result[4][1])
        if key in seen:
            continue
        seen.add(key)
        if is_forbidden_ip(address):
            continue
        public_results.append(result)

    return public_results


def check_policy(policy: ProxyPolicy, host: str, port: int) -> list[tuple]:
    if port not in policy.allowed_ports:
        raise PolicyError(f"port {port} is not allowed")

    if is_ip_literal(host):
        raise PolicyError("IP literal targets are not allowed")

    for pattern in policy.denied_domains:
        if domain_matches(pattern, host):
            raise PolicyError(f"{host} is denied by domain policy")

    if not any(domain_matches(pattern, host) for pattern in policy.allowed_domains):
        raise PolicyError(f"{host} is not in the domain allowlist")

    public_results = resolve_public_addresses(host, port)
    if not public_results:
        raise PolicyError(f"{host} did not resolve to any public addresses")
    return public_results


def connect_to_first_address(addresses: list[tuple], timeout: float) -> socket.socket:
    errors = []
    for family, socktype, proto, _, sockaddr in addresses:
        upstream = socket.socket(family, socktype, proto)
        upstream.settimeout(timeout)
        try:
            upstream.connect(sockaddr)
        except OSError as error:
            errors.append(error)
            upstream.close()
            continue
        upstream.settimeout(None)
        return upstream

    if errors:
        raise errors[-1]
    raise OSError("no resolved addresses to connect")


def relay(client: socket.socket, upstream: socket.socket) -> None:
    sockets = [client, upstream]
    while True:
        readable, _, _ = select.select(sockets, [], [])
        for ready in readable:
            try:
                data = ready.recv(BUFFER_SIZE)
            except (BrokenPipeError, ConnectionResetError, OSError):
                return
            if not data:
                return
            target = upstream if ready is client else client
            try:
                target.sendall(data)
            except (BrokenPipeError, ConnectionResetError, OSError):
                return


class ProxyHandler(socketserver.BaseRequestHandler):
    server: ProxyServer

    def handle(self) -> None:
        try:
            header = self.read_header()
            if not header:
                return

            request_line, headers = self.parse_header(header)
            method, target, version = request_line.split(" ", 2)
            if method.upper() == "CONNECT":
                host, port = split_host_port(target)
                self.handle_connect(host, port, version)
            else:
                self.handle_http(method, target, version, headers, header)
        except PolicyError as error:
            self.log("DENY", str(error))
            self.send_response(403, f"Forbidden: {error}\n")
        except Exception as error:  # noqa: BLE001 - proxy should fail closed, not crash.
            self.log("ERROR", repr(error))
            self.send_response(502, f"Proxy error: {error}\n")

    def read_header(self) -> bytes:
        chunks = []
        total = 0
        while total < MAX_HEADER_BYTES:
            data = self.request.recv(4096)
            if not data:
                break
            chunks.append(data)
            total += len(data)
            if b"\r\n\r\n" in data:
                break
        return b"".join(chunks)

    def parse_header(self, header: bytes) -> tuple[str, dict[str, str]]:
        head = header.split(b"\r\n\r\n", 1)[0]
        lines = head.decode("iso-8859-1").split("\r\n")
        request_line = lines[0]
        headers = {}
        for line in lines[1:]:
            name, _, value = line.partition(":")
            if name:
                headers[name.lower()] = value.strip()
        return request_line, headers

    def handle_connect(self, host: str, port: int, version: str) -> None:
        addresses = check_policy(self.server.policy, host, port)
        self.log("ALLOW", f"CONNECT {host}:{port}")
        upstream = connect_to_first_address(addresses, self.server.connect_timeout)
        with upstream:
            self.request.sendall(
                f"{version} 200 Connection established\r\n\r\n".encode()
            )
            relay(self.request, upstream)

    def handle_http(
        self,
        method: str,
        target: str,
        version: str,
        headers: dict[str, str],
        raw_header: bytes,
    ) -> None:
        parsed = urlsplit(target)
        if parsed.scheme and parsed.hostname:
            host = normalize_host(parsed.hostname)
            port = parsed.port or (443 if parsed.scheme == "https" else 80)
            path = parsed.path or "/"
            if parsed.query:
                path += f"?{parsed.query}"
        else:
            host_header = headers.get("host")
            if not host_header:
                raise PolicyError("HTTP request is missing Host header")
            host, port = split_host_port(host_header, 80)
            path = target

        addresses = check_policy(self.server.policy, host, port)
        self.log("ALLOW", f"{method} {host}:{port}")
        upstream = connect_to_first_address(addresses, self.server.connect_timeout)
        with upstream:
            _, rest = raw_header.split(b"\r\n", 1)
            upstream.sendall(f"{method} {path} {version}\r\n".encode("ascii") + rest)
            relay(self.request, upstream)

    def send_response(self, status: int, body: str) -> None:
        reason = (
            "OK" if status == 200 else "Forbidden" if status == 403 else "Bad Gateway"
        )
        encoded = body.encode()
        response = (
            f"HTTP/1.1 {status} {reason}\r\n"
            "Connection: close\r\n"
            "Content-Type: text/plain; charset=utf-8\r\n"
            f"Content-Length: {len(encoded)}\r\n"
            "\r\n"
        ).encode() + encoded
        self.request.sendall(response)

    def log(self, action: str, message: str) -> None:
        peer = self.client_address[0]
        print(f"{action} {peer} {message}", file=sys.stderr, flush=True)


class ProxyServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(
        self,
        server_address: tuple[str, int],
        policy: ProxyPolicy,
        connect_timeout: float,
    ) -> None:
        self.policy = policy
        self.connect_timeout = connect_timeout
        super().__init__(server_address, ProxyHandler)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--host", default="0.0.0.0", help="Host interface to listen on."
    )
    parser.add_argument(
        "--port", type=int, default=0, help="Port to listen on; 0 picks a free port."
    )
    parser.add_argument(
        "--allow",
        help="Comma-separated domain allowlist. Supports exact names and *.example.com wildcards.",
    )
    parser.add_argument(
        "--deny",
        help="Comma-separated denied domains layered on top of the allowlist.",
    )
    parser.add_argument(
        "--ports", default="80,443", help="Comma-separated allowed target ports."
    )
    parser.add_argument("--connect-timeout", type=float, default=10.0)
    parser.add_argument("--ready-file", help="Write bound host/port JSON to this path.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    policy = ProxyPolicy(
        allowed_domains=parse_list(args.allow, DEFAULT_ALLOWED_DOMAINS),
        denied_domains=parse_list(args.deny, DEFAULT_DENIED_DOMAINS),
        allowed_ports=parse_ports(args.ports),
    )

    with ProxyServer((args.host, args.port), policy, args.connect_timeout) as server:
        host, port = server.server_address
        if args.ready_file:
            with open(args.ready_file, "w", encoding="utf-8") as file:
                json.dump({"host": host, "port": port}, file)
        print(
            f"egress proxy listening on {host}:{port}; "
            f"allow={','.join(policy.allowed_domains)}",
            file=sys.stderr,
            flush=True,
        )
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            thread.join()
        except KeyboardInterrupt:
            server.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
