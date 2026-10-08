"""Serve reviewed package archives through an owned loopback index.

Keep this module compatible with Python 3.6 for the system-interpreter checks.
"""

import csv
import hashlib
import html
import http.server
import json
import logging
import os
import re
import shutil
import socketserver
import tempfile
import threading
import urllib.parse
import urllib.request
from pathlib import Path

FIXTURES = Path(__file__).resolve().parents[1] / "test/integration/package-fixtures"
LOGGER = logging.getLogger(__name__)


def normalize_name(name):
    return re.sub(r"[-_.]+", "-", name).lower()


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_profiles():
    with (FIXTURES / "profiles.json").open(encoding="utf-8") as source:
        return json.load(source)


class FixtureIndex:
    """Expose only the artifacts admitted by one reviewed workload profile."""

    def __init__(self, profile, cache, offline=False):
        self.profile = profile
        self.cache = Path(cache)
        self.offline = offline
        self.used = {}
        self.requests = []
        self.failures = []
        self._verified = set()
        self._lock = threading.Lock()
        admitted = set(profile["requirements"].items())
        admitted.update(profile["build_requirements"].items())
        self.artifacts = {}
        self.packages = {}
        found = set()
        with (FIXTURES / "artifacts.tsv").open(newline="", encoding="utf-8") as source:
            for artifact in csv.DictReader(source, delimiter="\t"):
                if (artifact["name"], artifact["version"]) not in admitted:
                    continue
                filename = artifact["filename"]
                if (
                    Path(filename).name != filename
                    or filename in ("", ".", "..")
                    or not re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"])
                ):
                    raise ValueError("Invalid artifact identity: " + filename)
                if filename in self.artifacts:
                    raise ValueError("Duplicate artifact filename: " + filename)
                self.artifacts[filename] = artifact
                self.packages.setdefault(artifact["name"], []).append(artifact)
                found.add((artifact["name"], artifact["version"]))
        missing = admitted - found
        if missing:
            raise ValueError(
                "Missing fixture artifacts for: "
                + ", ".join("{}=={}".format(*package) for package in sorted(missing))
            )
        self._artifact_locks = {
            filename: threading.Lock() for filename in self.artifacts
        }
        self._server = None
        self._thread = None

    def _archive(self, artifact):
        filename = artifact["filename"]
        destination = self.cache / artifact["sha256"] / filename
        with self._artifact_locks[filename]:
            if filename in self._verified:
                return destination
            if destination.exists():
                if sha256(destination) != artifact["sha256"]:
                    raise ValueError("Fixture archive hash mismatch: " + filename)
            else:
                if self.offline:
                    raise FileNotFoundError("Unprepared fixture archive: " + filename)
                destination.parent.mkdir(parents=True, exist_ok=True)
                temporary = None
                try:
                    with tempfile.NamedTemporaryFile(
                        dir=str(destination.parent), delete=False
                    ) as target:
                        temporary = Path(target.name)
                        with urllib.request.urlopen(
                            artifact["url"], timeout=60
                        ) as source:
                            shutil.copyfileobj(source, target)
                    if sha256(temporary) != artifact["sha256"]:
                        raise ValueError("Fixture archive hash mismatch: " + filename)
                    os.replace(str(temporary), str(destination))
                finally:
                    if temporary is not None and temporary.exists():
                        temporary.unlink()
            self._verified.add(filename)
            with self._lock:
                self.used[filename] = {
                    "sha256": artifact["sha256"],
                    "size": destination.stat().st_size,
                }
            return destination

    def __enter__(self):
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, message, *args):
                LOGGER.debug(message, *args)

            def do_GET(self):
                self.serve(body=True)

            def do_HEAD(self):
                self.serve(body=False)

            def serve(self, body):
                path = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
                with owner._lock:
                    owner.requests.append([self.command, path])
                if path.startswith("/simple/"):
                    name = normalize_name(path[len("/simple/") :].strip("/"))
                    artifacts = owner.packages.get(name)
                    if not artifacts:
                        self.send_error(404)
                        return
                    links = []
                    for artifact in artifacts:
                        filename = artifact["filename"]
                        url = "/files/" + urllib.parse.quote(filename, safe="")
                        url += "#sha256=" + artifact["sha256"]
                        requires_python = artifact["requires_python"]
                        attribute = (
                            ' data-requires-python="'
                            + html.escape(requires_python, quote=True)
                            + '"'
                            if requires_python
                            else ""
                        )
                        if artifact["yanked"]:
                            attribute += (
                                ' data-yanked="'
                                + html.escape(artifact["yanked_reason"], quote=True)
                                + '"'
                            )
                        links.append(
                            '<a href="'
                            + html.escape(url, quote=True)
                            + '"'
                            + attribute
                            + ">"
                            + html.escape(filename)
                            + "</a>"
                        )
                    payload = ("<!DOCTYPE html>\n" + "\n".join(links)).encode("utf-8")
                    self.send_response(200)
                    self.send_header("Content-Type", "text/html; charset=utf-8")
                    self.send_header("Content-Length", str(len(payload)))
                    self.send_header("Cache-Control", "public, max-age=600")
                    self.end_headers()
                    if body:
                        self.wfile.write(payload)
                    return
                if path.startswith("/files/"):
                    artifact = owner.artifacts.get(path[len("/files/") :])
                    if artifact is not None:
                        try:
                            archive = owner._archive(artifact)
                        except Exception as error:
                            with owner._lock:
                                owner.failures.append(str(error))
                            LOGGER.exception(
                                "Could not prepare %s", artifact["filename"]
                            )
                            self.send_error(503, "Fixture archive unavailable")
                            return
                        size = archive.stat().st_size
                        start, end = 0, size - 1
                        requested_range = self.headers.get("Range")
                        if requested_range:
                            match = re.fullmatch(r"bytes=(\d*)-(\d*)", requested_range)
                            if match is None or not any(match.groups()):
                                self.send_error(416)
                                return
                            first, last = match.groups()
                            if first:
                                start = int(first)
                                end = min(int(last), end) if last else end
                            else:
                                start = max(0, size - int(last))
                            if start > end or start >= size:
                                self.send_error(416)
                                return
                        self.send_response(206 if requested_range else 200)
                        self.send_header("Content-Type", "application/octet-stream")
                        self.send_header("Content-Length", str(end - start + 1))
                        self.send_header("Accept-Ranges", "bytes")
                        self.send_header("Cache-Control", "public, max-age=31536000")
                        if requested_range:
                            self.send_header(
                                "Content-Range", f"bytes {start}-{end}/{size}"
                            )
                        self.end_headers()
                        if body:
                            with archive.open("rb") as source:
                                try:
                                    source.seek(start)
                                    remaining = end - start + 1
                                    while remaining:
                                        chunk = source.read(min(1024 * 1024, remaining))
                                        self.wfile.write(chunk)
                                        remaining -= len(chunk)
                                except (BrokenPipeError, ConnectionResetError):
                                    # Metadata range probes can stop reading a full response.
                                    pass
                        return
                self.send_error(404)

        class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
            daemon_threads = True

        self._server = Server(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self._server.shutdown()
        self._server.server_close()
        self._thread.join()
        if exc_type is None and self.failures:
            raise RuntimeError(
                "Fixture preparation failed: " + "; ".join(self.failures)
            )

    @property
    def url(self):
        return f"http://127.0.0.1:{self._server.server_port}/simple"

    def environment(self, original):
        """Keep the selected loopback index independent of host index configuration."""
        environment = dict(original)
        for name in list(environment):
            upper = name.upper()
            if upper.startswith("UV_INDEX") or upper in {
                "UV_DEFAULT_INDEX",
                "UV_EXTRA_INDEX_URL",
                "UV_FIND_LINKS",
                "UV_NO_INDEX",
                "PIP_INDEX_URL",
                "PIP_EXTRA_INDEX_URL",
                "PIP_FIND_LINKS",
                "PIP_NO_INDEX",
            }:
                del environment[name]
        exclusions = ["localhost", "127.0.0.1", "::1"]
        for name in ("NO_PROXY", "no_proxy"):
            exclusions.extend(environment.get(name, "").split(","))
        bypass = ",".join(dict.fromkeys(value for value in exclusions if value))
        environment["NO_PROXY"] = bypass
        environment["no_proxy"] = bypass
        return environment

    def constraints(self, directory):
        paths = []
        for key in ("requirements", "build_requirements"):
            path = Path(directory) / (key + ".txt")
            path.write_text(
                "".join(
                    f"{name}=={version}\n"
                    for name, version in sorted(self.profile[key].items())
                ),
                encoding="utf-8",
            )
            paths.append(path)
        return paths

    def identity(self):
        return {
            "profile": self.profile,
            "catalog_sha256": sha256(FIXTURES / "artifacts.tsv"),
            "artifacts": dict(sorted(self.used.items())),
            "requests": self.requests,
        }
