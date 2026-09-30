"""Replay pinned distributions under controlled network conditions and compare uv binaries."""

from __future__ import annotations

import argparse
import concurrent.futures
import copy
import email.parser
import email.utils
import hashlib
import html
import http.client
import io
import json
import math
import os
import random
import re
import shutil
import socket
import ssl
import statistics
import subprocess
import tarfile
import tempfile
import threading
import time
import urllib.error
import urllib.request
import zipfile
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

HERE = Path(__file__).resolve().parent


def netem_profile() -> dict:
    return json.loads(os.environ.get("UV_BENCH_NETEM", "{}"))


def network_floor(
    profile: dict,
    required_bytes: int,
    required_waves: int,
    required_latency_ms: float | None = None,
    required_wait_ms: float = 0,
) -> float:
    netem = netem_profile()
    rates = [
        rate
        for rate in (
            profile.get("bytes_per_second", 0),
            netem.get("rate_mbit", 0) * 125000,
        )
        if rate
    ]
    application_latency = (
        required_waves
        * max(0, profile.get("latency_ms", 0) - profile.get("jitter_ms", 0))
        if required_latency_ms is None
        else required_latency_ms
    )
    latency = application_latency + required_waves * netem.get("rtt_ms", 0)
    return required_wait_ms / 1000 + max(
        required_bytes / min(rates) if rates else 0, latency / 1000
    )


def concurrent_latency_floor(
    latencies_ms: list[float], concurrency: int, rtt_ms: float = 0
) -> tuple[int, float]:
    """Return request waves and the latency contribution expected by `network_floor`."""
    if not latencies_ms or concurrency < 1:
        raise ValueError(
            "Concurrent request bounds require requests and a positive limit"
        )
    concurrency = min(concurrency, len(latencies_ms))
    waves = math.ceil(len(latencies_ms) / concurrency)
    durations = [latency + rtt_ms for latency in latencies_ms]
    # Combine RTT with each request before taking the scheduling bound. Adding a
    # full wave of RTT to the longest application delay can overstate the bound
    # when short requests finish while that long request is still running.
    duration = max(max(durations), sum(durations) / concurrency, waves * min(durations))
    return waves, max(0, duration - waves * rtt_ms)


def normalize(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def digest(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def tree_digest(
    root: Path,
    normalized_files: tuple[str, ...] | list[str] = (),
    context: dict | None = None,
    normalized_symlinks: tuple[str, ...] | list[str] = (),
) -> dict:
    if not root.is_dir():
        raise ValueError(f"Verification directory does not exist: {root}")
    entries = []
    for path in sorted(root.rglob("*")):
        name = path.relative_to(root).as_posix()
        if path.is_symlink():
            target = os.readlink(path)
            if any(Path(name).match(pattern) for pattern in normalized_symlinks):
                if context is None:
                    raise ValueError("Tree normalization requires a trial context")
                if Path(target).is_absolute():
                    try:
                        relative = Path(target).relative_to(context["work"])
                    except ValueError:
                        pass
                    else:
                        if ".." not in relative.parts:
                            target = "[WORK]/" + relative.as_posix()
            entries.append((name, "symlink", target))
        elif path.is_file():
            if any(Path(name).match(pattern) for pattern in normalized_files):
                if context is None:
                    raise ValueError("Tree normalization requires a trial context")
                contents = normalize_output(path.read_bytes(), context)
                file_digest = hashlib.sha256(contents).hexdigest()
            else:
                file_digest = digest(path)
            entries.append((name, "file", path.stat().st_mode & 0o111, file_digest))
        elif path.is_dir():
            entries.append((name, "directory"))
        else:
            raise ValueError(f"Unsupported verification entry: {path}")
    return {
        "entries": len(entries),
        "sha256": hashlib.sha256(json.dumps(entries).encode()).hexdigest(),
    }


def prepare(manifest: Path, directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    for item in json.loads(manifest.read_text()):
        path = directory / item["filename"]
        if not path.exists() or digest(path) != item["sha256"]:
            temporary = path.with_suffix(path.suffix + ".partial")
            for attempt in range(4):
                try:
                    with (
                        urllib.request.urlopen(item["url"], timeout=60) as response,
                        temporary.open("wb") as output,
                    ):
                        shutil.copyfileobj(response, output)
                    break
                except (OSError, urllib.error.URLError):
                    if attempt == 3:
                        raise
                    time.sleep(2**attempt)
            if digest(temporary) != item["sha256"]:
                raise ValueError(f"Fixture hash mismatch: {path.name}")
            temporary.replace(path)
        if path.stat().st_size != item["size"]:
            raise ValueError(f"Fixture size mismatch: {path.name}")


def distribution_metadata(path: Path) -> bytes:
    if path.suffix in (".whl", ".zip"):
        with zipfile.ZipFile(path) as archive:
            names = [
                name
                for name in archive.namelist()
                if (
                    name.endswith(".dist-info/METADATA")
                    if path.suffix == ".whl"
                    else name.count("/") == 1 and name.endswith("/PKG-INFO")
                )
            ]
            if len(names) != 1:
                raise ValueError(f"Expected one distribution metadata file: {path}")
            return archive.read(names[0])
    with tarfile.open(path) as archive:
        members = [
            member
            for member in archive
            if member.isfile()
            and member.name.count("/") == 1
            and member.name.endswith("/PKG-INFO")
        ]
        if len(members) != 1:
            raise ValueError(f"Expected one distribution metadata file: {path}")
        with archive.extractfile(members[0]) as metadata:
            return metadata.read()


class Fixtures:
    def __init__(self, manifest: Path, directory: Path, pep658: bool) -> None:
        self.files: dict[str, Path] = {}
        self.routes: dict[str, Path] = {}
        self.metadata: dict[str, bytes] = {}
        self.hashes: dict[str, str] = {}
        self.packages: dict[str, list[dict]] = {}
        self.osv: dict | None = None
        for item in json.loads(manifest.read_text()):
            path = directory / item["filename"]
            if digest(path) != item["sha256"]:
                raise ValueError(f"Fixture hash mismatch: {path}")
            self.files[path.name] = path
            self.hashes[path.name] = item["sha256"]
            for route in item.get("paths", []):
                if not route.startswith("/") or route in self.routes:
                    raise ValueError(f"Invalid or duplicate fixture path: {route}")
                self.routes[route] = path
            if item.get("kind") == "osv":
                if self.osv is not None:
                    raise ValueError("Only one OSV fixture is supported")
                self.osv = json.loads(path.read_text())
                continue
            if item.get("kind") == "raw":
                continue
            metadata = distribution_metadata(path)
            headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
            self.metadata[path.name + ".metadata"] = metadata
            self.packages.setdefault(normalize(headers["Name"]), []).append(
                {
                    "filename": path.name,
                    "url": f"/files/{path.name}",
                    "hashes": {"sha256": item["sha256"]},
                    "size": path.stat().st_size,
                    "requires-python": headers.get("Requires-Python"),
                    "core-metadata": (
                        {"sha256": hashlib.sha256(metadata).hexdigest()}
                        if pep658 and item.get("pep658", True)
                        else False
                    ),
                }
            )
        self.rebuild_indexes()

    def with_file_urls(self, urls: dict[str, str]) -> Fixtures:
        """Copy index responses while retaining the immutable artifact files."""
        fixtures = copy.copy(self)
        fixtures.packages = {
            name: [
                dict(file, url=urls.get(file["filename"], file["url"]))
                for file in files
            ]
            for name, files in self.packages.items()
        }
        fixtures.rebuild_indexes()
        return fixtures

    def rebuild_indexes(self) -> None:
        self.simple = {
            name: json.dumps(
                {"name": name, "meta": {"api-version": "1.4"}, "files": files},
                separators=(",", ":"),
            ).encode()
            for name, files in self.packages.items()
        }
        links = []
        for files in self.packages.values():
            for file in files:
                attributes = {
                    "href": f"{file['url']}#sha256={file['hashes']['sha256']}",
                    "data-requires-python": file["requires-python"],
                    "data-core-metadata": (
                        f"sha256={file['core-metadata']['sha256']}"
                        if file["core-metadata"]
                        else None
                    ),
                }
                attributes = " ".join(
                    f'{name}="{html.escape(value, quote=True)}"'
                    for name, value in attributes.items()
                    if value is not None
                )
                links.append(f"<a {attributes}>{html.escape(file['filename'])}</a>")
        self.flat = ("<!doctype html>\n" + "\n".join(links) + "\n").encode()


def osv_query_response(configuration: dict, request: bytes) -> tuple[bytes, dict]:
    """Validate a pinned query batch and return its deterministic page results."""
    payload = json.loads(request)
    if not isinstance(payload, dict) or set(payload) != {"queries"}:
        raise ValueError("Expected an OSV query batch")
    queries = payload["queries"]
    if not isinstance(queries, list) or not 1 <= len(queries) <= 1000:
        raise ValueError("OSV query batches require between 1 and 1000 queries")
    results = []
    first = None
    for query in queries:
        if not isinstance(query, dict) or set(query) not in (
            {"package", "version"},
            {"package", "version", "page_token"},
        ):
            raise ValueError("Unexpected OSV query fields")
        package = query["package"]
        if not isinstance(package, dict) or set(package) != {"name", "ecosystem"}:
            raise ValueError("Expected an OSV package name and ecosystem")
        name = package["name"]
        dependency = configuration["dependencies"].get(name)
        if (
            dependency is None
            or package["ecosystem"] != "PyPI"
            or query["version"] != dependency["version"]
        ):
            raise ValueError("Unknown OSV fixture dependency")
        token = query.get("page_token")
        page = 0 if token is None else int(token.removeprefix(name + ":"))
        if not 0 <= page < dependency["pages"] or (
            token is not None and (page == 0 or token != f"{name}:{page}")
        ):
            raise ValueError("Unexpected OSV page token")
        first = first or f"{name}:{page}"
        result = {"vulns": []}
        if page + 1 < dependency["pages"]:
            result["next_page_token"] = f"{name}:{page + 1}"
        results.append(result)
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return json.dumps({"results": results}, separators=(",", ":")).encode(), {
        "query_sha256": hashlib.sha256(canonical).hexdigest(),
        "query_count": len(queries),
        "first_query": first,
    }


class Limiter:
    """One response-body bottleneck shared by every connection."""

    def __init__(self, bytes_per_second: float) -> None:
        self.rate = bytes_per_second
        self.next_free = 0.0
        self.lock = threading.Lock()

    def wait(self, length: int) -> None:
        if not self.rate:
            return
        with self.lock:
            now = time.monotonic()
            self.next_free = max(now, self.next_free) + length / self.rate
            wait = self.next_free - now
        time.sleep(wait)


class Server(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 256

    def __init__(
        self,
        fixtures: Fixtures,
        profile: dict,
        socket_path: Path | None = None,
        *,
        git_root: Path | None = None,
        limiter: Limiter | None = None,
        origin: str = "index",
    ) -> None:
        if profile.get("artifact_origins"):
            raise ValueError("Use Replay to configure multiple artifact origins")
        self.public_url: str | None = None
        if socket_path is not None:
            self.address_family = socket.AF_UNIX
        super().__init__(str(socket_path) if socket_path else ("127.0.0.1", 0), Handler)
        self.fixtures = fixtures
        self.git_root = git_root.resolve() if git_root else None
        self.profile = profile
        self.limiter = limiter or Limiter(profile.get("bytes_per_second", 0))
        self.origin = origin
        self.lock = threading.Lock()
        self.attempts: Counter[tuple[str, str]] = Counter()
        self.events: list[dict] = []
        self.active = 0
        self.connection_count = 0
        self.epoch = time.perf_counter()

    def connection_opened(self) -> int:
        with self.lock:
            self.connection_count += 1
            return self.connection_count

    def server_bind(self) -> None:
        if self.address_family == socket.AF_UNIX:
            self.socket.bind(self.server_address)
            self.server_name, self.server_port = "localhost", 0
        else:
            super().server_bind()

    @property
    def url(self) -> str:
        return self.public_url or f"http://127.0.0.1:{self.server_port}"

    def begin(self, method: str, path: str, *, attempt_key: str | None = None) -> dict:
        with self.lock:
            key = method, attempt_key or path
            self.attempts[key] += 1
            self.active += 1
            return {
                "origin": self.origin,
                "method": method,
                "path": path,
                "attempt": self.attempts[key],
                "start": time.perf_counter() - self.epoch,
                "active": self.active,
                "bytes": 0,
            }

    def end(self, event: dict) -> None:
        with self.lock:
            event["end"] = time.perf_counter() - self.epoch
            self.active -= 1
            self.events.append(event)

    def wait_idle(self) -> None:
        """Finish recording responses after a timed client has disconnected."""
        deadline = time.monotonic() + 30
        while True:
            with self.lock:
                if not self.active:
                    return
            if time.monotonic() >= deadline:
                raise TimeoutError("Fixture responses did not finish")
            time.sleep(0.01)

    def reset(self, limiter: Limiter | None = None, epoch: float | None = None) -> None:
        self.wait_idle()
        with self.lock:
            self.attempts.clear()
            self.events.clear()
            self.limiter = limiter or Limiter(self.profile.get("bytes_per_second", 0))
            self.epoch = epoch if epoch is not None else time.perf_counter()


class Replay:
    """One index and optional artifact origins behind a shared bottleneck."""

    def __init__(
        self,
        fixtures: Fixtures,
        profile: dict,
        socket_path: Path | None = None,
        *,
        git_root: Path | None = None,
    ) -> None:
        origins = profile.get("artifact_origins", {})
        if origins and socket_path is not None:
            raise ValueError("Artifact origins cannot currently use the HTTP/2 proxy")
        self.profile = {
            key: value for key, value in profile.items() if key != "artifact_origins"
        }
        limiter = Limiter(self.profile.get("bytes_per_second", 0))
        self.servers: dict[str, Server] = {}
        self.threads: list[threading.Thread] = []
        self.file_urls: dict[str, str] = {}
        try:
            for name, config in origins.items():
                if name == "index" or not re.fullmatch(r"[a-z][a-z0-9_-]*", name):
                    raise ValueError(f"Invalid artifact origin name: {name}")
                origin_profile = self.profile | config.get("profile", {})
                if origin_profile.get("bytes_per_second", 0) != limiter.rate:
                    raise ValueError(
                        "Artifact origins must share the root bandwidth limit"
                    )
                server = Server(fixtures, origin_profile, limiter=limiter, origin=name)
                self.servers[name] = server
                for filename in config["filenames"]:
                    if filename not in fixtures.files or filename in self.file_urls:
                        raise ValueError(f"Unknown or duplicate artifact: {filename}")
                    self.file_urls[filename] = server.url + f"/files/{filename}"
            self.main = Server(
                fixtures.with_file_urls(self.file_urls) if origins else fixtures,
                self.profile,
                socket_path,
                git_root=git_root,
                limiter=limiter,
            )
            self.servers["index"] = self.main
            epoch = time.perf_counter()
            for server in self.servers.values():
                server.epoch = epoch
        except BaseException:
            for server in self.servers.values():
                server.server_close()
            raise

    @property
    def urls(self) -> dict[str, str]:
        return {name: server.url for name, server in self.servers.items()}

    @property
    def events(self) -> list[dict]:
        return sorted(
            (event for server in self.servers.values() for event in server.events),
            key=lambda event: event["start"],
        )

    def start(self) -> None:
        for server in self.servers.values():
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.threads.append(thread)

    def wait_idle(self) -> None:
        for server in self.servers.values():
            server.wait_idle()

    def reset(self) -> None:
        self.wait_idle()
        limiter = Limiter(self.profile.get("bytes_per_second", 0))
        epoch = time.perf_counter()
        for server in self.servers.values():
            server.reset(limiter, epoch)

    def stop(self) -> None:
        for server in self.servers.values():
            server.shutdown()
        self.wait_idle()
        for server in self.servers.values():
            server.server_close()
        for thread in self.threads:
            thread.join()


class Handler(BaseHTTPRequestHandler):
    def date_time_string(self, timestamp=None):
        return self.server.profile.get("response_date") or super().date_time_string(
            timestamp
        )

    protocol_version = "HTTP/1.1"
    server: Server

    def setup(self) -> None:
        super().setup()
        self.connection_id = self.server.connection_opened()
        self.connection_start = time.perf_counter()
        if self.connection.family != socket.AF_UNIX:
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        time.sleep(self.server.profile.get("connection_latency_ms", 0) / 1000)

    def log_message(self, format: str, *args: object) -> None:
        pass

    def handle(self) -> None:
        try:
            super().handle()
        except (BrokenPipeError, ConnectionResetError):
            # A measured client may cancel speculative requests or close an idle connection.
            pass

    def do_HEAD(self) -> None:
        self.respond(head=True)

    def do_GET(self) -> None:
        self.respond(head=False)

    def do_POST(self) -> None:
        if self.server.git_root and urlsplit(self.path).path.startswith("/git/"):
            self.respond(head=False)
        elif (
            getattr(self.server.fixtures, "osv", None) is not None
            and urlsplit(self.path).path == "/v1/querybatch"
        ):
            self.respond_osv()
        else:
            self.send_error(405)

    def respond_osv(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if self.headers.get("Transfer-Encoding") or not 0 < length <= 1024**2:
            self.close_connection = True
            self.send_error(413, "OSV replay requires a bounded request body")
            return
        request = self.rfile.read(length)
        try:
            body, query = osv_query_response(self.server.fixtures.osv, request)
        except (KeyError, TypeError, ValueError):
            self.send_error(400, "Request does not match the pinned OSV fixture")
            return
        profile = self.server.profile
        event = self.server.begin(
            "POST", "/v1/querybatch", attempt_key=query["query_sha256"]
        )
        event.update(
            query,
            origin_connection=self.connection_id,
            connection_opened=self.connection_start - self.server.epoch,
            request_bytes=len(request),
        )
        seed = f"{profile.get('seed', 1)}:{query['query_sha256']}:{event['attempt']}"
        jitter = random.Random(seed).uniform(-1, 1) * profile.get("jitter_ms", 0)
        latency = profile.get("osv_query_latency_ms", {}).get(
            query["first_query"], profile.get("latency_ms", 0)
        )
        try:
            time.sleep(max(0, latency + jitter) / 1000)
            failure = profile.get("osv_failures", {}).get(query["first_query"], {})
            status = 200
            if failure.get("status") and event["attempt"] <= failure.get("count", 0):
                status, body = failure["status"], b"Injected transient failure"
            event.update(status=status, response_length=len(body))
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.send_body(io.BytesIO(body).read, len(body), 0, event)
        except (BrokenPipeError, ConnectionResetError) as error:
            event["client_disconnect"] = type(error).__name__
        finally:
            self.server.end(event)

    def respond(self, *, head: bool) -> None:
        path = unquote(urlsplit(self.path).path)
        event = self.server.begin(self.command, path)
        event["origin_connection"] = self.connection_id
        event["connection_opened"] = self.connection_start - self.server.epoch
        event["range"] = self.headers.get("Range")
        event["if_none_match"] = self.headers.get("If-None-Match")
        event["if_range"] = self.headers.get("If-Range")
        profile = self.server.profile
        seed = f"{profile.get('seed', 1)}:{self.command}:{path}:{event['attempt']}"
        jitter = random.Random(seed).uniform(-1, 1) * profile.get("jitter_ms", 0)
        latency = profile.get("path_latency_ms", {}).get(
            path, profile.get("latency_ms", 0)
        )
        time.sleep(max(0, latency + jitter) / 1000)
        try:
            if self.server.git_root and path.startswith("/git/"):
                self.respond_git(path, event, head=head)
                return
            route = profile.get("path_aliases", {}).get(path, path)
            parts = route.strip("/").split("/")
            body: bytes | Path = b"Not found"
            status = 404
            content_type = "text/plain"
            if len(parts) == 2 and parts[0] == "simple":
                if (
                    value := self.server.fixtures.simple.get(normalize(parts[1]))
                ) is not None:
                    body, status = value, 200
                    content_type = "application/vnd.pypi.simple.v1+json"
            elif len(parts) == 2 and parts[0] == "flat":
                body, status = self.server.fixtures.flat, 200
                content_type = "text/html"
            elif len(parts) == 2 and parts[0] == "files":
                if (
                    value := self.server.fixtures.metadata.get(parts[1])
                ) is not None or (
                    value := self.server.fixtures.files.get(parts[1])
                ) is not None:
                    body, status = value, 200
                content_type = "application/octet-stream"
            elif (value := self.server.fixtures.routes.get(route)) is not None:
                body, status = value, 200
                content_type = "application/octet-stream"
            is_artifact = isinstance(body, Path)
            failure = profile.get("path_failures", {}).get(
                path,
                {
                    "status": profile.get("fail_status"),
                    "count": profile.get("fail_count", 0),
                },
            )
            retry_after = None
            if failure.get("status") and event["attempt"] <= failure.get("count", 0):
                status, body = failure["status"], b"Injected transient failure"
                if "retry_after" in failure:
                    retry_after = str(failure["retry_after"])
                elif "retry_after_date_seconds" in failure:
                    retry_after = email.utils.formatdate(
                        time.time() + failure["retry_after_date_seconds"], usegmt=True
                    )
            size = body.stat().st_size if isinstance(body, Path) else len(body)
            etag = (
                '"'
                + (
                    self.server.fixtures.hashes[body.name]
                    if isinstance(body, Path)
                    else hashlib.sha256(body).hexdigest()
                )
                + '"'
            )
            modified = profile.get("artifact_last_modified") if is_artifact else None
            if is_artifact and profile.get("artifact_etag") is False:
                etag = None
            elif is_artifact and profile.get("artifact_etag") == "weak":
                etag = "W/" + etag
            if status == 200 and any(
                tag.strip().removeprefix("W/")
                in {etag.removeprefix("W/") if etag else None, "*"}
                for tag in (event["if_none_match"] or "").split(",")
            ):
                status = 304
            start, end = 0, size - 1
            if (
                is_artifact
                and status == 200
                and event["range"]
                and (
                    event["if_range"] is None
                    or event["if_range"] == (etag if strong_etag(etag) else None)
                    or (modified is not None and event["if_range"] == modified)
                )
                and profile.get("ranges", True)
            ):
                match = re.fullmatch(r"bytes=(\d*)-(\d*)", event["range"])
                if not match or not any(match.groups()):
                    status = 416
                else:
                    first, last = match.groups()
                    if first:
                        start = int(first)
                        end = min(int(last), size - 1) if last else size - 1
                    else:
                        start = max(0, size - int(last))
                    status = 206 if 0 <= start <= end < size else 416
                if status == 416:
                    body, start, end = b"", 0, -1
            length = end - start + 1
            event.update(status=status, response_start=start, response_length=length)
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(length if status != 304 else 0))
            self.send_header(
                "Cache-Control", profile.get("cache_control", "public, max-age=3600")
            )
            if etag is not None:
                self.send_header("ETag", etag)
            if modified is not None:
                self.send_header("Last-Modified", modified)
            if retry_after is not None:
                self.send_header("Retry-After", retry_after)
                event["retry_after"] = retry_after
            if (
                is_artifact
                and profile.get("ranges", True)
                and (not head or profile.get("head_ranges", True))
            ):
                self.send_header("Accept-Ranges", "bytes")
            if status == 206:
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            elif status == 416:
                self.send_header("Content-Range", f"bytes */{size}")
            self.end_headers()
            if head or status == 304:
                return
            cut = (
                profile.get("cut_after_bytes", 0)
                if is_artifact and event["attempt"] <= profile.get("cut_count", 0)
                else 0
            )
            if isinstance(body, Path):
                with body.open("rb") as source:
                    source.seek(start)
                    self.send_body(source.read, length, cut, event)
            else:
                source = io.BytesIO(body[start : end + 1])
                self.send_body(source.read, length, 0, event)
        except (BrokenPipeError, ConnectionResetError) as error:
            event["client_disconnect"] = type(error).__name__
        finally:
            self.server.end(event)

    def respond_git(self, path: str, event: dict, *, head: bool) -> None:
        """Serve the read-only smart HTTP protocol through Git's own CGI backend."""
        query = urlsplit(self.path).query
        match = re.fullmatch(
            r"/git/([A-Za-z0-9_-]+\.git)/(info/refs|git-upload-pack)", path
        )
        if not match or (self.command, match[2], query) not in {
            ("GET", "info/refs", "service=git-upload-pack"),
            ("HEAD", "info/refs", "service=git-upload-pack"),
            ("POST", "git-upload-pack", ""),
        }:
            event["status"] = 404
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if self.headers.get("Transfer-Encoding") or not 0 <= length <= 10 * 1024**2:
            event["status"] = 413
            self.send_error(413, "Git replay requires a bounded request body")
            return
        request = self.rfile.read(length)
        event.update(
            request_bytes=len(request),
            request_sha256=hashlib.sha256(request).hexdigest(),
            git_protocol=self.headers.get("Git-Protocol"),
        )
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        env.update(
            GIT_PROJECT_ROOT=str(self.server.git_root),
            GIT_HTTP_EXPORT_ALL="1",
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            PATH_INFO=f"/{match[1]}/{match[2]}",
            QUERY_STRING=query,
            REQUEST_METHOD=self.command,
            CONTENT_TYPE=self.headers.get("Content-Type", ""),
            CONTENT_LENGTH=str(length),
            HTTP_CONTENT_ENCODING=self.headers.get("Content-Encoding", ""),
            HTTP_GIT_PROTOCOL=self.headers.get("Git-Protocol", ""),
            REMOTE_ADDR="127.0.0.1",
        )
        response = subprocess.run(
            ["git", "-c", "http.receivepack=false", "http-backend"],
            input=request,
            env=env,
            capture_output=True,
            timeout=60,
            check=True,
        ).stdout
        headers, separator, body = response.partition(b"\r\n\r\n")
        if not separator:
            raise ValueError("Git HTTP backend returned invalid CGI headers")
        headers = email.parser.BytesParser().parsebytes(headers, headersonly=True)
        status = int(headers.get("Status", "200").split()[0])
        event.update(status=status, response_length=len(body))
        self.send_response(status)
        for key, value in headers.items():
            if key.lower() not in {"status", "content-length"}:
                self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if not head:
            self.send_body(io.BytesIO(body).read, len(body), 0, event)

    def send_body(self, read, length: int, cut: int, event: dict) -> None:
        while length:
            remaining = min(length, 8192)
            if cut:
                remaining = min(remaining, cut - event["bytes"])
            chunk = read(remaining)
            if not chunk:
                raise EOFError("Fixture body ended early")
            self.server.limiter.wait(len(chunk))
            self.wfile.write(chunk)
            event.setdefault("first_body", time.perf_counter() - self.server.epoch)
            event["bytes"] += len(chunk)
            length -= len(chunk)
            if cut and event["bytes"] >= cut and length:
                event["injected_disconnect"] = True
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                return


def percentile(values: list[float], fraction: float) -> float:
    values = sorted(values)
    index = (len(values) - 1) * fraction
    lower = math.floor(index)
    upper = math.ceil(index)
    return values[lower] + (values[upper] - values[lower]) * (index - lower)


def summary(pairs: list[dict], samples: int = 10000) -> dict:
    ratios = [p["head"]["seconds"] / p["parent"]["seconds"] for p in pairs]
    rng = random.Random(20260929)
    bootstrap = [
        statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(samples)
    ]
    return {
        "pairs": len(pairs),
        "parent_median_seconds": statistics.median(
            p["parent"]["seconds"] for p in pairs
        ),
        "head_median_seconds": statistics.median(p["head"]["seconds"] for p in pairs),
        "median_paired_ratio": statistics.median(ratios),
        "ratio_95ci": [percentile(bootstrap, 0.025), percentile(bootstrap, 0.975)],
        "bootstrap_resamples": samples,
        "qualifies_5_percent": percentile(bootstrap, 0.975) <= 0.95,
    }


def calibrate(fixtures: Fixtures, profile: dict, output: Path) -> None:
    """Measure a known transfer independently of uv's request scheduler."""
    filename, file = max(
        fixtures.files.items(), key=lambda item: item[1].stat().st_size
    )
    length = min(file.stat().st_size, 256 * 1024)
    with file.open("rb") as source:
        expected = source.read(length)
    server = Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"{server.url}/files/{filename}"
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def transfer(_: int) -> None:
        request = urllib.request.Request(
            url, headers={"Range": f"bytes=0-{length - 1}"}
        )
        with loopback.open(request, timeout=60) as response:
            if response.read() != expected:
                raise ValueError("Calibration bytes differ from the pinned fixture")

    try:
        start = time.perf_counter()
        with loopback.open(
            urllib.request.Request(url, method="HEAD"), timeout=60
        ) as response:
            response.read()
        head_seconds = time.perf_counter() - start
        server.reset()
        start = time.perf_counter()
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(transfer, range(2)))
        transfer_seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    data = {
        "profile": profile,
        "netem": netem_profile(),
        "head_seconds": head_seconds,
        "transfer_bytes": length * 2,
        "transfer_seconds": transfer_seconds,
        "effective_bytes_per_second": length * 2 / transfer_seconds,
        "application_model_seconds": profile.get("latency_ms", 0) / 1000
        + (
            length * 2 / profile["bytes_per_second"]
            if profile.get("bytes_per_second")
            else 0
        ),
        "events": server.events,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in data.items() if key != "events"}, indent=2
        )
    )


def strong_etag(value: str | None) -> bool:
    return (
        value is not None
        and len(value) >= 2
        and value.startswith('"')
        and value.endswith('"')
    )


def response_validator(headers) -> tuple[str, str] | None:
    """Choose a validator safe for a later If-Range request."""
    etag = headers.get("ETag")
    if etag is not None:
        return ("ETag", etag) if strong_etag(etag) else None
    modified = headers.get("Last-Modified")
    date = headers.get("Date")
    if modified is not None and date is not None:
        try:
            age = email.utils.parsedate_to_datetime(
                date
            ) - email.utils.parsedate_to_datetime(modified)
        except (TypeError, ValueError):
            return None
        if age.total_seconds() >= 60:
            return ("Last-Modified", modified)
    return None


def read_resumable(loopback, url: str, size: int) -> bytes:
    """Immediately resume a pinned artifact after up to 32 interrupted responses."""
    body = bytearray()
    validator = None
    for _ in range(33):
        headers = {"Accept-Encoding": "identity"}
        if body and validator:
            headers["Range"] = f"bytes={len(body)}-"
            headers["If-Range"] = validator[1]
        try:
            with loopback.open(
                urllib.request.Request(url, headers=headers), timeout=60
            ) as response:
                if response.status == 200:
                    body.clear()
                elif response.status == 206:
                    content_range = re.fullmatch(
                        r"bytes (\d+)-(\d+)/(\d+)",
                        response.headers.get("Content-Range", ""),
                    )
                    if (
                        content_range is None
                        or int(content_range[1]) != len(body)
                        or not int(content_range[1]) <= int(content_range[2]) < size
                        or int(content_range[3]) != size
                    ):
                        raise ValueError("Oracle received an inconsistent range")
                else:
                    raise ValueError(f"Unexpected oracle response: {response.status}")
                if (
                    validator is not None
                    and response.headers.get(validator[0]) != validator[1]
                ):
                    raise ValueError("Oracle artifact changed while resuming")
                if validator is None:
                    validator = response_validator(response.headers)
                try:
                    body.extend(response.read())
                except http.client.IncompleteRead as error:
                    body.extend(error.partial)
        except urllib.error.HTTPError as error:
            if error.code not in {429, 500, 502, 503, 504}:
                raise
        if len(body) == size:
            return bytes(body)
        if len(body) > size:
            raise ValueError("Oracle received too many bytes")
    raise RuntimeError("Oracle exhausted its interruption limit")


def oracle(
    fixtures: Fixtures, profile: dict, filenames: list[str], route: str, output: Path
) -> None:
    """Fetch a known artifact set with perfect dependency foreknowledge."""
    if route == "metadata" and not profile.get("pep658", True):
        raise ValueError("The metadata oracle requires PEP 658 in the selected profile")
    raw = route in {"raw", "raw-resume"}
    names = {}
    for name, files in fixtures.packages.items():
        for file in files:
            if file["filename"] in filenames:
                names[file["filename"]] = name
    if not set(filenames).issubset(fixtures.files) or (
        not raw and set(names) != set(filenames)
    ):
        raise ValueError("Every oracle filename must occur in the fixture manifest")
    server = Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def read(path: str) -> bytes:
        with loopback.open(server.url + path, timeout=60) as response:
            return response.read()

    def fetch(filename: str) -> None:
        if raw:
            artifact = {
                "url": f"/files/{filename}",
                "size": fixtures.files[filename].stat().st_size,
            }
        else:
            index = json.loads(read(f"/simple/{names[filename]}/"))
            artifact = next(
                file for file in index["files"] if file["filename"] == filename
            )
        suffix = ".metadata" if route == "metadata" else ""
        body = (
            read_resumable(loopback, server.url + artifact["url"], artifact["size"])
            if route in {"resume", "raw-resume"}
            else read(artifact["url"] + suffix)
        )
        expected = (
            fixtures.metadata[filename + ".metadata"]
            if suffix
            else fixtures.files[filename].read_bytes()
        )
        if body != expected:
            raise ValueError(f"Oracle bytes differ: {filename}")

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=max(1, len(filenames))
        ) as pool:
            list(pool.map(fetch, filenames))
        seconds = time.perf_counter() - start
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    index_bytes = (
        0 if raw else sum(len(fixtures.simple[name]) for name in set(names.values()))
    )
    artifact_bytes = index_bytes + sum(
        fixtures.files[filename].stat().st_size for filename in filenames
    )
    required_bytes = (
        artifact_bytes
        if raw
        else index_bytes
        + sum(len(fixtures.metadata[filename + ".metadata"]) for filename in filenames)
    )
    waves = 1 if raw else 2
    floor = network_floor(profile, required_bytes, waves)
    data = {
        "profile": profile,
        "netem": netem_profile(),
        "filenames": filenames,
        "route": route,
        "seconds": seconds,
        "required_metadata_and_index_bytes": None if raw else required_bytes,
        "required_payload_bytes": required_bytes,
        "optimistic_network_floor_seconds": floor,
        "required_artifact_and_index_bytes": artifact_bytes,
        "optimistic_artifact_transfer_floor_seconds": network_floor(
            profile, artifact_bytes, waves
        ),
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "A known dependency graph with unlimited request concurrency. Full-artifact transfer is a realizable strategy, not a minimum-byte metadata claim. Raw routes fetch known artifact URLs without index metadata. Resume routes retry immediately and verify the complete artifact; they exclude resolution and installation.",
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in data.items() if key != "events"}, indent=2
        )
    )


class Http2Proxy:
    """Expose the replay origin through a local Caddy TLS/HTTP2 listener."""

    def __init__(self, work: Path, binary: Path, certificate: Path, key: Path) -> None:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        self.url = f"https://127.0.0.1:{port}"
        self.log_path = work / "caddy.log"
        self.process: subprocess.Popen | None = None
        config = {
            "admin": {"disabled": True},
            "apps": {
                "tls": {
                    "certificates": {
                        "load_files": [
                            {"certificate": str(certificate), "key": str(key)}
                        ]
                    }
                },
                "http": {
                    "servers": {
                        "benchmark": {
                            "listen": [f"127.0.0.1:{port}"],
                            "protocols": ["h1", "h2"],
                            "automatic_https": {"disable": True},
                            "tls_connection_policies": [{}],
                            "logs": {},
                            "routes": [
                                {
                                    "handle": [
                                        {
                                            "handler": "reverse_proxy",
                                            "upstreams": [
                                                {"dial": f"unix/{work / 'origin.sock'}"}
                                            ],
                                        }
                                    ]
                                }
                            ],
                        }
                    }
                },
            },
        }
        config_path = work / "caddy.json"
        config_path.write_text(json.dumps(config))
        env = os.environ.copy()
        env.update(
            XDG_DATA_HOME=str(work / "caddy-data"),
            XDG_CONFIG_HOME=str(work / "caddy-config"),
        )
        with self.log_path.open("wb") as log:
            self.process = subprocess.Popen(
                [str(binary), "run", "--config", str(config_path)],
                env=env,
                stdout=log,
                stderr=log,
            )
        context = ssl.create_default_context(cafile=str(certificate))
        context.set_alpn_protocols(["h2"])
        deadline = time.monotonic() + 30
        try:
            while True:
                if self.process.poll() is not None:
                    raise RuntimeError(self.log_path.read_text())
                try:
                    with (
                        socket.create_connection(
                            ("127.0.0.1", port), timeout=5
                        ) as sock,
                        context.wrap_socket(sock, server_hostname="127.0.0.1") as tls,
                    ):
                        if tls.selected_alpn_protocol() != "h2":
                            raise RuntimeError("Replay proxy did not negotiate HTTP/2")
                        break
                except (ConnectionRefusedError, TimeoutError):
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.05)
        except BaseException:
            self.stop()
            raise

    def stop(self) -> None:
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()

    def protocols(self) -> dict[str, int]:
        protocols: Counter[str] = Counter()
        for line in self.log_path.read_text().splitlines():
            event = json.loads(line)
            if event.get("logger", "").startswith("http.log.access"):
                protocols[event["request"]["proto"]] += 1
        if not protocols or set(protocols) != {"HTTP/2.0"}:
            raise ValueError(f"Replay requests did not all use HTTP/2: {protocols}")
        return dict(protocols)


def expand(value: str, context: dict) -> str:
    for key, replacement in context.items():
        value = value.replace("{" + key + "}", str(replacement))
    return value


def normalize_output(value: bytes, context: dict) -> bytes:
    origins = {"base": context["base"]} | {
        key: url for key, url in context.items() if key.startswith("origin:")
    }
    for key, url in sorted(
        origins.items(), key=lambda item: len(str(item[1])), reverse=True
    ):
        label = (
            "[INDEX]" if key == "base" else f"[ORIGIN:{key.removeprefix('origin:')}]"
        )
        value = value.replace(str(url).encode(), label.encode())
    return value.replace(str(context["work"]).encode(), b"[WORK]")


def maximum_active(events: list[dict]) -> int:
    active = maximum = 0
    for _, change in sorted(
        point for event in events for point in ((event["start"], 1), (event["end"], -1))
    ):
        active += change
        maximum = max(maximum, active)
    return maximum


def run_one(
    binary: Path, fixtures: Fixtures, profile: dict, args: argparse.Namespace
) -> dict:
    with tempfile.TemporaryDirectory(prefix="trial-", dir=args.work_dir) as directory:
        work = Path(directory)
        replay = Replay(
            fixtures,
            profile,
            work / "origin.sock" if args.http2_proxy else None,
            git_root=args.git_root,
        )
        server = replay.main
        replay.start()
        proxy = None
        try:
            if args.http2_proxy:
                proxy = Http2Proxy(
                    work, args.http2_proxy, args.tls_certificate, args.tls_key
                )
                server.public_url = proxy.url
            context = {
                "base": server.url,
                "index": server.url + "/simple",
                "work": work,
                "python": args.python,
                "fixtures": args.directory.resolve(),
                **{
                    f"origin:{name}": url
                    for name, url in replay.urls.items()
                    if name != "index"
                },
            }
            (work / "requirements.in").write_text(
                "".join(f"{expand(item, context)}\n" for item in args.requirement)
            )
            for name, contents in args.templates.items():
                (work / name).write_text(expand(contents, context))
            configuration = (
                ["--config-file", str(work / "uv.toml")]
                if "uv.toml" in args.templates
                else ["--no-config"]
            )
            prefix = [str(binary), *configuration, "--no-progress", "--color", "never"]
            command = prefix + [expand(arg, context) for arg in args.command]
            env = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith(("UV_", "GIT_"))
                and key
                not in {"VIRTUAL_ENV", "CONDA_PREFIX", "PYTHONPATH", "PYTHONHOME"}
            }
            env.update(
                UV_CACHE_DIR=str(work / "cache"),
                UV_PYTHON=args.python,
                UV_PYTHON_DOWNLOADS="never",
                GIT_CONFIG_NOSYSTEM="1",
                GIT_CONFIG_GLOBAL=os.devnull,
                GIT_TERMINAL_PROMPT="0",
                NO_PROXY="127.0.0.1,localhost",
                no_proxy="127.0.0.1,localhost",
            )
            env.update({key: expand(value, context) for key, value in args.env.items()})
            if proxy:
                env["SSL_CERT_FILE"] = str(args.tls_certificate)
            for setup in args.setup_commands:
                subprocess.run(
                    prefix + [expand(arg, context) for arg in setup],
                    cwd=work,
                    env=env,
                    capture_output=True,
                    timeout=args.timeout,
                    check=True,
                )
            if args.setup_commands:
                replay.reset()
            if args.cache_mode != "cold":
                subprocess.run(
                    command,
                    cwd=work,
                    env=env,
                    capture_output=True,
                    timeout=args.timeout,
                    check=True,
                )
                replay.reset()
            if args.cache_mode == "refresh":
                command.append("--refresh")
            start = time.perf_counter()
            result = subprocess.run(
                command,
                cwd=work,
                env=env,
                capture_output=True,
                timeout=args.timeout,
                check=False,
            )
            seconds = time.perf_counter() - start
        finally:
            if proxy:
                proxy.stop()
            replay.stop()
        output = normalize_output(result.stdout, context)
        stderr = normalize_output(result.stderr, context)
        if result.returncode:
            raise RuntimeError(
                f"Command failed ({result.returncode}): {command}\n{result.stderr.decode(errors='replace')}"
            )
        events = replay.events
        return {
            "seconds": seconds,
            "stdout_sha256": hashlib.sha256(output).hexdigest(),
            "stdout": output.decode(errors="replace"),
            "stderr": result.stderr.decode(errors="replace"),
            "stderr_sha256": hashlib.sha256(stderr).hexdigest(),
            "origins": replay.urls,
            "events": events,
            "bytes": sum(event["bytes"] for event in events),
            "requests": len(events),
            "origin_connections": len(
                {(event["origin"], event["origin_connection"]) for event in events}
            ),
            "max_active": maximum_active(events),
            "frontend_protocols": proxy.protocols() if proxy else None,
            "verified_tree": (
                tree_digest(
                    Path(args.verify_tree.format(work=work)),
                    args.normalize_tree_file,
                    context,
                    args.normalize_tree_symlink,
                )
                if args.verify_tree
                else None
            ),
            "verified_files": {
                name: hashlib.sha256(
                    normalize_output(Path(expand(name, context)).read_bytes(), context)
                ).hexdigest()
                for name in args.verify_file
            },
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=HERE / "fixtures.json")
    parser.add_argument("--directory", type=Path, required=True)
    subparsers = parser.add_subparsers(dest="action", required=True)
    subparsers.add_parser("prepare")
    calibration = subparsers.add_parser("calibrate")
    calibration.add_argument("--profile", default="slow")
    calibration.add_argument("--profiles", type=Path, default=HERE / "profiles.json")
    calibration.add_argument("--output", type=Path, required=True)
    reference = subparsers.add_parser("oracle")
    reference.add_argument("--profile", default="slow")
    reference.add_argument("--profiles", type=Path, default=HERE / "profiles.json")
    reference.add_argument("--filename", action="append", required=True)
    reference.add_argument(
        "--route",
        choices=["metadata", "wheel", "resume", "raw", "raw-resume"],
        default="metadata",
    )
    reference.add_argument("--output", type=Path, required=True)
    run = subparsers.add_parser("run")
    run.add_argument("--parent", type=Path, required=True)
    run.add_argument("--head", type=Path, required=True)
    run.add_argument("--parent-sha", required=True)
    run.add_argument("--head-sha", required=True)
    run.add_argument("--profile", default="slow")
    run.add_argument("--profiles", type=Path, default=HERE / "profiles.json")
    run.add_argument("--work-dir", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--python", default="3.12")
    run.add_argument("--requirement", action="append", default=[])
    run.add_argument("--verify-tree", help="Directory to compare after each command")
    run.add_argument(
        "--normalize-tree-file",
        action="append",
        default=[],
        help="Glob of files whose trial URL and directory are normalized before hashing",
    )
    run.add_argument(
        "--normalize-tree-symlink",
        action="append",
        default=[],
        help="Glob of symlinks whose absolute target beneath the trial directory is normalized",
    )
    run.add_argument("--verify-file", action="append", default=[])
    run.add_argument("--compare-stderr", action="store_true")
    run.add_argument("--config-template", type=Path)
    run.add_argument("--project-template", type=Path)
    run.add_argument("--lock-template", type=Path)
    run.add_argument("--pylock-template", type=Path)
    run.add_argument("--git-root", type=Path, help="Directory of bare Git fixtures")
    run.add_argument(
        "--setup-commands", type=Path, help="JSON array of uv argument arrays"
    )
    run.add_argument("--env", action="append", default=[], metavar="KEY=VALUE")
    run.add_argument("--http2-proxy", type=Path, help="Path to the Caddy binary")
    run.add_argument("--tls-certificate", type=Path)
    run.add_argument("--tls-key", type=Path)
    run.add_argument("--pairs", type=int, default=20)
    run.add_argument("--warmups", type=int, default=2)
    run.add_argument(
        "--cache-mode", choices=["cold", "warm", "refresh"], default="cold"
    )
    run.add_argument("--timeout", type=float, default=300)
    run.add_argument("--required-bytes", type=int, default=0)
    run.add_argument("--required-waves", type=int, default=0)
    run.add_argument("--required-latency-ms", type=float)
    run.add_argument("--required-wait-ms", type=float, default=0)
    run.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.manifest, args.directory)
        return
    if args.action == "calibrate":
        profile = json.loads(args.profiles.read_text())[args.profile]
        calibrate(
            Fixtures(args.manifest, args.directory, profile.get("pep658", True)),
            profile,
            args.output,
        )
        return
    if args.action == "oracle":
        profile = json.loads(args.profiles.read_text())[args.profile]
        oracle(
            Fixtures(args.manifest, args.directory, profile.get("pep658", True)),
            profile,
            args.filename,
            args.route,
            args.output,
        )
        return
    if args.command[:1] == ["--"]:
        args.command.pop(0)
    if not args.command or args.pairs < 2:
        parser.error("provide a command and at least two pairs")
    if args.required_latency_ms is not None and args.required_latency_ms < 0:
        parser.error("--required-latency-ms cannot be negative")
    if args.required_wait_ms < 0:
        parser.error("--required-wait-ms cannot be negative")
    args.templates = {
        name: path.read_text()
        for name, path in (
            ("uv.toml", args.config_template),
            ("pyproject.toml", args.project_template),
            ("uv.lock", args.lock_template),
            ("pylock.toml", args.pylock_template),
        )
        if path is not None
    }
    args.setup_commands = (
        json.loads(args.setup_commands.read_text()) if args.setup_commands else []
    )
    if not isinstance(args.setup_commands, list) or any(
        not isinstance(command, list)
        or not command
        or any(not isinstance(arg, str) for arg in command)
        for command in args.setup_commands
    ):
        parser.error(
            "--setup-commands must contain an array of nonempty argument arrays"
        )
    if any("=" not in item or not item.partition("=")[0] for item in args.env):
        parser.error("--env requires KEY=VALUE")
    args.env = dict(item.split("=", 1) for item in args.env)
    reserved_environment = {
        "UV_CACHE_DIR",
        "UV_PYTHON",
        "UV_BENCH_NETEM",
        "NO_PROXY",
        "no_proxy",
        "SSL_CERT_FILE",
    }
    if reserved_environment.intersection(args.env):
        parser.error("--env cannot override benchmark isolation settings")
    if args.http2_proxy:
        if not args.tls_certificate or not args.tls_key:
            parser.error("--http2-proxy requires --tls-certificate and --tls-key")
        args.http2_proxy = args.http2_proxy.resolve()
        args.tls_certificate = args.tls_certificate.resolve()
        args.tls_key = args.tls_key.resolve()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = Fixtures(args.manifest, args.directory, profile.get("pep658", True))
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    binaries = {"parent": args.parent.resolve(), "head": args.head.resolve()}
    data = {
        "parent_sha": args.parent_sha,
        "head_sha": args.head_sha,
        "binaries": {
            name: {
                "path": str(path),
                "sha256": digest(path),
                "version": subprocess.check_output(
                    [path, "--version"], text=True
                ).strip(),
            }
            for name, path in binaries.items()
        },
        "manifest_sha256": digest(args.manifest),
        "git_repositories": (
            {
                path.name: subprocess.check_output(
                    ["git", "--git-dir", str(path), "show-ref", "--head"], text=True
                ).splitlines()
                for path in sorted(args.git_root.resolve().glob("*.git"))
            }
            if args.git_root
            else None
        ),
        "git_version": (
            subprocess.check_output(["git", "--version"], text=True).strip()
            if args.git_root
            else None
        ),
        "profile": profile,
        "netem": netem_profile(),
        "command": args.command,
        "python_request": args.python,
        "python_executable_sha256": (
            digest(Path(args.python)) if Path(args.python).is_file() else None
        ),
        "requirements": args.requirement,
        "templates": args.templates,
        "setup_commands": args.setup_commands,
        "environment_overrides": args.env,
        "verify_tree": args.verify_tree,
        "normalize_tree_file": args.normalize_tree_file,
        "normalize_tree_symlink": args.normalize_tree_symlink,
        "verify_file": args.verify_file,
        "compare_stderr": args.compare_stderr,
        "http2_proxy": (
            {
                "binary": str(args.http2_proxy),
                "version": subprocess.check_output(
                    [args.http2_proxy, "version"], text=True
                ).strip(),
                "sha256": digest(args.http2_proxy),
                "certificate_sha256": digest(args.tls_certificate),
            }
            if args.http2_proxy
            else None
        ),
        "warmups": args.warmups,
        "timeout_seconds": args.timeout,
        "cache_mode": args.cache_mode,
        "pairs": [],
        "lower_bound": {
            "required_bytes": args.required_bytes,
            "required_waves": args.required_waves,
            "required_latency_ms": args.required_latency_ms,
            "required_wait_ms": args.required_wait_ms,
            "seconds": network_floor(
                profile,
                args.required_bytes,
                args.required_waves,
                args.required_latency_ms,
                args.required_wait_ms,
            ),
            "model": "Optimistic maximum of required-body serialization and serial response-latency waves, plus required waits that cannot overlap those transfers; excludes TCP/TLS and CPU costs.",
        },
    }
    for index in range(args.warmups + args.pairs):
        order = ["parent", "head"] if index % 2 == 0 else ["head", "parent"]
        pair = {
            name: run_one(binaries[name], fixtures, profile, args) for name in order
        }
        if pair["parent"]["stdout_sha256"] != pair["head"]["stdout_sha256"]:
            raise ValueError("Parent and head command outputs differ")
        if (
            args.compare_stderr
            and pair["parent"]["stderr_sha256"] != pair["head"]["stderr_sha256"]
        ):
            raise ValueError("Parent and head diagnostic outputs differ")
        if pair["parent"]["verified_tree"] != pair["head"]["verified_tree"]:
            raise ValueError("Parent and head installed file contents differ")
        if pair["parent"]["verified_files"] != pair["head"]["verified_files"]:
            raise ValueError("Parent and head result files differ")
        if index >= args.warmups:
            data["pairs"].append(pair)
            args.output.write_text(json.dumps(data, indent=2) + "\n")
            print(
                f"pair {index - args.warmups + 1}/{args.pairs}: parent={pair['parent']['seconds']:.4f}s head={pair['head']['seconds']:.4f}s",
                flush=True,
            )
    data["summary"] = summary(data["pairs"])
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps(data["summary"], indent=2))


if __name__ == "__main__":
    main()
