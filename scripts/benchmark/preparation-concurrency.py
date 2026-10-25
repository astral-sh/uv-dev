"""Compare uv download/install concurrency against an immutable loopback index."""

from __future__ import annotations

import argparse
import base64
import hashlib
import io
import itertools
import json
import math
import os
import platform
import random
import shutil
import statistics
import subprocess
import sys
import threading
import time
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def wheel(name: str, payload: bytes, compression: int) -> tuple[bytes, bytes, int]:
    metadata = f"Metadata-Version: 2.3\nName: {name}\nVersion: 1.0\n\n".encode()
    info = f"{name}-1.0.dist-info"
    members = {
        f"{name}/__init__.py": b"",
        f"{name}/payload.bin": payload,
        f"{info}/METADATA": metadata,
        f"{info}/WHEEL": (
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    records = []
    for path, data in sorted(members.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
        records.append(f"{path},sha256={digest.decode()},{len(data)}\n")
    members[f"{info}/RECORD"] = ("".join(records) + f"{info}/RECORD,,\n").encode()
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        for path, data in sorted(members.items()):
            entry = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, data, compress_type=compression, compresslevel=6)
    return output.getvalue(), metadata, sum(map(len, members.values()))


def corpus(
    packages: int, max_kib: int, seed: int, compression: str, payload_kind: str
) -> tuple[dict, list[dict]]:
    generator = random.Random(seed)
    routes = {}
    manifest = []
    for index in range(packages):
        name = f"concurrency_fixture_{index:03}"
        # Interleave small and large wheels instead of ordering names by size.
        size = 1024 * max(1, max_kib // (2 ** (index % 5)))
        if payload_kind == "random":
            payload = generator.randbytes(size)
        else:
            block = generator.randbytes(64)
            payload = (block * math.ceil(size / len(block)))[:size]
        data, metadata, expanded_bytes = wheel(
            name,
            payload,
            zipfile.ZIP_STORED if compression == "stored" else zipfile.ZIP_DEFLATED,
        )
        filename = f"{name}-1.0-py3-none-any.whl"
        digest = hashlib.sha256(data).hexdigest()
        routes[f"/files/{filename}"] = (data, "application/octet-stream")
        routes[f"/files/{filename}.metadata"] = (metadata, "text/plain")
        detail = {
            "meta": {"api-version": "1.0"},
            "name": name.replace("_", "-"),
            "files": [
                {
                    "filename": filename,
                    "url": f"/files/{filename}",
                    "hashes": {"sha256": digest},
                    "size": len(data),
                    "core-metadata": {"sha256": hashlib.sha256(metadata).hexdigest()},
                }
            ],
        }
        routes[f"/simple/{name.replace('_', '-')}/"] = (
            json.dumps(detail).encode(),
            "application/vnd.pypi.simple.v1+json",
        )
        manifest.append(
            {
                "name": name,
                "filename": filename,
                "sha256": digest,
                "size": len(data),
                "expanded_bytes": expanded_bytes,
                "payload_bytes": len(payload),
                "compression": compression,
                "payload_kind": payload_kind,
                "payload_sha256": hashlib.sha256(payload).hexdigest(),
            }
        )
    return routes, manifest


class ReplayServer(ThreadingHTTPServer):
    def __init__(self, routes: dict, latency: float, chunk_delay: float):
        self.request_queue_size = max(128, len(routes))
        super().__init__(("127.0.0.1", 0), ReplayHandler)
        self.routes = routes
        self.latency = latency
        self.chunk_delay = chunk_delay
        self.events: list[dict] = []
        self.event_lock = threading.Lock()


class ReplayHandler(BaseHTTPRequestHandler):
    server: ReplayServer

    def do_GET(self):
        path = urlsplit(self.path).path
        started = time.perf_counter()
        data, content_type = self.server.routes.get(path, (b"", "text/plain"))
        status = 200 if path in self.server.routes else 404
        is_wheel = path.endswith(".whl")
        written = 0
        try:
            if is_wheel and self.server.latency:
                time.sleep(self.server.latency)
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "public, max-age=31536000, immutable")
            self.end_headers()
            for offset in range(0, len(data), 64 * 1024):
                if is_wheel and self.server.chunk_delay:
                    time.sleep(self.server.chunk_delay)
                chunk = data[offset : offset + 64 * 1024]
                self.wfile.write(chunk)
                written += len(chunk)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            with self.server.event_lock:
                self.server.events.append(
                    {
                        "path": path,
                        "status": status,
                        "bytes": written,
                        "start": started,
                        "end": time.perf_counter(),
                    }
                )

    def log_message(self, format, *args):
        pass


def command(
    arguments: list[str], environment: dict, log: Path, *, timeout: float = 300
) -> None:
    timed_out = threading.Event()
    with (
        log.open("w") as output,
        subprocess.Popen(
            arguments,
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
            text=True,
        ) as process,
    ):

        def expire() -> None:
            if process.poll() is None:
                timed_out.set()
                process.kill()

        timer = threading.Timer(timeout, expire)
        timer.daemon = True
        timer.start()
        try:
            returncode = process.wait()
        except BaseException:
            process.kill()
            process.wait()
            raise
        finally:
            timer.cancel()
            timer.join()
    if timed_out.is_set():
        raise subprocess.TimeoutExpired(arguments, timeout)
    if returncode:
        raise RuntimeError(f"Command failed ({returncode}); see {log}")


def run_sample(
    uv: str,
    directory: Path,
    cache: Path,
    requirements: Path,
    server: ReplayServer,
    manifest: list[dict],
    downloads: int,
    installs: int,
    environment: dict,
) -> dict:
    directory.mkdir()
    venv = directory / "venv"
    prefix = [uv, "--no-config", "--no-python-downloads", "--cache-dir", str(cache)]
    command(
        prefix + ["venv", "--python", sys.executable, str(venv)],
        environment,
        directory / "venv.log",
    )
    python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    run_environment = environment | {
        "UV_CONCURRENT_DOWNLOADS": str(downloads),
        "UV_CONCURRENT_INSTALLS": str(installs),
    }
    with server.event_lock:
        server.events.clear()
    started = time.perf_counter()
    command(
        prefix
        + [
            "pip",
            "sync",
            str(requirements),
            "--python",
            str(python),
            "--index-url",
            f"http://127.0.0.1:{server.server_port}/simple/",
            "--require-hashes",
            "--no-build",
            "--link-mode",
            "copy",
        ],
        run_environment,
        directory / "sync.log",
    )
    elapsed = time.perf_counter() - started
    # Verify installed bytes outside the timed region.
    site = Path(
        subprocess.check_output(
            [
                str(python),
                "-c",
                "import sysconfig; print(sysconfig.get_path('purelib'))",
            ],
            env=environment,
            text=True,
        ).strip()
    )
    for package in manifest:
        with (site / package["name"] / "payload.bin").open("rb") as file:
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        if digest != package["payload_sha256"]:
            raise RuntimeError(f"Installed payload mismatch: {package['name']}")
    with server.event_lock:
        requests = [event.copy() for event in server.events]
    for event in requests:
        event["start"] -= started
        event["end"] -= started
    wheel_requests = [event for event in requests if event["path"].endswith(".whl")]
    transitions = sorted(
        (event[edge], change)
        for event in wheel_requests
        for edge, change in [("start", 1), ("end", -1)]
    )
    active = peak = 0
    for _, change in transitions:
        active += change
        peak = max(peak, active)
    shutil.rmtree(venv)
    return {
        "seconds": elapsed,
        "downloads": downloads,
        "installs": installs,
        "wheel_requests": len(wheel_requests),
        "wheel_bytes": sum(event["bytes"] for event in wheel_requests),
        "peak_wheel_requests": peak,
        "requests": requests,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--uv", required=True, action="append", help="Repeat to compare binaries"
    )
    parser.add_argument(
        "--output", required=True, type=Path, help="A new directory for evidence"
    )
    parser.add_argument("--downloads", nargs="+", type=positive, default=[1, 8])
    parser.add_argument("--installs", nargs="+", type=positive, default=[1, 8])
    parser.add_argument("--runs", type=positive, default=5)
    parser.add_argument("--packages", type=positive, default=16)
    parser.add_argument("--max-kib", type=positive, default=1024)
    parser.add_argument(
        "--compression", choices=("stored", "deflated"), default="stored"
    )
    parser.add_argument("--payload", choices=("random", "repeated"), default="random")
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--latency-ms", type=float, default=0)
    parser.add_argument("--chunk-delay-ms", type=float, default=0)
    args = parser.parse_args()
    if any(
        not math.isfinite(value) or value < 0
        for value in (args.latency_ms, args.chunk_delay_ms)
    ):
        parser.error("delays must be finite and nonnegative")
    binaries = []
    for name in args.uv:
        found = shutil.which(name)
        if found is None:
            parser.error(f"executable not found: {name}")
        binaries.append(str(Path(found).resolve()))
    output = args.output.resolve()
    output.mkdir(parents=True)
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("UV_")
        and key
        not in {
            "VIRTUAL_ENV",
            "PYTHONHOME",
            "PYTHONPATH",
            "RUST_LOG",
            "TRACING_DURATIONS_FILE",
        }
    }
    environment.update(
        NO_PROXY="127.0.0.1,localhost,::1", no_proxy="127.0.0.1,localhost,::1"
    )
    routes, manifest = corpus(
        args.packages, args.max_kib, args.seed, args.compression, args.payload
    )
    (output / "corpus.json").write_text(json.dumps(manifest, indent=2) + "\n")
    requirements = output / "requirements.txt"
    requirements.write_text(
        "".join(
            f"{package['name']}==1.0 --hash=sha256:{package['sha256']}\n"
            for package in manifest
        )
    )
    evidence = {
        "platform": platform.platform(),
        "python": sys.version,
        "arguments": vars(args) | {"output": str(output)},
        "corpus": {
            "wheel_bytes": sum(package["size"] for package in manifest),
            "expanded_bytes": sum(package["expanded_bytes"] for package in manifest),
            "payload_bytes": sum(package["payload_bytes"] for package in manifest),
            "compression": args.compression,
            "payload_kind": args.payload,
        },
        "binaries": [],
        "samples": [],
        "summary": [],
    }
    server = ReplayServer(routes, args.latency_ms / 1000, args.chunk_delay_ms / 1000)
    evidence["server_backlog"] = server.request_queue_size
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        configurations = list(
            itertools.product(range(len(binaries)), args.downloads, args.installs)
        )
        generator = random.Random(args.seed)
        for uv in binaries:
            with Path(uv).open("rb") as file:
                digest = hashlib.file_digest(file, "sha256").hexdigest()
            evidence["binaries"].append(
                {
                    "path": uv,
                    "sha256": digest,
                    "version": subprocess.check_output(
                        [uv, "--version"], env=environment, text=True
                    ).strip(),
                }
            )
        for repeat in range(args.runs):
            generator.shuffle(configurations)
            for binary, downloads, installs in configurations:
                label = f"b{binary}-d{downloads}-i{installs}-r{repeat}"
                cache = output / f"{label}-cache"
                for state in ("cold", "warm"):
                    sample = run_sample(
                        binaries[binary],
                        output / f"{label}-{state}",
                        cache,
                        requirements,
                        server,
                        manifest,
                        downloads,
                        installs,
                        environment,
                    ) | {"binary": binary, "repeat": repeat, "cache": state}
                    evidence["samples"].append(sample)
                    (output / "results.json").write_text(
                        json.dumps(evidence, indent=2) + "\n"
                    )
                    print(
                        f"{label} {state}: {sample['seconds']:.4f}s; "
                        f"{sample['wheel_requests']} wheel requests",
                        flush=True,
                    )
                shutil.rmtree(cache)
        for binary, downloads, installs, state in itertools.product(
            range(len(binaries)), args.downloads, args.installs, ("cold", "warm")
        ):
            values = sorted(
                sample["seconds"]
                for sample in evidence["samples"]
                if (
                    sample["binary"],
                    sample["downloads"],
                    sample["installs"],
                    sample["cache"],
                )
                == (binary, downloads, installs, state)
            )
            evidence["summary"].append(
                {
                    "binary": binary,
                    "downloads": downloads,
                    "installs": installs,
                    "cache": state,
                    "samples": len(values),
                    "p50_seconds": statistics.median(values),
                    "p95_seconds": values[math.ceil(0.95 * len(values)) - 1],
                }
            )
        (output / "results.json").write_text(json.dumps(evidence, indent=2) + "\n")
    except Exception as error:
        evidence["error"] = {"type": type(error).__name__, "message": str(error)}
        (output / "results.json").write_text(json.dumps(evidence, indent=2) + "\n")
        raise
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == "__main__":
    main()
