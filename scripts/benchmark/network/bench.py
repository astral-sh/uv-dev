"""Replay pinned wheels under controlled network conditions and compare uv binaries."""

from __future__ import annotations

import argparse
import concurrent.futures
import email.parser
import hashlib
import io
import json
import math
import os
import random
import re
import shutil
import socket
import statistics
import subprocess
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


def normalize(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def digest(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


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


class Fixtures:
    def __init__(self, manifest: Path, directory: Path, pep658: bool) -> None:
        self.files: dict[str, Path] = {}
        self.metadata: dict[str, bytes] = {}
        self.hashes: dict[str, str] = {}
        self.packages: dict[str, list[dict]] = {}
        for item in json.loads(manifest.read_text()):
            path = directory / item["filename"]
            if digest(path) != item["sha256"]:
                raise ValueError(f"Fixture hash mismatch: {path}")
            with zipfile.ZipFile(path) as wheel:
                names = [
                    n for n in wheel.namelist() if n.endswith(".dist-info/METADATA")
                ]
                if len(names) != 1:
                    raise ValueError(f"Expected one METADATA file: {path}")
                metadata = wheel.read(names[0])
            headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
            self.files[path.name] = path
            self.hashes[path.name] = item["sha256"]
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
                        if pep658
                        else False
                    ),
                }
            )
        self.simple = {
            name: json.dumps(
                {"name": name, "meta": {"api-version": "1.4"}, "files": files},
                separators=(",", ":"),
            ).encode()
            for name, files in self.packages.items()
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

    def __init__(self, fixtures: Fixtures, profile: dict) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.fixtures = fixtures
        self.profile = profile
        self.limiter = Limiter(profile.get("bytes_per_second", 0))
        self.lock = threading.Lock()
        self.attempts: Counter[tuple[str, str]] = Counter()
        self.events: list[dict] = []
        self.active = 0
        self.epoch = time.perf_counter()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def begin(self, method: str, path: str) -> dict:
        with self.lock:
            key = method, path
            self.attempts[key] += 1
            self.active += 1
            return {
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

    def reset(self) -> None:
        deadline = time.monotonic() + 30
        while True:
            with self.lock:
                if not self.active:
                    self.attempts.clear()
                    self.events.clear()
                    self.limiter = Limiter(self.profile.get("bytes_per_second", 0))
                    self.epoch = time.perf_counter()
                    return
            if time.monotonic() >= deadline:
                raise TimeoutError("Fixture responses did not finish")
            time.sleep(0.01)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: Server

    def setup(self) -> None:
        super().setup()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    def log_message(self, format: str, *args: object) -> None:
        pass

    def do_HEAD(self) -> None:
        self.respond(head=True)

    def do_GET(self) -> None:
        self.respond(head=False)

    def respond(self, *, head: bool) -> None:
        path = unquote(urlsplit(self.path).path)
        event = self.server.begin(self.command, path)
        event["range"] = self.headers.get("Range")
        profile = self.server.profile
        seed = f"{profile.get('seed', 1)}:{self.command}:{path}:{event['attempt']}"
        jitter = random.Random(seed).uniform(-1, 1) * profile.get("jitter_ms", 0)
        time.sleep(max(0, profile.get("latency_ms", 0) + jitter) / 1000)
        try:
            parts = path.strip("/").split("/")
            body: bytes | Path = b"Not found"
            status = 404
            content_type = "text/plain"
            if len(parts) == 2 and parts[0] == "simple":
                if (
                    value := self.server.fixtures.simple.get(normalize(parts[1]))
                ) is not None:
                    body, status = value, 200
                    content_type = "application/vnd.pypi.simple.v1+json"
            elif len(parts) == 2 and parts[0] == "files":
                if (
                    value := self.server.fixtures.metadata.get(parts[1])
                ) is not None or (
                    value := self.server.fixtures.files.get(parts[1])
                ) is not None:
                    body, status = value, 200
                content_type = "application/octet-stream"
            is_artifact = isinstance(body, Path)
            if profile.get("fail_status") and event["attempt"] <= profile.get(
                "fail_count", 0
            ):
                status, body = profile["fail_status"], b"Injected transient failure"
            size = body.stat().st_size if isinstance(body, Path) else len(body)
            start, end = 0, size - 1
            if (
                is_artifact
                and status == 200
                and event["range"]
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
            etag = (
                '"'
                + (
                    self.server.fixtures.hashes[body.name]
                    if isinstance(body, Path)
                    else hashlib.sha256(body).hexdigest()
                )
                + '"'
            )
            if status == 200 and self.headers.get("If-None-Match") == etag:
                status = 304
            length = end - start + 1
            event.update(status=status, response_start=start, response_length=length)
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(length if status != 304 else 0))
            self.send_header(
                "Cache-Control", profile.get("cache_control", "public, max-age=3600")
            )
            self.send_header("ETag", etag)
            if is_artifact and profile.get("ranges", True):
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

    def transfer(_: int) -> None:
        request = urllib.request.Request(
            url, headers={"Range": f"bytes=0-{length - 1}"}
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            if response.read() != expected:
                raise ValueError("Calibration bytes differ from the pinned fixture")

    try:
        start = time.perf_counter()
        with urllib.request.urlopen(
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


def oracle(
    fixtures: Fixtures, profile: dict, filenames: list[str], route: str, output: Path
) -> None:
    """Fetch a known artifact set with perfect dependency foreknowledge."""
    if route == "metadata" and not profile.get("pep658", True):
        raise ValueError("The metadata oracle requires PEP 658 in the selected profile")
    names = {}
    for name, files in fixtures.packages.items():
        for file in files:
            if file["filename"] in filenames:
                names[file["filename"]] = name
    if set(names) != set(filenames):
        raise ValueError("Every oracle filename must occur in the fixture manifest")
    server = Server(fixtures, profile)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def read(path: str) -> bytes:
        with urllib.request.urlopen(server.url + path, timeout=60) as response:
            return response.read()

    def fetch(filename: str) -> None:
        index = json.loads(read(f"/simple/{names[filename]}/"))
        artifact = next(file for file in index["files"] if file["filename"] == filename)
        suffix = ".metadata" if route == "metadata" else ""
        body = read(artifact["url"] + suffix)
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
    required_bytes = sum(
        len(fixtures.simple[name]) for name in set(names.values())
    ) + sum(len(fixtures.metadata[filename + ".metadata"]) for filename in filenames)
    floor = max(
        required_bytes / profile["bytes_per_second"]
        if profile.get("bytes_per_second")
        else 0,
        2 * max(0, profile.get("latency_ms", 0) - profile.get("jitter_ms", 0)) / 1000,
    )
    data = {
        "profile": profile,
        "filenames": filenames,
        "route": route,
        "seconds": seconds,
        "required_metadata_and_index_bytes": required_bytes,
        "optimistic_network_floor_seconds": floor,
        "actual_bytes": sum(event["bytes"] for event in server.events),
        "requests": len(server.events),
        "events": server.events,
        "scope": "A known dependency graph with unlimited request concurrency. Full-wheel transfer is a realizable strategy, not a minimum-byte claim.",
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2) + "\n")
    print(
        json.dumps(
            {key: value for key, value in data.items() if key != "events"}, indent=2
        )
    )


def run_one(
    binary: Path, fixtures: Fixtures, profile: dict, args: argparse.Namespace
) -> dict:
    with tempfile.TemporaryDirectory(prefix="trial-", dir=args.work_dir) as directory:
        work = Path(directory)
        (work / "requirements.in").write_text(
            "".join(f"{item}\n" for item in args.requirement)
        )
        server = Server(fixtures, profile)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        command = [str(binary), "--no-config", "--no-progress", "--color", "never"]
        command.extend(
            arg.format(index=server.url + "/simple", work=work, python=args.python)
            for arg in args.command
        )
        env = {
            key: value for key, value in os.environ.items() if not key.startswith("UV_")
        }
        env.update(
            UV_CACHE_DIR=str(work / "cache"),
            UV_PYTHON_DOWNLOADS="never",
            NO_PROXY="127.0.0.1,localhost",
        )
        try:
            if args.cache_mode != "cold":
                subprocess.run(
                    command,
                    env=env,
                    capture_output=True,
                    timeout=args.timeout,
                    check=True,
                )
                server.reset()
            if args.cache_mode == "refresh":
                command.insert(1, "--refresh")
            start = time.perf_counter()
            result = subprocess.run(
                command, env=env, capture_output=True, timeout=args.timeout, check=False
            )
            seconds = time.perf_counter() - start
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        output = result.stdout.replace(server.url.encode(), b"[INDEX]").replace(
            str(work).encode(), b"[WORK]"
        )
        if result.returncode:
            raise RuntimeError(
                f"Command failed ({result.returncode}): {command}\n{result.stderr.decode(errors='replace')}"
            )
        return {
            "seconds": seconds,
            "stdout_sha256": hashlib.sha256(output).hexdigest(),
            "stdout": output.decode(errors="replace"),
            "stderr": result.stderr.decode(errors="replace"),
            "events": sorted(server.events, key=lambda event: event["start"]),
            "bytes": sum(event["bytes"] for event in server.events),
            "requests": len(server.events),
            "max_active": max((event["active"] for event in server.events), default=0),
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
    reference.add_argument("--route", choices=["metadata", "wheel"], default="metadata")
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
    run.add_argument("--pairs", type=int, default=20)
    run.add_argument("--warmups", type=int, default=2)
    run.add_argument(
        "--cache-mode", choices=["cold", "warm", "refresh"], default="cold"
    )
    run.add_argument("--timeout", type=float, default=300)
    run.add_argument("--required-bytes", type=int, default=0)
    run.add_argument("--required-waves", type=int, default=0)
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
        "profile": profile,
        "command": args.command,
        "requirements": args.requirement,
        "warmups": args.warmups,
        "cache_mode": args.cache_mode,
        "pairs": [],
        "lower_bound": {
            "required_bytes": args.required_bytes,
            "required_waves": args.required_waves,
            "seconds": max(
                args.required_bytes / profile["bytes_per_second"]
                if profile.get("bytes_per_second")
                else 0,
                args.required_waves
                * max(0, profile.get("latency_ms", 0) - profile.get("jitter_ms", 0))
                / 1000,
            ),
            "model": "Optimistic maximum of required-body serialization and serial response-latency waves; excludes TCP/TLS and CPU costs.",
        },
    }
    for index in range(args.warmups + args.pairs):
        order = ["parent", "head"] if index % 2 == 0 else ["head", "parent"]
        pair = {
            name: run_one(binaries[name], fixtures, profile, args) for name in order
        }
        if pair["parent"]["stdout_sha256"] != pair["head"]["stdout_sha256"]:
            raise ValueError("Parent and head command outputs differ")
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
