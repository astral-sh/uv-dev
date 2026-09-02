"""Terminate HTTP/TLS and enforce an exact request policy before every dial."""

from __future__ import annotations

import http.client
import re
import socketserver
import ssl
import time
from dataclasses import dataclass
from http import HTTPStatus

from policy import hostname

IO_TIMEOUT = 20
MAX_REQUEST_LINE = 8192
MAX_HEADER_LINE = 8192
MAX_HEADER_BYTES = 65536
MAX_HEADERS = 100
MAX_BODY = 64 * 1024 * 1024
TOKEN = re.compile(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+")
METHODS = {"GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"}
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
    "content-length",
}
ROUTING_HEADERS = {
    "forwarded",
    "x-original-url",
    "x-rewrite-url",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-forwarded-port",
    "x-forwarded-prefix",
    "x-forwarded-uri",
    "x-forwarded-path",
    "x-original-host",
    "x-original-uri",
    "x-envoy-original-path",
    "x-http-method-override",
    "x-method-override",
    "x-http-method",
    "x-original-method",
    "x-forwarded-method",
}


class RequestError(Exception):
    def __init__(self, status=400):
        self.status = status


@dataclass(frozen=True)
class Request:
    method: str
    target: str
    version: str
    headers: dict[str, tuple[str, str]]
    length: int
    expect_continue: bool


def deadline_timeout(stream, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise RequestError(408)
    stream.settimeout(remaining)


def read_line(stream, limit, deadline):
    # Do not buffer beyond the line: a CONNECT client can send its ClientHello
    # in the same packet as the HTTP headers, before TLS owns the socket.
    line = bytearray()
    while len(line) <= limit:
        deadline_timeout(stream, deadline)
        try:
            value = stream.recv(1)
        except TimeoutError:
            raise RequestError(408) from None
        if not value:
            raise RequestError()
        line.extend(value)
        if value == b"\n":
            if not line.endswith(b"\r\n") or len(line) > limit:
                raise RequestError()
            return bytes(line[:-2])
    raise RequestError(431)


def header_value(value):
    if any(character != 9 and not 32 <= character <= 126 for character in value):
        raise RequestError()
    return value.decode("ascii").strip(" \t")


def connection_tokens(value):
    if not value:
        return set()
    tokens = [item.strip().lower() for item in value.split(",")]
    if any(not TOKEN.fullmatch(item) for item in tokens):
        raise RequestError()
    return set(tokens)


def read_request(stream):
    deadline = time.monotonic() + IO_TIMEOUT
    line = read_line(stream, MAX_REQUEST_LINE, deadline)
    parts = line.split(b" ")
    if len(parts) != 3 or any(not part for part in parts):
        raise RequestError()
    try:
        method, target, version = (part.decode("ascii") for part in parts)
    except UnicodeError:
        raise RequestError() from None
    if (
        not TOKEN.fullmatch(method)
        or method != method.upper()
        or version not in {"HTTP/1.0", "HTTP/1.1"}
        or any(not 33 <= ord(character) <= 126 for character in target)
        or "#" in target
        or "\\" in target
    ):
        raise RequestError()
    headers = {}
    size = len(line) + 2
    while True:
        line = read_line(stream, MAX_HEADER_LINE, deadline)
        size += len(line) + 2
        if size > MAX_HEADER_BYTES:
            raise RequestError(431)
        if not line:
            break
        if line[:1] in (b" ", b"\t") or len(headers) >= MAX_HEADERS:
            raise RequestError()
        key, separator, value = line.partition(b":")
        try:
            name = key.decode("ascii")
        except UnicodeError:
            raise RequestError() from None
        lowered = name.lower()
        # CGI/WSGI-style origins can collapse hyphens and underscores into the
        # same header name. Do not admit alternate framing or routing spellings.
        if (
            not separator
            or not TOKEN.fullmatch(name)
            or "_" in name
            or lowered in headers
        ):
            raise RequestError()
        headers[lowered] = (name, header_value(value))
    if "transfer-encoding" in headers or "trailer" in headers:
        raise RequestError()
    if ROUTING_HEADERS.intersection(headers):
        raise RequestError()
    connection = connection_tokens(headers.get("connection", ("", ""))[1])
    if "upgrade" in headers or "upgrade" in connection:
        raise RequestError()
    value = headers.get("content-length", ("", "0"))[1]
    if not value.isascii() or not value.isdecimal():
        raise RequestError()
    if len(value) > 20:
        raise RequestError(413)
    length = int(value)
    if length > MAX_BODY:
        raise RequestError(413)
    expect = headers.get("expect", ("", ""))[1].lower()
    if expect not in {"", "100-continue"}:
        raise RequestError(417)
    return Request(method, target, version, headers, length, bool(expect))


def body_chunks(stream, length):
    if not 0 <= length <= MAX_BODY:
        raise RequestError(413)
    deadline = time.monotonic() + IO_TIMEOUT
    remaining = length
    while remaining:
        deadline_timeout(stream, deadline)
        try:
            block = stream.recv(min(65536, remaining))
        except TimeoutError:
            raise RequestError(408) from None
        if not block:
            raise RequestError()
        remaining -= len(block)
        yield block


def read_body(stream, length):
    body = bytearray()
    for block in body_chunks(stream, length):
        body.extend(block)
    return bytes(body)


def deny_request(stream, request):
    # Closing TLS with unread request data can reset the connection before the
    # client receives our denial. Consume only the validated, bounded body; it
    # is never retained, authorized, or passed to an upstream connection.
    if request.expect_continue:
        stream.sendall(b"HTTP/1.1 100 Continue\r\n\r\n")
    for _ in body_chunks(stream, request.length):
        pass
    send_reply(stream, 403)


def authority(value, default_port):
    if (
        not value
        or value.count(":") > 1
        or any(character in value for character in "@/?#\\[] \t")
    ):
        raise RequestError()
    raw_name, separator, raw_port = value.partition(":")
    try:
        name = hostname(raw_name)
    except ValueError:
        raise RequestError() from None
    if raw_name.lower() != name:
        raise RequestError()
    if not separator:
        return name, default_port
    if (
        not raw_port.isascii()
        or not raw_port.isdecimal()
        or not 1 <= int(raw_port) <= 65535
        or str(int(raw_port)) != raw_port
    ):
        raise RequestError()
    return name, int(raw_port)


def request_host(request):
    if "host" not in request.headers:
        raise RequestError()
    return request.headers["host"][1]


def origin_target(target):
    if not target.startswith("/"):
        raise RequestError()
    # An exact reviewed rule may opt in to a repeated slash. Keep the raw path
    # for that decision; never resolve it as a network-relative URL.
    return target


def cleartext_target(request, state, private_authority=None):
    requested = request_host(request)
    if private_authority is not None and requested != private_authority:
        raise PermissionError()
    target = request.target
    if target.startswith("http://"):
        remainder = target.removeprefix("http://")
        boundary = min(
            (position for marker in "/?" if (position := remainder.find(marker)) >= 0),
            default=len(remainder),
        )
        target_authority, target = remainder[:boundary], remainder[boundary:]
        if (
            requested in state.private_origins
            or target_authority in state.private_origins
        ):
            matches = target_authority == requested
        else:
            matches = authority(target_authority, 80) == authority(requested, 80)
        if not matches:
            raise RequestError()
        if not target or target.startswith("?"):
            target = "/" + target
    target = origin_target(target)
    if requested in state.private_origins:
        return "https", state.private_origins[requested], 443, target
    name, port = authority(requested, 80)
    if port != 80:
        raise PermissionError()
    return "http", name, port, target


def send_reply(stream, status):
    reason = HTTPStatus(status).phrase
    marker = "X-UV-URL-Policy: denied\r\n" if status == 403 else ""
    stream.settimeout(IO_TIMEOUT)
    stream.sendall(
        f"HTTP/1.1 {status} {reason}\r\n{marker}Content-Length: 0\r\nConnection: close\r\n\r\n".encode(
            "ascii"
        )
    )


def health_request(request, server):
    if (
        request.method not in {"GET", "HEAD"}
        or request.target != "/__uv_network_proxy_health"
        or request.length
        or request.expect_continue
    ):
        return False
    requested = request_host(request)
    port = server.server_address[1]
    return requested in {
        f"127.0.0.1:{port}",
        f"[::1]:{port}",
        f"localhost:{port}",
    }


def make_tls_context(certificate, key, state):
    allowed = frozenset(state.url_policy.hosts)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.set_alpn_protocols(["http/1.1"])
    context.load_cert_chain(certificate, key)

    def server_name(stream, value, _initial_context):
        try:
            name = hostname(value)
        except ValueError:
            stream._url_rejected_host = ""
            return ssl.ALERT_DESCRIPTION_UNRECOGNIZED_NAME
        expected = getattr(stream, "_url_expected_host", None)
        if name not in allowed or (expected is not None and name != expected):
            stream._url_rejected_host = name
            return ssl.ALERT_DESCRIPTION_UNRECOGNIZED_NAME
        stream._url_sni = name
        return None

    context.sni_callback = server_name
    return context


def response_headers(response):
    headers = response.getheaders()
    values = {}
    for name, value in headers:
        if not TOKEN.fullmatch(name) or any(
            character != "\t" and not 32 <= ord(character) <= 126 for character in value
        ):
            raise http.client.HTTPException("invalid upstream header")
        values.setdefault(name.lower(), []).append(value)
    if (
        response.status < 200
        or "upgrade" in values
        or len(values.get("content-length", [])) > 1
        or len(values.get("transfer-encoding", [])) > 1
        or ("content-length" in values and "transfer-encoding" in values)
        or values.get("transfer-encoding", [""])[0].lower() not in {"", "chunked"}
    ):
        raise http.client.HTTPException("unsupported upstream framing")
    if "content-length" in values:
        length = values["content-length"][0]
        if not length.isascii() or not length.isdecimal() or len(length) > 20:
            raise http.client.HTTPException("invalid upstream length")
    try:
        connection = connection_tokens(",".join(values.get("connection", [])))
    except RequestError:
        raise http.client.HTTPException("invalid upstream connection header") from None
    if "upgrade" in connection:
        raise http.client.HTTPException("upstream upgrade is not allowed")
    # Only locally generated denials may carry this proof marker.
    hop = HOP_HEADERS | connection | {"x-uv-url-policy"}
    return [(name, value) for name, value in headers if name.lower() not in hop]


def forward(stream, state, request, scheme, name, port, target):
    if not state.url_policy.permits(scheme, name, port, request.method, target):
        state.count("denied", name)
        raise PermissionError()
    if request.expect_continue:
        stream.sendall(b"HTTP/1.1 100 Continue\r\n\r\n")
    body = read_body(stream, request.length)
    hop = (
        HOP_HEADERS
        | {"host", "expect"}
        | connection_tokens(request.headers.get("connection", ("", ""))[1])
    )
    headers = {
        original: value
        for lowered, (original, value) in request.headers.items()
        if lowered not in hop
    }
    headers["Host"] = name
    headers["Connection"] = "close"
    connection = http.client.HTTPConnection(name, port, timeout=IO_TIMEOUT)
    response_started = False
    try:
        connection.sock = state.connect(name, port)
        if scheme == "https":
            if (
                not state.upstream_context.check_hostname
                or state.upstream_context.verify_mode != ssl.CERT_REQUIRED
            ):
                raise http.client.HTTPException("upstream TLS must be verified")
            connection.sock = state.upstream_context.wrap_socket(
                connection.sock, server_hostname=name
            )
            if connection.sock.selected_alpn_protocol() not in {None, "http/1.1"}:
                raise http.client.HTTPException("unsupported upstream protocol")
        connection.request(request.method, target, body=body, headers=headers)
        response = connection.getresponse()
        fields = response_headers(response)
        output = [
            f"HTTP/1.1 {response.status} {HTTPStatus(response.status).phrase}\r\n"
        ]
        output.extend(f"{key}: {value}\r\n" for key, value in fields)
        output.append("Connection: close\r\n\r\n")
        stream.settimeout(IO_TIMEOUT)
        stream.sendall("".join(output).encode("ascii"))
        response_started = True
        if request.method != "HEAD":
            while block := response.read(65536):
                stream.sendall(block)
    except PermissionError:
        if not response_started:
            send_reply(stream, 403)
    except (OSError, ValueError, http.client.HTTPException):
        state.count("http_error")
        if not response_started:
            send_reply(stream, 502)
    finally:
        connection.close()


def serve_tls(stream, server, expected=None):
    stream.settimeout(IO_TIMEOUT)
    with server.state.tls_context.wrap_socket(
        stream, server_side=True, do_handshake_on_connect=False
    ) as secured:
        secured._url_expected_host = expected
        try:
            secured.do_handshake()
        except ssl.SSLError:
            if (rejected := getattr(secured, "_url_rejected_host", None)) is not None:
                server.state.count("denied", rejected)
            raise
        name = getattr(secured, "_url_sni", None)
        if name is None or (expected is not None and name != expected):
            raise PermissionError()
        if secured.selected_alpn_protocol() not in {None, "http/1.1"}:
            raise PermissionError()
        serve_http(secured, server, tls_host=name)


def connect_request(stream, server, request):
    if request.length or request.expect_continue:
        raise RequestError()
    private = request.target if request.target in server.state.private_origins else None
    if private:
        name = server.state.private_origins[private]
    else:
        name, port = authority(request.target, 443)
        if port != 443 or ":" not in request.target:
            raise PermissionError()
    if "host" in request.headers:
        if private:
            matched = request_host(request) == private
        else:
            matched = authority(request_host(request), 443) == (name, 443)
        if not matched:
            raise PermissionError()
    elif request.version != "HTTP/1.0":
        raise RequestError()
    if name not in server.state.url_policy.hosts:
        server.state.count("denied", name)
        raise PermissionError()
    stream.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
    if private:
        serve_http(stream, server, private_authority=private)
    else:
        try:
            serve_tls(stream, server, expected=name)
        except (OSError, ValueError):
            server.state.count("tls_error")


def serve_http(stream, server, tls_host=None, private_authority=None):
    stream.settimeout(IO_TIMEOUT)
    request = None
    try:
        request = read_request(stream)
        if request.method == "CONNECT":
            if tls_host is not None or private_authority is not None:
                raise PermissionError()
            connect_request(stream, server, request)
            return
        if request.method not in METHODS:
            raise RequestError(405)
        if (
            tls_host is None
            and private_authority is None
            and health_request(request, server)
        ):
            send_reply(stream, 204)
            return
        if tls_host is not None:
            name, port = authority(request_host(request), 443)
            if name != tls_host or port != 443:
                raise PermissionError()
            scheme, target = "https", origin_target(request.target)
        else:
            scheme, name, port, target = cleartext_target(
                request, server.state, private_authority
            )
        forward(stream, server.state, request, scheme, name, port, target)
    except RequestError as error:
        server.state.count("http_error")
        send_reply(stream, error.status)
    except PermissionError:
        if request is None:
            send_reply(stream, 403)
        else:
            try:
                deny_request(stream, request)
            except RequestError as error:
                server.state.count("http_error")
                send_reply(stream, error.status)
    except (OSError, ValueError, http.client.HTTPException):
        server.state.count("http_error")
        send_reply(stream, 502)


class HTTPHandler(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            serve_http(self.request, self.server)
        except OSError:
            self.server.state.count("http_error")


class TLSHandler(socketserver.BaseRequestHandler):
    def handle(self):
        # The listener remains a plain TCP socket. Each worker owns its timed
        # TLS handshake, so an incomplete ClientHello cannot block accepts.
        try:
            serve_tls(self.request, self.server)
        except (OSError, ValueError):
            self.server.state.count("tls_error")
