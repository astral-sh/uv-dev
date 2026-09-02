"""Exercise the request-aware proxy using real loopback HTTP and TLS."""

from __future__ import annotations

import contextlib
import http.client
import http.server
import json
import socket
import ssl
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

import proxy
import url_proxy
from policy import Policy, validate_private_origins
from url_policy import URLPolicy

RESULTS = "results-receiver.actions.githubusercontent.com"
CACHE = "artifactcache.actions.githubusercontent.com"
HOSTS = ("allowed.example", "second.example", RESULTS, CACHE)


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
    protocol_version = "HTTP/1.1"

    def handle_request(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.server.requests.append(
            {
                "method": self.command,
                "target": self.path,
                "host": self.headers["Host"],
                "headers": {key.lower(): value for key, value in self.headers.items()},
                "body": body,
            }
        )
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "https://allowed.example/private")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if self.path == "/chunked-response":
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Set-Cookie", "first=1")
            self.send_header("Set-Cookie", "second=2")
            self.end_headers()
            self.wfile.write(b"5\r\nhello\r\n0\r\n\r\n")
            return
        if self.path in {"/upstream-200", "/upstream-302", "/upstream-403"}:
            self.send_response(int(self.path.removeprefix("/upstream-")))
            self.send_header("x-UV-uRL-PoLiCy", "denied")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        payload = json.dumps(
            {"method": self.command, "target": self.path, "host": self.headers["Host"]}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    do_GET = handle_request
    do_HEAD = handle_request
    do_POST = handle_request
    do_PUT = handle_request
    do_DELETE = handle_request

    def log_message(self, format, *args):
        pass


class LoopbackState(proxy.State):
    """Replace only outbound dialing, retaining the real policies and TLS."""

    def __init__(self, policy, cleartext_port, tls_port, trust, certificate, key):
        super().__init__(Policy(policy.hosts), [], None)
        self.url_policy = policy
        self.cleartext_port = cleartext_port
        self.tls_port = tls_port
        self.upstream_context = trust
        self.private_origins = validate_private_origins(
            {"10.2.3.4:977": CACHE, "10.2.3.4:978": RESULTS}
        )
        self.dials = []
        self.tls_context = url_proxy.make_tls_context(certificate, key, self)

    def connect(self, name, port):
        self.require(name)
        if port not in {80, 443}:
            raise PermissionError("port denied")
        self.dials.append((name, port))
        return socket.create_connection(
            ("127.0.0.1", self.tls_port if port == 443 else self.cleartext_port),
            timeout=3,
        )


class URLProxyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        cls.scratch = tempfile.TemporaryDirectory(
            prefix="uv-url-proxy-test-", dir=scratch
        )
        directory = Path(cls.scratch.name)
        cls.certificate, cls.key = directory / "certificate.pem", directory / "key.pem"
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
                "subjectAltName=" + ",".join(f"DNS:{name}" for name in HOSTS),
                "-keyout",
                str(cls.key),
                "-out",
                str(cls.certificate),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        cls.trust = ssl.create_default_context(cafile=cls.certificate)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cls.certificate, cls.key)
        cls.cleartext_origin = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Origin)
        cls.tls_origin = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Origin)
        cls.tls_origin.socket = context.wrap_socket(
            cls.tls_origin.socket, server_side=True
        )
        for server in (cls.cleartext_origin, cls.tls_origin):
            server.requests = []
            threading.Thread(target=server.serve_forever, daemon=True).start()
        cls.policy = URLPolicy.from_dict(
            {
                "rules": [
                    {
                        "url": f"{scheme}://allowed.example/exact?x=1",
                        "methods": ["GET", "HEAD"],
                    }
                    for scheme in ("http", "https")
                ]
                + [
                    {"url": f"{scheme}://allowed.example/echo", "methods": ["POST"]}
                    for scheme in ("http", "https")
                ]
                + [
                    {
                        "url": f"{scheme}://allowed.example/upstream-{status}",
                        "methods": ["GET"],
                    }
                    for scheme in ("http", "https")
                    for status in (200, 302, 403)
                ]
                + [
                    {"url": "http://allowed.example/bare?", "methods": ["GET"]},
                    {
                        "url": "http://allowed.example/query/",
                        "methods": ["GET"],
                        "match": "prefix",
                        "query": "any",
                    },
                    {"url": "https://allowed.example/redirect", "methods": ["GET"]},
                    {
                        "url": "https://allowed.example/chunked-response",
                        "methods": ["GET"],
                    },
                    {"url": "https://second.example/exact?x=1", "methods": ["GET"]},
                    {"url": f"https://{RESULTS}/artifact", "methods": ["GET"]},
                    {"url": f"https://{CACHE}/artifact", "methods": ["GET"]},
                ]
            }
        )

    @classmethod
    def tearDownClass(cls):
        for server in (cls.cleartext_origin, cls.tls_origin):
            server.shutdown()
            server.server_close()
        cls.scratch.cleanup()

    def setUp(self):
        self.cleartext_origin.requests.clear()
        self.tls_origin.requests.clear()
        self.state = LoopbackState(
            self.policy,
            self.cleartext_origin.server_port,
            self.tls_origin.server_port,
            self.trust,
            self.certificate,
            self.key,
        )

    @contextlib.contextmanager
    def proxy(self, handler=url_proxy.HTTPHandler):
        server = proxy.TCPServer(("127.0.0.1", 0), handler)
        server.state = self.state
        with serving(server) as port:
            yield port

    def response(self, connection, method, target, *, headers=None, body=None):
        try:
            connection.request(method, target, headers=headers or {}, body=body)
            response = connection.getresponse()
            return response.status, response.getheaders(), response.read()
        finally:
            connection.close()

    def clear_request(
        self,
        port,
        target,
        *,
        method="GET",
        host="allowed.example",
        headers=None,
        body=None,
    ):
        return self.response(
            http.client.HTTPConnection("127.0.0.1", port, timeout=3),
            method,
            target,
            headers={"Host": host, **(headers or {})},
            body=body,
        )

    def tls_connection(self, port, *, connect=False, name="allowed.example"):
        if connect:
            connection = http.client.HTTPSConnection(
                "127.0.0.1", port, timeout=3, context=self.trust
            )
            connection.set_tunnel(name, 443)
            return connection
        connection = http.client.HTTPConnection(name, 443, timeout=3)
        connection.sock = self.trust.wrap_socket(
            socket.create_connection(("127.0.0.1", port), timeout=3),
            server_hostname=name,
        )
        return connection

    def raw_response(self, port, data, *, tls=False, half_close=False):
        stream = socket.create_connection(("127.0.0.1", port), timeout=3)
        if tls:
            stream = self.trust.wrap_socket(stream, server_hostname="allowed.example")
        with stream:
            stream.sendall(data)
            if half_close:
                stream.shutdown(socket.SHUT_WR)
            response = http.client.HTTPResponse(stream)
            response.begin()
            result = response.status, response.getheaders(), response.read()
            response.close()
            return result

    def test_cleartext_url_method_and_query_policy(self):
        with self.proxy() as port:
            cases = (
                ("GET", "/exact?x=1", "allowed.example", 200),
                ("GET", "http://allowed.example:80/exact?x=1", "ALLOWED.EXAMPLE", 200),
                ("HEAD", "/exact?x=1", "allowed.example", 200),
                ("POST", "/exact?x=1", "allowed.example", 403),
                ("GET", "/exact?x=2", "allowed.example", 403),
                ("GET", "/exact", "allowed.example", 403),
                ("GET", "/exact?", "allowed.example", 403),
                ("GET", "/exact/child?x=1", "allowed.example", 403),
                ("GET", "/bare?", "allowed.example", 200),
                ("GET", "/bare", "allowed.example", 403),
                ("GET", "/query/item?sig=a%2Fb%3D", "allowed.example", 200),
                ("GET", "/query/item?sig=%0d%0a", "allowed.example", 403),
                ("GET", "/exact?x=1", "denied.example", 403),
                ("GET", "http://second.example/exact?x=1", "allowed.example", 400),
                ("GET", "/exact?x=1", "allowed.example:81", 403),
            )
            for method, target, host, status in cases:
                with self.subTest(method=method, target=target, host=host):
                    before = len(self.state.dials)
                    result = self.clear_request(port, target, method=method, host=host)
                    self.assertEqual(result[0], status)
                    self.assertEqual(len(self.state.dials) - before, int(status == 200))

    def test_transparent_tls_and_connect_enforce_request_policy(self):
        for handler in (url_proxy.TLSHandler, url_proxy.HTTPHandler):
            with self.subTest(handler=handler.__name__), self.proxy(handler) as port:
                for method, target, expected in (
                    ("GET", "/exact?x=1", 200),
                    ("POST", "/exact?x=1", 403),
                    ("GET", "/exact?x=2", 403),
                    ("GET", "/private", 403),
                ):
                    with self.subTest(method=method, target=target):
                        before = len(self.state.dials)
                        result = self.response(
                            self.tls_connection(
                                port, connect=handler is url_proxy.HTTPHandler
                            ),
                            method,
                            target,
                        )
                        self.assertEqual(result[0], expected)
                        self.assertEqual(
                            len(self.state.dials) - before, int(expected == 200)
                        )

    def test_denied_bodies_preserve_the_policy_response(self):
        cache_target = (
            "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL"
        )
        for handler in (url_proxy.TLSHandler, url_proxy.HTTPHandler):
            with self.subTest(handler=handler.__name__), self.proxy(handler) as port:
                for name, target, size in (
                    ("allowed.example", "/private", 1024 * 1024),
                    (RESULTS, cache_target, 128),
                    (RESULTS, cache_target, 1024 * 1024),
                ):
                    with self.subTest(host=name, target=target, size=size):
                        status, headers, body = self.response(
                            self.tls_connection(
                                port,
                                connect=handler is url_proxy.HTTPHandler,
                                name=name,
                            ),
                            "POST",
                            target,
                            body=b"x" * size,
                        )
                        self.assertEqual((status, body), (403, b""))
                        self.assertIn(("X-UV-URL-Policy", "denied"), headers)
        with self.proxy() as port:
            for host, target in (
                ("allowed.example", "/private"),
                ("10.2.3.4:978", cache_target),
            ):
                with self.subTest(cleartext_host=host):
                    status, headers, body = self.clear_request(
                        port,
                        target,
                        method="POST",
                        host=host,
                        body=b"x" * (1024 * 1024),
                    )
                    self.assertEqual((status, body), (403, b""))
                    self.assertIn(("X-UV-URL-Policy", "denied"), headers)
        self.assertEqual(self.state.dials, [])
        self.assertEqual(self.cleartext_origin.requests, [])
        self.assertEqual(self.tls_origin.requests, [])

    def test_denied_expect_continue_is_drained_without_forwarding(self):
        body = b"synthetic denied body" * 4096
        for handler in (url_proxy.TLSHandler, url_proxy.HTTPHandler):
            with self.subTest(handler=handler.__name__), self.proxy(handler) as port:
                connection = self.tls_connection(
                    port, connect=handler is url_proxy.HTTPHandler
                )
                try:
                    if connection.sock is None:
                        connection.connect()
                    stream = connection.sock
                    stream.sendall(
                        (
                            "POST /private HTTP/1.1\r\nHost: allowed.example\r\n"
                            f"Content-Length: {len(body)}\r\n"
                            "Expect: 100-continue\r\n\r\n"
                        ).encode()
                    )
                    with stream.makefile("rb", buffering=0) as incoming:
                        self.assertEqual(
                            incoming.readline(), b"HTTP/1.1 100 Continue\r\n"
                        )
                        self.assertEqual(incoming.readline(), b"\r\n")
                    stream.sendall(body)
                    response = http.client.HTTPResponse(stream)
                    response.begin()
                    self.assertEqual(
                        (response.status, response.getheader("X-UV-URL-Policy")),
                        (403, "denied"),
                    )
                    self.assertEqual(response.read(), b"")
                    response.close()
                finally:
                    connection.close()
        self.assertEqual(self.state.dials, [])
        self.assertEqual(self.cleartext_origin.requests, [])
        self.assertEqual(self.tls_origin.requests, [])

    def test_denied_body_size_timeout_and_framing_limits_remain(self):
        with self.proxy() as port:
            for framing, expected in (
                (b"Content-Length: 67108865\r\n", 413),
                (b"Transfer-Encoding: chunked\r\n", 400),
            ):
                with self.subTest(framing=framing):
                    self.assertEqual(
                        self.raw_response(
                            port,
                            b"POST /private HTTP/1.1\r\nHost: allowed.example\r\n"
                            + framing
                            + b"\r\n",
                        )[0],
                        expected,
                    )
            self.assertEqual(
                self.raw_response(
                    port,
                    b"POST /private HTTP/1.1\r\nHost: allowed.example\r\n"
                    b"Content-Length: 2\r\n\r\nx",
                    half_close=True,
                )[0],
                400,
            )
        with patch.object(url_proxy, "IO_TIMEOUT", 1):
            for handler in (url_proxy.TLSHandler, url_proxy.HTTPHandler):
                with (
                    self.subTest(handler=handler.__name__),
                    self.proxy(handler) as port,
                ):
                    connection = self.tls_connection(
                        port, connect=handler is url_proxy.HTTPHandler
                    )
                    try:
                        if connection.sock is None:
                            connection.connect()
                        connection.sock.sendall(
                            b"POST /private HTTP/1.1\r\nHost: allowed.example\r\n"
                            b"Content-Length: 16\r\n\r\nx"
                        )
                        response = http.client.HTTPResponse(connection.sock)
                        response.begin()
                        self.assertEqual(response.status, 408)
                        self.assertEqual(response.read(), b"")
                        response.close()
                    finally:
                        connection.close()
        self.assertEqual(self.state.dials, [])
        self.assertEqual(self.cleartext_origin.requests, [])
        self.assertEqual(self.tls_origin.requests, [])

    def test_connect_authority_sni_and_host_must_agree(self):
        with self.proxy() as port:
            for target, host, expected in (
                ("denied.example:443", "denied.example:443", 403),
                ("allowed.example:22", "allowed.example:22", 403),
                ("127.0.0.1:443", "127.0.0.1:443", 400),
                ("allowed.example:443", "second.example:443", 403),
            ):
                with self.subTest(target=target, host=host):
                    self.assertEqual(
                        self.raw_response(
                            port,
                            f"CONNECT {target} HTTP/1.1\r\nHost: {host}\r\n\r\n".encode(),
                        )[0],
                        expected,
                    )
            with socket.create_connection(("127.0.0.1", port), timeout=3) as stream:
                stream.sendall(
                    b"CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example:443\r\n\r\n"
                )
                response = http.client.HTTPResponse(stream)
                response.begin()
                self.assertEqual(response.status, 200)
                response.close()
                with self.assertRaises(ssl.SSLError):
                    self.trust.wrap_socket(stream, server_hostname="second.example")
        with self.proxy(url_proxy.TLSHandler) as port:
            for name in ("denied.example", None):
                with self.subTest(sni=name):
                    context = ssl._create_unverified_context()
                    with (
                        socket.create_connection(
                            ("127.0.0.1", port), timeout=3
                        ) as stream,
                        self.assertRaises(ssl.SSLError),
                    ):
                        context.wrap_socket(stream, server_hostname=name)
            for target, host, expected in (
                ("/exact?x=1", "second.example", 403),
                ("/exact?x=1", "allowed.example:80", 403),
                ("https://allowed.example/exact?x=1", "allowed.example", 400),
            ):
                with self.subTest(target=target, host=host):
                    self.assertEqual(
                        self.response(
                            self.tls_connection(port),
                            "GET",
                            target,
                            headers={"Host": host},
                        )[0],
                        expected,
                    )
            self.assertEqual(
                self.raw_response(
                    port,
                    b"CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example:443\r\n\r\n",
                    tls=True,
                )[0],
                403,
            )
        self.assertEqual(self.state.dials, [])

    def test_private_http_endpoints_are_translated_and_filtered(self):
        with self.proxy() as port:
            for number, expected in ((977, CACHE), (978, RESULTS)):
                private = f"10.2.3.4:{number}"
                for absolute in (False, True):
                    with self.subTest(private=private, absolute=absolute):
                        target = (
                            f"http://{private}/artifact" if absolute else "/artifact"
                        )
                        result = self.clear_request(port, target, host=private)
                        self.assertEqual(result[0], 200)
                        self.assertEqual(json.loads(result[2])["host"], expected)
                        self.assertEqual(self.state.dials[-1], (expected, 443))
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
                connection.set_tunnel("10.2.3.4", number)
                self.assertEqual(
                    self.response(
                        connection, "GET", "/artifact", headers={"Host": private}
                    )[0],
                    200,
                )
                before = len(self.state.dials)
                self.assertEqual(
                    self.clear_request(
                        port,
                        "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL",
                        method="POST",
                        host=private,
                    )[0],
                    403,
                )
                self.assertEqual(len(self.state.dials), before)
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
            connection.set_tunnel("10.2.3.4", 978)
            before = len(self.state.dials)
            self.assertEqual(
                self.response(
                    connection, "GET", "/artifact", headers={"Host": "10.2.3.4:977"}
                )[0],
                403,
            )
            self.assertEqual(len(self.state.dials), before)

    def test_ambiguous_request_targets_and_headers_never_dial(self):
        with self.proxy() as port:
            for target in (
                "/query//item",
                "/query/./item",
                "/query/sub/../item",
                "/query/%2e/item",
                "/query/%2Fitem",
                "/query/%252e/item",
                "/query/item?bad=%",
                "/query/item?bad=%7f",
            ):
                with self.subTest(target=target):
                    self.assertEqual(self.clear_request(port, target)[0], 403)
            for fields in (
                b"Host: allowed.example\r\nhost: second.example\r\n",
                b"Host: allowed.example\r\nX-Test: first\r\nx-test: second\r\n",
                b"H_ost: allowed.example\r\n",
                b"Host: allowed.example\r\nHost_: second.example\r\n",
                b"Host: allowed.example\r\nContent_Length: 0\r\n",
                b"Host: allowed.example\r\nTransfer_Encoding: chunked\r\n",
                b"Host: allowed.example\r\nX_HTTP_Method_Override: DELETE\r\n",
                b"Host: allowed.example\r\nX_Forwarded_Host: second.example\r\n",
                b"Host: allowed.example\r\nX_Custom: value\r\n",
                b"Host: allowed.example\r\nX-Test: first\r\n Transfer-Encoding: chunked\r\n",
                b"Host: allowed.example\r\nContent-Length: 0\r\nContent-Length: 0\r\n",
                b"Host: allowed.example\r\nTransfer-Encoding: chunked\r\n",
                b"Host: allowed.example\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n",
                b"Host: allowed.example\r\nContent-Length: -1\r\n",
                b"Host: allowed.example\r\nContent-Length: +1\r\n",
                b"Host: allowed.example\r\nBad Name: value\r\n",
                b"Host: allowed.example\r\nUpgrade: websocket\r\n",
                b"Host: allowed.example\r\nConnection: upgrade\r\n",
                b"Host: allowed.example\r\nX-Test: value\x00suffix\r\n",
                b"X-Test: no-host\r\n",
            ):
                with self.subTest(fields=fields):
                    data = b"GET /exact?x=1 HTTP/1.1\r\n" + fields + b"\r\n"
                    self.assertEqual(self.raw_response(port, data)[0], 400)
            self.assertEqual(
                self.raw_response(
                    port,
                    b"POST /echo HTTP/1.1\r\nHost: allowed.example\r\nContent-Length: 2\r\n\r\nx",
                    half_close=True,
                )[0],
                400,
            )
            self.assertEqual(
                self.raw_response(
                    port, b"GET  /exact?x=1 HTTP/1.1\r\nHost: allowed.example\r\n\r\n"
                )[0],
                400,
            )
            self.assertEqual(
                self.raw_response(
                    port,
                    b"POST /echo HTTP/1.1\r\nHost: allowed.example\r\nContent-Length: 67108865\r\n\r\n",
                )[0],
                413,
            )
            self.assertEqual(self.state.dials, [])

    def test_request_body_and_hop_headers_are_reframed(self):
        with self.proxy() as port:
            result = self.clear_request(
                port,
                "/echo",
                method="POST",
                body=b"synthetic-body",
                headers={
                    "Authorization": "Bearer synthetic",
                    "Proxy-Authorization": "drop",
                    "Connection": "X-Remove",
                    "X-Remove": "drop",
                },
            )
        self.assertEqual(result[0], 200)
        observed = self.cleartext_origin.requests[-1]
        self.assertEqual(observed["body"], b"synthetic-body")
        self.assertEqual(observed["headers"]["authorization"], "Bearer synthetic")
        self.assertNotIn("proxy-authorization", observed["headers"])
        self.assertNotIn("x-remove", observed["headers"])
        self.assertEqual(observed["headers"]["content-length"], "14")

    def test_routing_and_method_override_headers_are_rejected(self):
        with self.proxy() as port:
            names = url_proxy.ROUTING_HEADERS | {
                name.replace("-", "_") for name in url_proxy.ROUTING_HEADERS
            }
            for name in sorted(names):
                with self.subTest(header=name):
                    self.assertEqual(
                        self.clear_request(
                            port,
                            "/exact?x=1",
                            headers={name: "/private"},
                        )[0],
                        400,
                    )
            status, headers, _ = self.clear_request(port, "/private")
            self.assertEqual(status, 403)
            self.assertIn(("X-UV-URL-Policy", "denied"), headers)
            status, headers, _ = self.raw_response(
                port,
                b"CONNECT denied.example:443 HTTP/1.1\r\nHost: denied.example:443\r\n\r\n",
            )
            self.assertEqual(status, 403)
            self.assertIn(("X-UV-URL-Policy", "denied"), headers)
        self.assertEqual(self.state.dials, [])

    def test_redirects_are_not_followed_and_chunked_responses_are_reframed(self):
        with self.proxy(url_proxy.TLSHandler) as port:
            status, headers, body = self.response(
                self.tls_connection(port), "GET", "/redirect"
            )
            self.assertEqual((status, body), (302, b""))
            self.assertIn(("Location", "https://allowed.example/private"), headers)
            self.assertEqual(len(self.state.dials), 1)
            self.assertEqual(
                self.response(self.tls_connection(port), "GET", "/private")[0], 403
            )
            self.assertEqual(len(self.state.dials), 1)
            status, headers, body = self.response(
                self.tls_connection(port), "GET", "/chunked-response"
            )
            self.assertEqual((status, body), (200, b"hello"))
            self.assertNotIn("transfer-encoding", {key.lower() for key, _ in headers})
            self.assertEqual(
                [value for key, value in headers if key.lower() == "set-cookie"],
                ["first=1", "second=2"],
            )

    def test_upstream_cannot_spoof_policy_denial(self):
        for handler in (url_proxy.HTTPHandler, url_proxy.TLSHandler):
            with self.subTest(handler=handler.__name__), self.proxy(handler) as port:
                for expected in (200, 302, 403):
                    with self.subTest(status=expected):
                        before = len(self.state.dials)
                        target = f"/upstream-{expected}"
                        if handler is url_proxy.HTTPHandler:
                            status, headers, body = self.clear_request(port, target)
                        else:
                            status, headers, body = self.response(
                                self.tls_connection(port), "GET", target
                            )
                        self.assertEqual((status, body), (expected, b""))
                        self.assertNotIn(
                            "x-uv-url-policy", {key.lower() for key, _ in headers}
                        )
                        self.assertEqual(len(self.state.dials), before + 1)

    def test_upstream_certificate_is_verified(self):
        self.state.upstream_context = ssl.create_default_context()
        with self.proxy(url_proxy.TLSHandler) as port:
            self.assertEqual(
                self.response(self.tls_connection(port), "GET", "/exact?x=1")[0], 502
            )
        self.assertEqual(self.tls_origin.requests, [])

    def test_slow_client_hello_does_not_block_other_connections(self):
        with (
            self.proxy(url_proxy.TLSHandler) as port,
            socket.create_connection(("127.0.0.1", port), timeout=3),
        ):
            self.assertEqual(
                self.response(self.tls_connection(port), "GET", "/exact?x=1")[0],
                200,
            )
        with (
            self.proxy() as port,
            socket.create_connection(("127.0.0.1", port), timeout=3) as slow,
        ):
            slow.sendall(
                b"CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example:443\r\n\r\n"
            )
            response = http.client.HTTPResponse(slow)
            response.begin()
            self.assertEqual(response.status, 200)
            response.close()
            self.assertEqual(
                self.response(
                    self.tls_connection(port, connect=True), "GET", "/exact?x=1"
                )[0],
                200,
            )

    def test_health_is_only_a_loopback_cleartext_endpoint(self):
        with self.proxy() as port:
            self.assertEqual(
                self.clear_request(
                    port, "/__uv_network_proxy_health", host=f"127.0.0.1:{port}"
                )[0],
                204,
            )
            self.assertEqual(
                self.clear_request(port, "/__uv_network_proxy_health")[0], 403
            )
        with self.proxy(url_proxy.TLSHandler) as port:
            self.assertEqual(
                self.response(
                    self.tls_connection(port), "GET", "/__uv_network_proxy_health"
                )[0],
                403,
            )
        self.assertEqual(self.state.dials, [])


if __name__ == "__main__":
    unittest.main()
