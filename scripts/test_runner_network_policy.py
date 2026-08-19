"""Exercise the runner proxy on loopback without changing the host network."""

import contextlib
import http.client
import http.server
import io
import json
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

import install
import proxy
from generate_runner_network_policy import generate
from policy import Policy, hostname


def query(name, kind=1):
    encoded = b"".join(bytes([len(part)]) + part.encode() for part in name.split("."))
    return (
        struct.pack("!6H", 1234, 0x100, 1, 0, 0, 0)
        + encoded
        + b"\0"
        + struct.pack("!HH", kind, 1)
    )


def hello(name):
    outgoing = ssl.MemoryBIO()
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    stream = context.wrap_bio(ssl.MemoryBIO(), outgoing, server_hostname=name)
    with contextlib.suppress(ssl.SSLWantReadError):
        stream.do_handshake()
    return outgoing.read()


class BytesSocket:
    def __init__(self, data):
        self.data = io.BytesIO(data)

    def recv(self, size):
        return self.data.read(size)


@contextlib.contextmanager
def serving(server):
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server.server_address[1]
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


class Origin(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(
            {
                "path": self.path,
                "host": self.headers["Host"],
                "proxy_auth": self.headers.get("Proxy-Authorization"),
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        pass


class LoopbackState(proxy.State):
    """Replace only the outbound dialer, leaving real protocol parsing intact."""

    def __init__(self, port):
        super().__init__(Policy(("allowed.example",)), [], None)
        self.port = port

    def connect(self, name, port):
        self.require(name)
        return socket.create_connection(("127.0.0.1", self.port), timeout=5)


class PolicyTests(unittest.TestCase):
    def test_default_deny_and_label_boundaries(self):
        policy = Policy(("github.com", "*.example.com"), ("private.example.com",))
        self.assertTrue(policy.permits("GITHUB.COM."))
        self.assertTrue(policy.permits("sub.example.com"))
        for name in (
            "example.com",
            "badexample.com",
            "github.com.attacker.test",
            "private.example.com",
            "127.0.0.1",
            "[::1]",
            "github.com:443",
            "github.com\n",
        ):
            with self.subTest(name=name):
                self.assertFalse(policy.permits(name))

    def test_invalid_hostnames(self):
        for name in (
            "",
            None,
            "a..com",
            "-a.com",
            "a-.com",
            "a" * 64 + ".com",
            "user@example.com",
            "a/b",
            "1.2.3.4",
        ):
            with self.subTest(name=name), self.assertRaises(ValueError):
                hostname(name)

    def test_bundle_matches_reviewed_source(self):
        self.assertEqual((ACTION / "policies.json").read_text(), generate())
        document = json.loads(generate())
        self.assertIn(
            "static.rust-lang.org", document["profiles"]["rust-checks"]["allow"]
        )
        self.assertNotIn("pypi.org", document["profiles"]["rustfmt"]["allow"])

    def test_private_and_mixed_dns_answers_are_rejected(self):
        state = proxy.State(Policy(("allowed.example",)), [], None)
        for addresses in (
            ("127.0.0.1",),
            ("169.254.169.254",),
            ("93.184.216.34", "10.0.0.1"),
            ("::1",),
        ):
            answers = [
                (socket.AF_INET, socket.SOCK_STREAM, 6, "", (address, 443))
                for address in addresses
            ]
            with (
                self.subTest(addresses=addresses),
                patch.object(socket, "getaddrinfo", return_value=answers),
                self.assertRaises(PermissionError),
            ):
                state.connect("allowed.example", 443)

    def test_denied_hosts_do_not_resolve(self):
        state = proxy.State(Policy(("allowed.example",)), [], None)
        with (
            patch.object(socket, "getaddrinfo") as resolve,
            self.assertRaises(PermissionError),
        ):
            state.connect("denied.example", 443)
        resolve.assert_not_called()

    def test_dns_refusal(self):
        packet = query("denied.example")
        state = proxy.State(Policy(("allowed.example",)), [], None)
        response = state.dns(packet)
        self.assertEqual(
            struct.unpack("!6H", response[:12]), (1234, 0x8185, 1, 0, 0, 0)
        )
        self.assertEqual(response[12:], packet[12:])
        self.assertEqual(
            proxy.dns_question(query("allowed.example", 28)),
            ("allowed.example", len(query("allowed.example", 28))),
        )
        with self.assertRaises(ValueError):
            proxy.dns_question(query("allowed.example", 16))

    def test_client_hello_and_fragmentation(self):
        original = hello("allowed.example")
        self.assertEqual(
            proxy.client_hello(BytesSocket(original)), ("allowed.example", original)
        )
        payload = original[5:]
        fragmented = b"".join(
            original[:3] + struct.pack("!H", len(part)) + part
            for part in (payload[:2], payload[2:37], payload[37:])
        )
        self.assertEqual(
            proxy.client_hello(BytesSocket(fragmented)), ("allowed.example", fragmented)
        )
        for value in (b"GET / HTTP/1.1\r\n", original[:-1], b"\x16\x03\x03\xff\xff"):
            with self.subTest(value=value[:5]), self.assertRaises(ValueError):
                proxy.client_hello(BytesSocket(value))

    def test_firewall_is_default_deny_for_both_families(self):
        rules = install.firewall(
            991,
            1001,
            ["10.0.0.2", "2606:4700:4700::1111"],
            [(4, "140.82.112.3", 45678, 443)],
        )
        self.assertIn("table inet uv_network_policy", rules)
        self.assertIn("udp dport 53 redirect to :1053", rules)
        self.assertIn("tcp dport 443 redirect to :18443", rules)
        self.assertIn(
            "meta skuid 1001 ct state established ip daddr 140.82.112.3 tcp sport 45678 tcp dport 443 accept",
            rules,
        )
        self.assertNotIn("ct state established accept", rules)
        self.assertEqual(rules.count("reject with icmpx type admin-prohibited"), 2)
        self.assertIn("type filter hook forward", rules)


class HTTPTests(unittest.TestCase):
    def test_http_forwarding_and_denials(self):
        with serving(
            http.server.ThreadingHTTPServer(("127.0.0.1", 0), Origin)
        ) as origin:
            server = proxy.TCPServer(("127.0.0.1", 0), proxy.HTTPHandler)
            server.state = LoopbackState(origin)
            with serving(server) as port:
                for target, host, expected in (
                    ("http://allowed.example/a?b=1", "allowed.example", 200),
                    ("/origin-form", "allowed.example", 200),
                    ("http://denied.example/", "denied.example", 403),
                    ("http://denied.example/", "allowed.example", 502),
                    ("http://allowed.example:81/", "allowed.example:81", 502),
                ):
                    with self.subTest(target=target):
                        connection = http.client.HTTPConnection(
                            "127.0.0.1", port, timeout=5
                        )
                        connection.request(
                            "GET",
                            target,
                            headers={"Host": host, "Proxy-Authorization": "secret"},
                        )
                        response = connection.getresponse()
                        self.assertEqual(response.status, expected)
                        body = response.read()
                        if expected == 200:
                            self.assertIsNone(json.loads(body)["proxy_auth"])
                        connection.close()

    def test_connect_denied_before_tunneling(self):
        server = proxy.TCPServer(("127.0.0.1", 0), proxy.HTTPHandler)
        server.state = proxy.State(Policy(("allowed.example",)), [], None)
        with serving(server) as port:
            for target in ("denied.example:443", "allowed.example:22", "127.0.0.1:443"):
                with self.subTest(target=target):
                    connection = http.client.HTTPConnection(
                        "127.0.0.1", port, timeout=5
                    )
                    connection.request("CONNECT", target)
                    self.assertEqual(connection.getresponse().status, 403)
                    connection.close()

    def test_real_tls_through_transparent_and_connect_proxies(self):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="uv-network-policy-test-", dir=scratch
        ) as temporary:
            directory = Path(temporary)
            certificate, key = directory / "cert.pem", directory / "key.pem"
            subprocess.run(
                [
                    "openssl",
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-days",
                    "1",
                    "-subj",
                    "/CN=allowed.example",
                    "-addext",
                    "subjectAltName=DNS:allowed.example",
                    "-keyout",
                    str(key),
                    "-out",
                    str(certificate),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(certificate, key)
            origin = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Origin)
            origin.socket = context.wrap_socket(origin.socket, server_side=True)
            trust = ssl.create_default_context(cafile=certificate)
            with serving(origin) as origin_port:
                for handler in (proxy.TLSHandler, proxy.HTTPHandler):
                    server = proxy.TCPServer(("127.0.0.1", 0), handler)
                    server.state = LoopbackState(origin_port)
                    with (
                        self.subTest(handler=handler.__name__),
                        serving(server) as port,
                    ):
                        if handler is proxy.HTTPHandler:
                            connection = http.client.HTTPSConnection(
                                "127.0.0.1", port, timeout=5, context=trust
                            )
                            connection.set_tunnel("allowed.example", 443)
                        else:
                            connection = http.client.HTTPConnection(
                                "allowed.example", timeout=5
                            )
                            connection.sock = trust.wrap_socket(
                                socket.create_connection(
                                    ("127.0.0.1", port), timeout=5
                                ),
                                server_hostname="allowed.example",
                            )
                        connection.request("GET", "/encrypted")
                        response = connection.getresponse()
                        self.assertEqual(response.status, 200)
                        self.assertEqual(
                            json.loads(response.read())["path"], "/encrypted"
                        )
                        connection.close()


if __name__ == "__main__":
    unittest.main()
