"""Replay immutable package-index and wheel fixtures on an ephemeral loopback port."""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import html
import json
import re
import threading
import time
import tomllib
import zipfile
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit


def normalize(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


class Fixtures:
    def __init__(
        self,
        directory: Path,
        manifest: Path,
        lockfiles: list[Path],
        *,
        core_metadata: bool = True,
    ) -> None:
        self.files: dict[str, Path] = {}
        self.metadata: dict[str, bytes] = {}
        packages: dict[str, dict[str, dict]] = {}
        for lockfile in lockfiles:
            with lockfile.open("rb") as file:
                lock = tomllib.load(file)
            for package in lock["package"]:
                if "registry" not in package["source"]:
                    continue
                releases = packages.setdefault(normalize(package["name"]), {})
                for artifact in [
                    *package.get("wheels", []),
                    *([package["sdist"]] if "sdist" in package else []),
                ]:
                    filename = unquote(
                        urlsplit(artifact["url"]).path.rsplit("/", 1)[-1]
                    )
                    releases[filename] = {
                        "filename": filename,
                        "url": f"/files/{filename}",
                        "hashes": {"sha256": artifact["hash"].removeprefix("sha256:")},
                        "size": artifact.get("size"),
                        "core-metadata": False,
                    }
        for item in json.loads(manifest.read_text()):
            filename = item["filename"]
            path = directory / filename
            if not filename.endswith(".whl"):
                continue
            if not path.is_file():
                raise FileNotFoundError(f"Run prepare-fixtures.py first: {path}")
            with zipfile.ZipFile(path) as wheel:
                names = [
                    name
                    for name in wheel.namelist()
                    if name.endswith(".dist-info/METADATA")
                ]
                if len(names) != 1:
                    raise ValueError(f"Expected one METADATA file in {filename}")
                metadata = wheel.read(names[0])
            headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
            digest = hashlib.sha256(metadata).hexdigest()
            self.files[filename] = path
            self.metadata[filename + ".metadata"] = metadata
            packages.setdefault(normalize(headers["Name"]), {})[filename] = {
                "filename": filename,
                "url": f"/files/{filename}",
                "hashes": {"sha256": item["sha256"]},
                "size": path.stat().st_size,
                "requires-python": headers.get("Requires-Python"),
                "core-metadata": {"sha256": digest} if core_metadata else False,
            }
        self.simple = {
            name: json.dumps(
                {
                    "name": name,
                    "meta": {"api-version": "1.4"},
                    "files": list(files.values()),
                }
            ).encode()
            for name, files in packages.items()
        }
        self.flat = (
            "<!DOCTYPE html><html><body>"
            + "".join(
                f'<a href="/files/{html.escape(name)}">{html.escape(name)}</a>'
                for name in self.files
            )
            + "</body></html>"
        ).encode()


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, fixtures: Fixtures, delay: float, body_delay: float = 0) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.fixtures = fixtures
        self.delay = delay
        self.body_delay = body_delay
        self.counts: Counter[str] = Counter()
        self.active: Counter[str] = Counter()
        self.counts_lock = threading.Lock()

    def begin_counted_request(
        self, method: str, path: str, file_kind: str | None
    ) -> tuple[str, ...]:
        groups = (method,)
        if file_kind is not None:
            groups += (f"{method} /files", f"{method} file {file_kind}")
        with self.counts_lock:
            self.counts[f"{method} {path}"] += 1
            for group in groups:
                self.active[group] += 1
                peak = f"{group} [max-active]"
                self.counts[peak] = max(self.counts[peak], self.active[group])
        return groups

    def end_counted_request(self, groups: tuple[str, ...]) -> None:
        with self.counts_lock:
            for group in groups:
                self.active[group] -= 1

    def record_bytes(self, key: str, size: int) -> None:
        with self.counts_lock:
            self.counts[key] += size


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: Server

    def log_message(self, format: str, *args: object) -> None:
        pass

    def handle(self) -> None:
        try:
            super().handle()
        except (BrokenPipeError, ConnectionResetError):
            # Interrupted-download workloads close both active and keep-alive connections.
            pass

    def do_HEAD(self) -> None:
        self.handle_request(head=True)

    def do_GET(self) -> None:
        self.handle_request(head=False)

    def do_POST(self) -> None:
        if urlsplit(self.path).path == "/reset":
            with self.server.counts_lock:
                self.server.counts.clear()
                for group, active in self.server.active.items():
                    if active:
                        self.server.counts[f"{group} [max-active]"] = active
            self.respond(b"{}", "application/json")
        else:
            self.send_error(404)

    def handle_request(self, *, head: bool) -> None:
        path = unquote(urlsplit(self.path).path)
        if path == "/stats":
            with self.server.counts_lock:
                counts = dict(self.server.counts)
                counts.update(
                    {
                        f"{group} [active]": active
                        for group, active in self.server.active.items()
                    }
                )
                body = json.dumps(counts).encode()
            self.respond(body, "application/json", head=head)
            return
        file_kind = None
        if path.startswith("/files/"):
            if path.endswith(".metadata"):
                file_kind = "metadata"
            else:
                file_kind = "ranges" if self.headers.get("Range") else "bodies"
        groups = self.server.begin_counted_request(self.command, path, file_kind)
        try:
            time.sleep(self.server.delay)
            self.dispatch_request(path, head=head)
        finally:
            self.server.end_counted_request(groups)

    def dispatch_request(self, path: str, *, head: bool) -> None:
        parts = path.strip("/").split("/")
        if len(parts) == 2 and parts[0] in {"simple", "simple-a", "simple-b"}:
            body = self.server.fixtures.simple.get(normalize(parts[1]))
            if body is not None:
                self.respond(body, "application/vnd.pypi.simple.v1+json", head=head)
                return
        elif len(parts) == 2 and parts[0] == "flat" and parts[1] in {"a", "b"}:
            self.respond(self.server.fixtures.flat, "text/html", head=head)
            return
        elif len(parts) == 2 and parts[0] == "files":
            filename = parts[1]
            metadata = self.server.fixtures.metadata.get(filename)
            if metadata is not None:
                self.respond(metadata, "application/octet-stream", head=head)
                return
            file = self.server.fixtures.files.get(filename)
            if file is not None:
                self.serve_file(file, head=head)
                return
        self.send_error(404)

    def respond(self, body: bytes, content_type: str, *, head: bool = False) -> None:
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "public, max-age=3600")
        self.end_headers()
        if not head:
            self.wfile.write(body)
            path = unquote(urlsplit(self.path).path)
            if path not in {"/stats", "/reset"}:
                self.server.record_bytes(f"{self.command} {path} [bytes]", len(body))

    def serve_file(self, path: Path, *, head: bool) -> None:
        size = path.stat().st_size
        start, end = 0, size - 1
        range_header = self.headers.get("Range")
        if range_header:
            match = re.fullmatch(r"bytes=(\d*)-(\d*)", range_header)
            if match is None or not any(match.groups()):
                self.send_error(416)
                return
            first, last = match.groups()
            if first:
                start = int(first)
                end = min(int(last), size - 1) if last else size - 1
            else:
                start = max(0, size - int(last))
            if start > end or start >= size:
                self.send_error(416)
                return
        length = end - start + 1
        kind = "range" if range_header else "body"
        if not head:
            with self.server.counts_lock:
                self.server.counts[f"GET /files/{path.name} [{kind}]"] += 1
        self.send_response(206 if range_header else 200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(length))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Cache-Control", "public, max-age=3600")
        if range_header:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        if not head:
            with path.open("rb") as file:
                file.seek(start)
                while length:
                    chunk = file.read(min(length, 64 * 1024))
                    if not chunk:
                        raise EOFError(path)
                    self.wfile.write(chunk)
                    self.server.record_bytes(
                        f"GET /files/{path.name} [{kind}-bytes]", len(chunk)
                    )
                    length -= len(chunk)
                    if length and not range_header and self.server.body_delay:
                        time.sleep(self.server.body_delay)


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-fixtures"
    )
    parser.add_argument(
        "--manifest", type=Path, default=Path(__file__).with_name("fixtures.json")
    )
    parser.add_argument("--lockfile", type=Path, action="append", default=[])
    parser.add_argument("--delay-ms", type=float, default=20)
    parser.add_argument("--body-delay-ms", type=float, default=0)
    parser.add_argument("--no-core-metadata", action="store_true")
    args = parser.parse_args()
    if args.delay_ms < 0 or args.body_delay_ms < 0:
        parser.error("request and body delays must be nonnegative")
    server = Server(
        Fixtures(
            args.directory,
            args.manifest,
            args.lockfile,
            core_metadata=not args.no_core_metadata,
        ),
        args.delay_ms / 1000,
        args.body_delay_ms / 1000,
    )
    print(f"http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
