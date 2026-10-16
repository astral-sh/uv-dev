import datetime
import json
import os
import platform
import socket
import subprocess
import time
from pathlib import Path

workspace = Path.cwd()
output = workspace / "python-download-traces"
output.mkdir(exist_ok=True)
records = []
for name, version, http1 in [
    ("auto-313", "3.13t", False),
    ("http1-313", "3.13t", True),
    ("http1-314", "3.14.0", True),
    ("auto-314", "3.14.0", False),
]:
    case = output / name
    case.mkdir(exist_ok=True)
    environment = os.environ.copy()
    environment.update(
        {
            "UV_PYTHON_CACHE_DIR": str(case / "archives"),
            "UV_PYTHON_INSTALL_DIR": str(case / "installed"),
            "UV_CACHE_DIR": str(case / "cache"),
            "UV_PYTHON_DOWNLOADS_JSON_URL": str(
                workspace / "crates/uv-python-managed/download-metadata.json"
            ),
            "UV_LOG_CONTEXT": "1",
            "RUST_LOG": "uv_python_managed=debug,uv_client=debug,uv_fs=debug,h2::codec=debug,hyper_util::client::legacy::connect=debug",
        }
    )
    environment.pop("UV_INTERNAL__DIAGNOSTIC_HTTP1", None)
    if http1:
        environment["UV_INTERNAL__DIAGNOSTIC_HTTP1"] = "1"
    record = {
        "name": name,
        "version": version,
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
                [
                    str(workspace / "target/fast-build-nightly/uv.exe"),
                    "--no-config",
                    "--preview",
                    "python",
                    "install",
                    "--no-bin",
                    version,
                ],
                env=environment,
                stdout=stdout,
                stderr=stderr,
                timeout=120,
                check=False,
            )
            record["exit_code"] = result.returncode
        except subprocess.TimeoutExpired:
            record["timed_out"] = True
    record["elapsed_seconds"] = time.monotonic() - started
    records.append(record)
    (output / "protocol-results.json").write_text(
        json.dumps(records, indent=2), encoding="utf-8"
    )
    print(json.dumps(record), flush=True)
