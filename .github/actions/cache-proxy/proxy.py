"""Disposable, narrowly scoped GitHub Actions cache-denial proxy."""

import argparse
import http.client
import http.server
import json
import os
import socket
import ssl
import threading
from collections import Counter
from pathlib import Path
from urllib.parse import unquote, urlsplit

HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
}
MAX_BODY = 64 * 1024 * 1024


def cache_operation(method, target):
    path = unquote(urlsplit(target).path).lower()
    if "github.actions.results.api.v1.cacheservice/" in path:
        return "read" if path.endswith("/getcacheentrydownloadurl") else "write"
    if "/_apis/artifactcache/" in path:
        return "read" if method in {"GET", "HEAD"} else "write"
    return None


class PinnedConnection(http.client.HTTPSConnection):
    def __init__(self, hostname, address, port, context):
        super().__init__(hostname, port, timeout=30, context=context)
        self.address = address

    def connect(self):
        sock = socket.create_connection((self.address, self.port), self.timeout)
        self.sock = self._context.wrap_socket(sock, server_hostname=self.host)


class PinnedHTTPConnection(http.client.HTTPConnection):
    def __init__(self, hostname, address, port):
        super().__init__(hostname, port, timeout=30)
        self.address = address

    def connect(self):
        self.sock = socket.create_connection((self.address, self.port), self.timeout)


class ProxyServer(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, origins, context=None, audit_path=None):
        super().__init__(address, ProxyHandler)
        self.origins = origins
        self.upstream_context = context or ssl.create_default_context()
        self.audit_path = audit_path
        self.counts = Counter()
        self.audit_lock = threading.Lock()

    def count(self, category):
        with self.audit_lock:
            self.counts[category] += 1
            if self.audit_path:
                temporary = self.audit_path.with_suffix(".tmp")
                temporary.write_text(
                    json.dumps(dict(self.counts), sort_keys=True) + "\n"
                )
                temporary.replace(self.audit_path)

    def handle_error(self, request, client_address):
        # Never log requests, credentials, signed URLs, or exception objects.
        self.count("connection_error")


class ProxyHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        pass

    def reply(self, status, data, extra=None):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        for name, value in (extra or {}).items():
            self.send_header(name, value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)
        self.close_connection = True

    def read_body(self):
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            chunks = []
            size = 0
            while True:
                length = int(self.rfile.readline(128).split(b";", 1)[0], 16)
                if length == 0:
                    while self.rfile.readline(8192).strip():
                        pass
                    break
                size += length
                if size > MAX_BODY:
                    raise ValueError("body too large")
                chunks.append(self.rfile.read(length))
                if self.rfile.read(2) != b"\r\n":
                    raise ValueError("invalid chunk")
            return b"".join(chunks)
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 <= length <= MAX_BODY:
            raise ValueError("body too large")
        return self.rfile.read(length)

    def handle_request(self):
        authority = self.headers.get("Host", "").lower()
        host = urlsplit("//" + authority).hostname
        origin = self.server.origins.get(authority) or self.server.origins.get(host)
        if not origin or not self.path.startswith("/") or self.path.startswith("//"):
            self.reply(421, {"error": "unconfigured origin"})
            return
        if self.path == "/__uv_cache_proxy_health":
            self.reply(200, {"healthy": True})
            return
        operation = cache_operation(self.command, self.path)
        if operation:
            self.server.count(f"cache_{operation}_denied")
            self.reply(
                403,
                {
                    "code": "permission_denied",
                    "msg": f"cache {operation} denied: uv cache proxy",
                },
                {"X-UV-Cache-Proxy": "denied"},
            )
            return
        if origin.get("forward_origin"):
            authority = origin["forward_origin"]
            host = authority
            origin = self.server.origins[authority]
        try:
            body = self.read_body()
            headers = {
                name: value
                for name, value in self.headers.items()
                if name.lower() not in HOP_HEADERS
            }
            headers["Host"] = authority
            headers["Connection"] = "close"
            response = None
            for address in origin["addresses"]:
                connection = (
                    PinnedHTTPConnection(host, address, origin["port"])
                    if origin.get("scheme") == "http"
                    else PinnedConnection(
                        host,
                        address,
                        origin.get("port", 443),
                        self.server.upstream_context,
                    )
                )
                try:
                    connection.request(
                        self.command, self.path, body=body, headers=headers
                    )
                    response = connection.getresponse()
                    payload = response.read(MAX_BODY + 1)
                    if len(payload) > MAX_BODY:
                        raise ValueError("response too large")
                    status, response_headers = response.status, response.getheaders()
                    break
                except (OSError, http.client.HTTPException):
                    response = None
                finally:
                    connection.close()
            if response is None:
                raise OSError("upstream unavailable")
            category = (
                "artifact_forwarded"
                if ".ArtifactService/" in self.path
                else "other_forwarded"
            )
            self.server.count(category)
            self.send_response(status)
            for name, value in response_headers:
                if name.lower() not in HOP_HEADERS:
                    self.send_header(name, value)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Connection", "close")
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(payload)
            self.close_connection = True
        except (OSError, ValueError, http.client.HTTPException):
            self.server.count("upstream_error")
            self.reply(502, {"error": "upstream request failed"})

    do_GET = handle_request
    do_HEAD = handle_request
    do_POST = handle_request
    do_PUT = handle_request
    do_PATCH = handle_request
    do_DELETE = handle_request
    do_OPTIONS = handle_request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--uid", type=int)
    parser.add_argument("--gid", type=int)
    args = parser.parse_args()
    config = json.loads((args.directory / "origins.json").read_text())
    server = ProxyServer(
        ("127.0.0.1", 443), config, audit_path=args.directory / "audit.json"
    )
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.minimum_version = ssl.TLSVersion.TLSv1_2
    tls.set_alpn_protocols(["http/1.1"])
    tls.load_cert_chain(args.directory / "server.crt", args.directory / "server.key")
    server.socket = tls.wrap_socket(server.socket, server_side=True)
    listeners = [server]
    for port in sorted(
        {
            origin["listen_port"]
            for origin in config.values()
            if origin.get("scheme") == "http"
        }
    ):
        listener = ProxyServer(("127.0.0.1", port), config)
        # Share counters and their lock across the HTTP and HTTPS listeners.
        listener.count = server.count
        listeners.append(listener)
    if args.uid is not None and args.gid is not None:
        os.setgroups([])
        os.setgid(args.gid)
        os.setuid(args.uid)
    for listener in listeners[1:]:
        threading.Thread(target=listener.serve_forever, daemon=True).start()
    server.serve_forever()


if __name__ == "__main__":
    main()
