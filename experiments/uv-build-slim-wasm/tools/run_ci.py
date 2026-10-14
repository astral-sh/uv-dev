"""Install a checksum-pinned Python and run native backend comparisons."""

import argparse
import hashlib
import json
import os
import subprocess
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv-source", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--runtime", required=True)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--streaming", action="store_true")
    args = parser.parse_args()
    source, reference, report = map(
        Path.resolve, (args.uv_source, args.reference, args.report)
    )
    report.mkdir(parents=True, exist_ok=True)
    downloads = json.loads(
        (source / "crates/uv-python-managed/download-metadata.json").read_text()
    )
    entry = downloads[args.runtime]
    scratch = Path.home() / "code/tmp/uv-build-slim-ci" / args.runtime
    scratch.mkdir(parents=True, exist_ok=True)
    archive = scratch / "python.tar.gz"
    if not archive.exists():
        with urllib.request.urlopen(entry["url"]) as response, archive.open(
            "wb"
        ) as stream:
            while block := response.read(1024 * 1024):
                stream.write(block)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != entry["sha256"]:
        raise ValueError("Python archive checksum mismatch")
    installation = scratch / "runtime"
    if not installation.exists():
        installation.mkdir()
        with tarfile.open(archive) as stream:
            stream.extractall(installation, filter="data")
    executable = (
        installation / "python" / ("python.exe" if os.name == "nt" else "bin/python3")
    )
    environment = dict(os.environ, TMPDIR=str(scratch), PYTHONDONTWRITEBYTECODE="1")
    if entry.get("variant") == "freethreaded":
        environment["PYTHON_GIL"] = "0"
    wheel_directory = scratch / "wheel"
    wheel_directory.mkdir(exist_ok=True)
    filename = subprocess.check_output(
        [str(reference), "build-wheel", str(wheel_directory)],
        cwd=ROOT,
        text=True,
        encoding="utf-8",
    ).strip()
    command = [
        str(executable),
        str(ROOT / "tests/platform_probe.py"),
        "--source",
        str(source / "test/packages/built-by-uv"),
        "--reference",
        str(reference),
        "--wheel",
        str(wheel_directory / filename),
        "--report",
        str(report),
    ]
    if args.streaming:
        command.append("--streaming")
    (report / "python-download.json").write_text(json.dumps(entry, indent=2) + "\n")
    result = subprocess.run(command, env=environment, check=False)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
