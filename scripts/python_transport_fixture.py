"""Serve bounded loopback TLS bodies while honoring HTTP/2 receive windows."""

import select
import socket
import ssl
import threading
import time

from python_reference_tls import exact, frame


def hpack_string(value):
    if len(value) >= 127:
        raise ValueError("The local HPACK fixture only supports short literals")
    return bytes([len(value)]) + value


def literal(name, value):
    return b"\x00" + hpack_string(name) + hpack_string(value)


class StreamingServer:
    def __init__(self, certificate, private_key, protocol, body_size, canaries):
        if not 1 <= body_size <= 16 * 1024 * 1024:
            raise ValueError("The local fixture body exceeds its bound")
        self.body_size = body_size
        self.canaries = canaries
        self.result = {"accepted_connections": 0, "requests": 0, "body_bytes_sent": 0}
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(10)
        self.port = self.listener.getsockname()[1]
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(certificate, private_key)
        self.context.set_alpn_protocols([protocol])
        self.context.set_servername_callback(
            lambda _socket, name, _context: self.result.update(
                sni_matches=name == "releases.astral.sh"
            )
        )
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def body_chunk(self, offset, count):
        prefix = self.canaries["body"]
        value = prefix[offset : offset + count] if offset < len(prefix) else b""
        return value + b"b" * (count - len(value))

    def serve(self):
        try:
            raw, _address = self.listener.accept()
            self.result["accepted_connections"] += 1
            raw.settimeout(8)
            with self.context.wrap_socket(raw, server_side=True) as connection:
                self.result["alpn"] = connection.selected_alpn_protocol()
                if self.result["alpn"] == "h2":
                    self.serve_h2(connection)
                elif self.result["alpn"] == "http/1.1":
                    self.serve_h1(connection)
                else:
                    raise ValueError(
                        "The local fixture negotiated no supported protocol"
                    )
                self.result["body_sent_monotonic"] = time.monotonic()
                connection.settimeout(1)
                try:
                    connection.unwrap().close()
                except OSError:
                    pass
        except (OSError, ValueError, EOFError) as error:
            self.result["error_type"] = type(error).__name__
        finally:
            self.listener.close()

    def serve_h1(self, connection):
        request = b""
        while b"\r\n\r\n" not in request:
            if len(request) >= 16384:
                raise ValueError("The local request header exceeded its bound")
            request += connection.recv(4096)
        self.result["requests"] += 1
        self.result["request_monotonic"] = time.monotonic()
        connection.sendall(
            b"HTTP/1.1 200 OK\r\nContent-Length: "
            + str(self.body_size).encode()
            + b"\r\nX-Control-Header: "
            + self.canaries["header"]
            + b"\r\nConnection: close\r\n\r\n"
        )
        offset = 0
        while offset < self.body_size:
            chunk = self.body_chunk(offset, min(16384, self.body_size - offset))
            connection.sendall(chunk)
            offset += len(chunk)
        self.result["body_bytes_sent"] = offset

    def serve_h2(self, connection):
        connection.sendall(frame(4, 0, 0))
        if exact(connection, 24) != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n":
            raise ValueError("The local HTTP/2 preface differs")
        initial_window = 65535
        connection_window = 65535
        stream_window = 0
        stream_id = None
        control_frames = 0

        def receive():
            nonlocal \
                initial_window, \
                connection_window, \
                stream_window, \
                stream_id, \
                control_frames
            header = exact(connection, 9)
            length = int.from_bytes(header[:3], "big")
            if length > 16384:
                raise ValueError("The local HTTP/2 control frame exceeds its bound")
            kind, flags = header[3:5]
            stream = int.from_bytes(header[5:], "big") & 0x7FFFFFFF
            payload = exact(connection, length)
            control_frames += 1
            if control_frames > 8192:
                raise ValueError("The local control-frame count exceeds its bound")
            if kind == 4 and not flags & 1:
                if length % 6:
                    raise ValueError("The local SETTINGS payload differs")
                for offset in range(0, length, 6):
                    setting = int.from_bytes(payload[offset : offset + 2], "big")
                    value = int.from_bytes(payload[offset + 2 : offset + 6], "big")
                    if setting == 4:
                        stream_window += value - initial_window
                        initial_window = value
                connection.sendall(frame(4, 1, 0))
            elif kind == 8:
                if length != 4:
                    raise ValueError("The local WINDOW_UPDATE payload differs")
                increment = int.from_bytes(payload, "big") & 0x7FFFFFFF
                if stream == 0:
                    connection_window += increment
                elif stream == stream_id:
                    stream_window += increment
            elif kind == 1 and stream:
                if stream_id is not None:
                    raise ValueError("The local fixture received a second request")
                stream_id = stream
                stream_window = initial_window
                self.result["requests"] += 1
                self.result["request_monotonic"] = time.monotonic()
            elif kind in (3, 7):
                raise ValueError("The local HTTP/2 peer ended the connection")

        while stream_id is None:
            receive()
        connection.sendall(
            frame(
                1,
                4,
                stream_id,
                b"\x88"
                + literal(b"content-length", str(self.body_size).encode())
                + literal(b"x-control-header", self.canaries["header"]),
            )
        )
        offset = 0
        while offset < self.body_size:
            while connection.pending() or select.select([connection], [], [], 0)[0]:
                receive()
            if min(connection_window, stream_window) <= 0:
                receive()
                continue
            count = min(
                16384, self.body_size - offset, connection_window, stream_window
            )
            connection.sendall(frame(0, 0, stream_id, self.body_chunk(offset, count)))
            offset += count
            connection_window -= count
            stream_window -= count
        connection.sendall(
            frame(
                1, 5, stream_id, literal(b"x-control-trailer", self.canaries["trailer"])
            )
        )
        self.result.update(body_bytes_sent=offset, control_frames=control_frames)

    def finish(self):
        self.thread.join(timeout=12)
        if self.thread.is_alive():
            raise RuntimeError("The bounded loopback fixture did not terminate")
        return self.result
