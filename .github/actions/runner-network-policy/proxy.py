"""A policy-aware DNS, HTTP, and transparent TLS proxy for disposable CI VMs."""

import argparse
import http.client
import http.server
import ipaddress
import json
import selectors
import socket
import socketserver
import struct
import threading
from collections import Counter
from pathlib import Path
from urllib.parse import urlsplit

from policy import hostname, load

HTTP_PORT = 18080
TLS_PORT = 18443
DNS_PORT = 1053
MAX_HELLO = 65536
MAX_BODY = 64 * 1024 * 1024
HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


def read_exact(stream, size):
    result = bytearray()
    while len(result) < size:
        block = stream.recv(size - len(result))
        if not block:
            raise ValueError("truncated message")
        result.extend(block)
    return bytes(result)


def client_hello(stream):
    """Read a bounded, possibly fragmented ClientHello and return its SNI."""
    records = bytearray()
    handshake = bytearray()
    needed = None
    while needed is None or len(handshake) < needed:
        header = read_exact(stream, 5)
        length = int.from_bytes(header[3:5], "big")
        if header[0] != 22 or not 0 < length <= 18432:
            raise ValueError("expected TLS handshake record")
        if len(records) + length + 5 > MAX_HELLO:
            raise ValueError("ClientHello too large")
        payload = read_exact(stream, length)
        records.extend(header + payload)
        handshake.extend(payload)
        if len(handshake) >= 4:
            if handshake[0] != 1:
                raise ValueError("expected ClientHello")
            needed = 4 + int.from_bytes(handshake[1:4], "big")
            if needed > MAX_HELLO:
                raise ValueError("ClientHello too large")
    data = memoryview(handshake)[4:needed]

    def take(offset, size):
        if offset + size > len(data):
            raise ValueError("truncated ClientHello")
        return data[offset : offset + size]

    def number(offset, size):
        return int.from_bytes(take(offset, size), "big")

    offset = 34
    offset += 1 + number(offset, 1)
    offset += 2 + number(offset, 2)
    offset += 1 + number(offset, 1)
    end = offset + 2 + number(offset, 2)
    offset += 2
    if end != len(data):
        raise ValueError("invalid extension length")
    names = []
    while offset < end:
        kind = number(offset, 2)
        size = number(offset + 2, 2)
        offset += 4
        extension = bytes(take(offset, size))
        offset += size
        if kind != 0:
            continue
        if len(extension) < 2 or int.from_bytes(extension[:2], "big") != size - 2:
            raise ValueError("invalid SNI list")
        position = 2
        while position < size:
            if position + 3 > size:
                raise ValueError("invalid SNI entry")
            name_kind = extension[position]
            name_size = int.from_bytes(extension[position + 1 : position + 3], "big")
            position += 3
            if position + name_size > size or name_kind != 0:
                raise ValueError("invalid SNI name")
            names.append(
                hostname(extension[position : position + name_size].decode("ascii"))
            )
            position += name_size
    if offset != end or len(names) != 1:
        raise ValueError("one cleartext SNI name is required")
    return names[0], bytes(records)


def dns_question(packet):
    if len(packet) < 12:
        raise ValueError("truncated DNS header")
    _, flags, count, _, _, _ = struct.unpack("!6H", packet[:12])
    if flags & 0xF800 or count != 1:
        raise ValueError("unsupported DNS query")
    labels = []
    offset = 12
    while True:
        if offset >= len(packet):
            raise ValueError("truncated DNS question")
        size = packet[offset]
        offset += 1
        if size == 0:
            break
        if size > 63 or offset + size > len(packet):
            raise ValueError("invalid DNS label")
        labels.append(packet[offset : offset + size].decode("ascii"))
        offset += size
    if offset + 4 > len(packet):
        raise ValueError("truncated DNS question")
    kind, query_class = struct.unpack("!HH", packet[offset : offset + 4])
    if query_class != 1 or kind not in {1, 28, 65}:
        raise ValueError("unsupported DNS question")
    return hostname(".".join(labels)), offset + 4


def dns_refused(packet):
    if len(packet) < 12:
        return b""
    try:
        _, end = dns_question(packet)
    except (ValueError, UnicodeError):
        end = 12
    flags = 0x8085 | (int.from_bytes(packet[2:4], "big") & 0x0100)
    return (
        packet[:2] + struct.pack("!5H", flags, int(end > 12), 0, 0, 0) + packet[12:end]
    )


class State:
    def __init__(self, policy, resolvers, audit):
        self.policy = policy
        self.resolvers = resolvers
        self.audit = audit
        self.counts = Counter()
        self.lock = threading.Lock()

    def count(self, category, name=""):
        with self.lock:
            self.counts[(category, name)] += 1
            if self.audit:
                value = [
                    {"event": event, "host": host, "count": count}
                    for (event, host), count in sorted(self.counts.items())
                ]
                temporary = self.audit.with_suffix(".tmp")
                temporary.write_text(json.dumps(value, indent=2) + "\n")
                temporary.replace(self.audit)

    def require(self, name):
        name = hostname(name)
        if not self.policy.permits(name):
            self.count("denied", name)
            raise PermissionError("domain denied")
        return name

    def connect(self, name, port):
        name = self.require(name)
        if port not in {80, 443}:
            raise PermissionError("port denied")
        addresses = socket.getaddrinfo(name, port, type=socket.SOCK_STREAM)
        if not addresses or any(
            not ipaddress.ip_address(address[4][0]).is_global for address in addresses
        ):
            self.count("nonpublic_address", name)
            raise PermissionError("non-public upstream address")
        for family, kind, protocol, _, address in addresses:
            stream = socket.socket(family, kind, protocol)
            stream.settimeout(20)
            try:
                stream.connect(address)
            except OSError:
                stream.close()
                continue
            self.count("connected", name)
            return stream
        raise OSError("upstream unavailable")

    def dns(self, packet, tcp=False):
        try:
            name, _ = dns_question(packet)
            self.require(name)
            for resolver in self.resolvers:
                family = socket.AF_INET6 if ":" in resolver else socket.AF_INET
                kind = socket.SOCK_STREAM if tcp else socket.SOCK_DGRAM
                try:
                    with socket.socket(family, kind) as upstream:
                        upstream.settimeout(5)
                        upstream.connect((resolver, 53))
                        if tcp:
                            upstream.sendall(struct.pack("!H", len(packet)) + packet)
                            response = read_exact(
                                upstream, int.from_bytes(read_exact(upstream, 2), "big")
                            )
                        else:
                            upstream.sendall(packet)
                            response = upstream.recv(65535)
                    if (
                        len(response) >= 12
                        and response[:2] == packet[:2]
                        and response[2] & 0x80
                    ):
                        self.count("dns", name)
                        return response
                except OSError:
                    continue
        except (ValueError, UnicodeError, PermissionError):
            pass
        return dns_refused(packet)


def relay(client, upstream):
    with selectors.DefaultSelector() as selector:
        selector.register(client, selectors.EVENT_READ, upstream)
        selector.register(upstream, selectors.EVENT_READ, client)
        while selector.get_map():
            events = selector.select(120)
            if not events:
                return
            for key, _ in events:
                block = key.fileobj.recv(65536)
                if block:
                    key.data.sendall(block)
                else:
                    selector.unregister(key.fileobj)
                    key.data.shutdown(socket.SHUT_WR)


def tunnel(stream, state, expected=None):
    stream.settimeout(20)
    name, hello = client_hello(stream)
    if expected is not None and name != expected:
        raise PermissionError("CONNECT and SNI differ")
    with state.connect(name, 443) as upstream:
        upstream.sendall(hello)
        relay(stream, upstream)


class TLSHandler(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            tunnel(self.request, self.server.state)
        except (OSError, ValueError, UnicodeError):
            self.server.state.count("tls_error")


class DNSDatagramHandler(socketserver.BaseRequestHandler):
    def handle(self):
        packet, stream = self.request
        response = self.server.state.dns(packet)
        if response:
            stream.sendto(response, self.client_address)


class DNSStreamHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        try:
            packet = read_exact(
                self.request, int.from_bytes(read_exact(self.request, 2), "big")
            )
            response = self.server.state.dns(packet, tcp=True)
            self.request.sendall(struct.pack("!H", len(response)) + response)
        except (OSError, ValueError):
            self.server.state.count("dns_error")


def authority(value, default_port):
    parsed = urlsplit("//" + value)
    if (
        parsed.username is not None
        or parsed.password is not None
        or parsed.path
        or parsed.query
        or parsed.fragment
    ):
        raise ValueError("invalid authority")
    return hostname(parsed.hostname), parsed.port or default_port


class HTTPHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # CONNECT switches to the raw socket; buffered reads could otherwise consume TLS bytes.
    rbufsize = 0

    def log_message(self, format, *args):
        pass

    def reply(self, status):
        self.send_response(status)
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True

    def do_CONNECT(self):
        try:
            name, port = authority(self.path, 443)
            self.server.state.require(name)
            if port != 443:
                raise PermissionError("port denied")
        except (ValueError, PermissionError):
            self.reply(403)
            return
        self.send_response(200, "Connection Established")
        self.end_headers()
        self.close_connection = True
        try:
            tunnel(self.connection, self.server.state, name)
        except (OSError, ValueError, UnicodeError):
            self.server.state.count("tls_error")

    def handle_request(self):
        if self.path == "/__uv_network_proxy_health":
            self.reply(204)
            return
        connection = None
        response_started = False
        try:
            hosts = self.headers.get_all("Host", [])
            if len(hosts) != 1:
                raise ValueError("one Host header required")
            name, port = authority(hosts[0], 80)
            target = urlsplit(self.path)
            if target.scheme:
                if target.scheme != "http" or authority(target.netloc, 80) != (
                    name,
                    port,
                ):
                    raise ValueError("unexpected request target")
                path = target.path or "/"
                if target.query:
                    path += "?" + target.query
            else:
                path = self.path
            if not path.startswith("/") or path.startswith("//") or port != 80:
                raise ValueError("unexpected request target")
            lengths = self.headers.get_all("Content-Length", [])
            if self.headers.get("Transfer-Encoding") or len(lengths) > 1:
                raise ValueError("unsupported request framing")
            length = int(lengths[0]) if lengths else 0
            if not 0 <= length <= MAX_BODY:
                raise ValueError("request too large")
            # Resolve and authorize before reading a potentially large request body.
            connection = http.client.HTTPConnection(name, port, timeout=30)
            connection.sock = self.server.state.connect(name, port)
            body = self.rfile.read(length)
            if len(body) != length:
                raise ValueError("truncated body")
            hop = HOP_HEADERS | {
                item.strip().lower()
                for item in self.headers.get("Connection", "").split(",")
            }
            headers = {
                key: value
                for key, value in self.headers.items()
                if key.lower() not in hop
            }
            headers["Host"] = name
            headers["Connection"] = "close"
            connection.request(self.command, path, body=body, headers=headers)
            response = connection.getresponse()
            self.send_response(response.status)
            response_started = True
            response_hop = HOP_HEADERS | {
                item.strip().lower()
                for item in response.getheader("Connection", "").split(",")
            }
            for key, value in response.getheaders():
                if key.lower() not in response_hop:
                    self.send_header(key, value)
            self.send_header("Connection", "close")
            self.end_headers()
            if self.command != "HEAD":
                while block := response.read(65536):
                    self.wfile.write(block)
        except PermissionError:
            if not response_started:
                self.reply(403)
        except (OSError, ValueError, http.client.HTTPException):
            if not response_started:
                self.reply(502)
            self.server.state.count("http_error")
        finally:
            self.close_connection = True
            if connection:
                connection.close()

    do_GET = handle_request
    do_HEAD = handle_request
    do_POST = handle_request
    do_PUT = handle_request
    do_PATCH = handle_request
    do_DELETE = handle_request
    do_OPTIONS = handle_request


class TCPServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def handle_error(self, request, client_address):
        self.state.count("connection_error")


class UDPServer(socketserver.ThreadingUDPServer):
    allow_reuse_address = True
    daemon_threads = True

    def handle_error(self, request, client_address):
        self.state.count("connection_error")


def serve(directory):
    settings = json.loads((directory / "settings.json").read_text())
    state = State(
        load(directory / "policies.json", settings["profile"]),
        settings["resolvers"],
        directory / "audit" / "events.json",
    )
    servers = []
    for family, address in ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1")):
        for base, handler, port in (
            (TCPServer, HTTPHandler, HTTP_PORT),
            (TCPServer, TLSHandler, TLS_PORT),
            (TCPServer, DNSStreamHandler, DNS_PORT),
            (UDPServer, DNSDatagramHandler, DNS_PORT),
        ):
            server_type = type("Server", (base,), {"address_family": family})
            server = server_type((address, port), handler)
            server.state = state
            servers.append(server)
    state.count("ready")
    for server in servers[1:]:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    servers[0].serve_forever()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    serve(parser.parse_args().directory)
