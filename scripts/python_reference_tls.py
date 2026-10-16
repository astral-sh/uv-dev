"""Serve small loopback responses for the Python reference capture preflights."""

import gzip
import io
import socket
import ssl
import tarfile
import threading
import time


def fixture_body():
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        payload = b"local Python reference transfer fixture\n"
        member = tarfile.TarInfo("python/fixture.txt")
        member.size = len(payload)
        archive.addfile(member, io.BytesIO(payload))
    return gzip.compress(buffer.getvalue(), mtime=0)


BODY = fixture_body()


def frame(kind, flags, stream, payload=b""):
    return (
        len(payload).to_bytes(3, "big")
        + bytes([kind, flags])
        + stream.to_bytes(4, "big")
        + payload
    )


def exact(connection, count):
    output = b""
    while len(output) < count:
        received = connection.recv(count - len(output))
        if not received:
            raise EOFError("Loopback peer closed before the expected frame")
        output += received
    return output


class Server:
    def __init__(
        self,
        certificate,
        private_key,
        protocol="h2",
        *,
        proxy=False,
        refuse_stream=False,
    ):
        self.protocol = protocol
        self.proxy = proxy
        self.refuse_stream = refuse_stream
        self.result = {
            "sni": [],
            "accepted_connections": 0,
            "requests": 0,
            "request_headers": {},
            "body_bytes": len(BODY),
        }
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(12)
        self.address, self.port = self.listener.getsockname()
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(certificate, private_key)
        self.context.set_alpn_protocols([protocol])
        self.context.set_servername_callback(
            lambda _socket, name, _context: self.result["sni"].append(name)
        )
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        try:
            if self.refuse_stream:
                self.serve_refusals()
                return
            raw, _ = self.listener.accept()
            self.result["accepted_connections"] += 1
            raw.settimeout(10)
            if self.proxy:
                with raw:
                    request = b""
                    while b"\r\n\r\n" not in request and len(request) < 65536:
                        received = raw.recv(4096)
                        if not received:
                            raise EOFError("Loopback proxy request ended early")
                        request += received
                    method, authority, version = (
                        request.split(b"\r\n", 1)[0].decode().split(" ")
                    )
                    if method != "CONNECT" or version != "HTTP/1.1":
                        raise ValueError("Expected a CONNECT request")
                    self.result["connect_authority"] = authority
                    raw.sendall(
                        b"HTTP/1.1 502 Local diagnostic proxy\r\n"
                        b"Content-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                return
            with self.context.wrap_socket(raw, server_side=True) as connection:
                self.result["alpn"] = connection.selected_alpn_protocol()
                if self.result["alpn"] == "h2":
                    connection.sendall(frame(4, 0, 0))
                    if exact(connection, 24) != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n":
                        raise ValueError("Expected the HTTP/2 client preface")
                    for _ in range(32):
                        header = exact(connection, 9)
                        length = int.from_bytes(header[:3], "big")
                        kind, flags = header[3:5]
                        stream = int.from_bytes(header[5:], "big") & 0x7FFFFFFF
                        exact(connection, length)
                        if kind == 4 and not flags & 1:
                            connection.sendall(frame(4, 1, 0))
                        if kind == 1 and stream:
                            self.result["requests"] += 1
                            self.result["request_monotonic"] = time.monotonic()
                            connection.sendall(
                                frame(1, 4, stream, b"\x88") + frame(0, 1, stream, BODY)
                            )
                            break
                    else:
                        raise ValueError("No HTTP/2 request headers arrived")
                elif self.result["alpn"] == "http/1.1":
                    request = b""
                    while b"\r\n\r\n" not in request and len(request) < 65536:
                        received = connection.recv(4096)
                        if not received:
                            raise EOFError("Loopback request ended early")
                        request += received
                    lines = request.decode("ascii").split("\r\n")
                    method, target, version = lines[0].split(" ")
                    if method != "GET" or version != "HTTP/1.1":
                        raise ValueError("Expected an HTTP/1.1 GET")
                    self.result["requests"] += 1
                    self.result["request_target"] = target
                    self.result["request_monotonic"] = time.monotonic()
                    allowed = {"host", "user-agent", "accept", "accept-encoding"}
                    for line in lines[1:]:
                        if ":" in line:
                            name, value = line.split(":", 1)
                            if name.lower() in allowed:
                                self.result["request_headers"][name.lower()] = (
                                    value.strip()
                                )
                    connection.sendall(
                        b"HTTP/1.1 200 OK\r\nContent-Length: "
                        + str(len(BODY)).encode()
                        + b"\r\nConnection: close\r\n\r\n"
                        + BODY
                    )
                else:
                    raise ValueError("No expected ALPN protocol was negotiated")
                self.result["body_sent_monotonic"] = time.monotonic()
                connection.settimeout(1)
                try:
                    connection.unwrap().close()
                except OSError:
                    pass
        except (OSError, ValueError, EOFError) as error:
            self.result["error_type"] = type(error).__name__
            self.result["error"] = str(error)
        finally:
            self.listener.close()

    def serve_refusals(self):
        self.listener.settimeout(1)
        for _ in range(8):
            try:
                raw, _ = self.listener.accept()
            except TimeoutError:
                return
            self.result["accepted_connections"] += 1
            raw.settimeout(2)
            try:
                with self.context.wrap_socket(raw, server_side=True) as connection:
                    self.result["alpn"] = connection.selected_alpn_protocol()
                    if self.result["alpn"] != "h2":
                        raise ValueError("The refused-stream fixture requires HTTP/2")
                    connection.sendall(frame(4, 0, 0))
                    if exact(connection, 24) != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n":
                        raise ValueError("Expected the HTTP/2 client preface")
                    for _ in range(32):
                        header = exact(connection, 9)
                        length = int.from_bytes(header[:3], "big")
                        kind, flags = header[3:5]
                        stream = int.from_bytes(header[5:], "big") & 0x7FFFFFFF
                        exact(connection, length)
                        if kind == 4 and not flags & 1:
                            connection.sendall(frame(4, 1, 0))
                        if kind == 1 and stream:
                            self.result["requests"] += 1
                            connection.sendall(
                                frame(3, 0, stream, (7).to_bytes(4, "big"))
                            )
            except (OSError, EOFError) as error:
                self.result.setdefault("connection_end_types", []).append(
                    type(error).__name__
                )

    def finish(self):
        self.thread.join(timeout=15)
        if self.thread.is_alive():
            raise RuntimeError("Loopback server did not terminate")
        return self.result
