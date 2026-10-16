import datetime
import hashlib
import http.server
import json
import os
import platform
import socket
import socketserver
import subprocess
import threading
import time
from pathlib import Path

workspace = Path.cwd()
output = workspace / "python-download-traces"
output.mkdir(exist_ok=True)
binary = workspace / "target/fast-build-nightly/uv.exe"
metadata_path = workspace / "crates/uv-python-managed/download-metadata.json"
metadata = json.loads(metadata_path.read_text())
entry = metadata["cpython-3.13.16+freethreaded-windows-x86_64-none"]
github_url = entry["url"]
mirror_url = github_url.replace(
    "https://github.com/astral-sh/python-build-standalone/",
    "https://releases.astral.sh/github/python-build-standalone/",
)
(output / "curl-version.txt").write_text(
    subprocess.check_output(["curl.exe", "--version"], text=True), encoding="utf-8"
)
records = []
for name, client, http1, mirror in [
    ("uv-auto-mirror", "uv", False, None),
    ("curl-http1-mirror", "curl", True, None),
    ("uv-http1-mirror", "uv", True, None),
    (
        "uv-http1-github",
        "uv",
        True,
        "https://github.com/astral-sh/python-build-standalone/releases/download",
    ),
]:
    case = output / name
    case.mkdir(exist_ok=True)
    environment = os.environ.copy()
    environment.update(
        {
            "UV_PYTHON_CACHE_DIR": str(case / "archives"),
            "UV_PYTHON_INSTALL_DIR": str(case / "installed"),
            "UV_CACHE_DIR": str(case / "cache"),
            "UV_PYTHON_DOWNLOADS_JSON_URL": str(metadata_path),
            "UV_LOG_CONTEXT": "1",
            "RUST_LOG": "uv_python_managed=debug,uv_client=debug,uv_fs=debug,h2::codec=debug,hyper_util::client::legacy::connect=debug",
        }
    )
    environment.pop("UV_INTERNAL__DIAGNOSTIC_HTTP1", None)
    if http1:
        environment["UV_INTERNAL__DIAGNOSTIC_HTTP1"] = "1"
    if mirror:
        environment["UV_PYTHON_INSTALL_MIRROR"] = mirror
    if client == "curl":
        command = [
            "curl.exe",
            "--http1.1",
            "--location",
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "120",
            "--output",
            str(case / "archive.tar.gz"),
            "--write-out",
            '{"http_code":%{http_code},"remote_ip":"%{remote_ip}","http_version":"%{http_version}","size_download":%{size_download},"time_starttransfer":%{time_starttransfer},"time_total":%{time_total},"speed_download":%{speed_download}}',
            mirror_url,
        ]
    else:
        command = [
            str(binary),
            "--no-config",
            "--preview",
            "python",
            "install",
            "--no-bin",
            "3.13t",
        ]
    record = {
        "name": name,
        "client": client,
        "http1_only": http1,
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "started_at": datetime.datetime.now(datetime.UTC).isoformat(),
    }
    print(json.dumps(record), flush=True)
    started = time.monotonic()
    with (
        (case / "stderr.log").open("w", encoding="utf-8") as stderr,
        (case / "stdout.log").open("w", encoding="utf-8") as stdout,
    ):
        try:
            result = subprocess.run(
                command,
                env=environment,
                stdout=stdout,
                stderr=stderr,
                timeout=125 if client == "curl" else 120,
                check=False,
            )
            record["exit_code"] = result.returncode
        except subprocess.TimeoutExpired:
            record["timed_out"] = True
    record["elapsed_seconds"] = time.monotonic() - started
    if client == "curl" and record.get("exit_code") == 0:
        digest = hashlib.sha256((case / "archive.tar.gz").read_bytes()).hexdigest()
        record["sha256_matches_metadata"] = digest == entry["sha256"]
    records.append(record)
    (output / "protocol-results.json").write_text(
        json.dumps(records, indent=2), encoding="utf-8"
    )
    print(json.dumps(record), flush=True)


class IdleBodyHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", "1048576")
        self.end_headers()
        self.wfile.write(b"x")
        self.wfile.flush()
        time.sleep(5)
        self.close_connection = True

    def log_message(self, *args):
        pass


class IdleBodyServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


server = IdleBodyServer(("127.0.0.1", 0), IdleBodyHandler)
threading.Thread(target=server.serve_forever, daemon=True).start()
case = output / "idle-body-timeout"
case.mkdir(exist_ok=True)
environment.update(
    {
        "UV_PYTHON_CACHE_DIR": str(case / "archives"),
        "UV_PYTHON_INSTALL_DIR": str(case / "installed"),
        "UV_CACHE_DIR": str(case / "cache"),
        "UV_PYTHON_INSTALL_MIRROR": f"http://127.0.0.1:{server.server_port}",
        "UV_HTTP_TIMEOUT": "1",
        "UV_HTTP_RETRIES": "0",
    }
)
started = time.monotonic()
result = subprocess.run(
    [str(binary), "--no-config", "python", "install", "--no-bin", "3.13t"],
    env=environment,
    capture_output=True,
    text=True,
    timeout=10,
    check=False,
)
record = {
    "name": "idle-body-timeout",
    "elapsed_seconds": time.monotonic() - started,
    "exit_code": result.returncode,
}
(case / "stderr.log").write_text(result.stderr, encoding="utf-8")
(output / "idle-body-result.json").write_text(
    json.dumps(record, indent=2), encoding="utf-8"
)
print(json.dumps(record), flush=True)
server.shutdown()
