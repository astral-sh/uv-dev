"""A dependency-free PEP 517 backend for installed-sidecar qualification."""

from __future__ import annotations

import base64
import csv
import hashlib
import io
import json
import os
import zipfile
from pathlib import Path

NAME = "installed-sidecar-fixture"
MODULE = NAME.replace("-", "_")
VERSION = "1.0.0"
DIST_INFO = f"{MODULE}-{VERSION}.dist-info"
METADATA = (
    f"Metadata-Version: 2.3\nName: {NAME}\nVersion: {VERSION}\n"
    "Requires-Python: >=3.9\n\n"
)
WHEEL = (
    "Wheel-Version: 1.0\nGenerator: uv-installed-sidecar-qualification\n"
    "Root-Is-Purelib: true\nTag: py3-none-any\n\n"
)


def event(phase: str, flavor: str) -> None:
    path = os.environ.get("SIDECAR_QUALIFICATION_LOG")
    if path is None:
        return
    value = json.dumps(
        {"phase": phase, "flavor": flavor, "pid": os.getpid()}, sort_keys=True
    )
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(descriptor, (value + "\n").encode())
    finally:
        os.close(descriptor)


def get_requires_for_build_wheel(config_settings=None):
    return []


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    directory = Path(metadata_directory) / DIST_INFO
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "METADATA").write_text(METADATA)
    (directory / "WHEEL").write_text(WHEEL)
    return DIST_INFO


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    flavor = (config_settings or {}).get("flavor", "default")
    if isinstance(flavor, list):
        flavor = flavor[-1]
    if flavor not in {"alpha", "beta", "default"}:
        raise ValueError("Unsupported fixture flavor")
    event("start", flavor)

    files = {
        f"{MODULE}.py": f"VALUE = {flavor!r}\n".encode(),
        f"{DIST_INFO}/METADATA": METADATA.encode(),
        f"{DIST_INFO}/WHEEL": WHEEL.encode(),
    }
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for name, contents in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(contents).digest())
        writer.writerow([name, "sha256=" + digest.rstrip(b"=").decode(), len(contents)])
    writer.writerow([f"{DIST_INFO}/RECORD", "", ""])
    files[f"{DIST_INFO}/RECORD"] = record.getvalue().encode()

    filename = f"{MODULE}-{VERSION}-py3-none-any.whl"
    with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
        for name, contents in sorted(files.items()):
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            wheel.writestr(info, contents)

    event("end", flavor)
    return filename
