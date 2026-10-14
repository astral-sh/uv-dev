"""Compare real wheel and sdist output with a pinned native backend."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
import uv_build_slim_wasm as backend


def compare(native, generated):
    report = {
        "bytes_equal": native.read_bytes() == generated.read_bytes(),
        "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
        "generated_sha256": hashlib.sha256(generated.read_bytes()).hexdigest(),
    }
    differences = []
    if native.suffix == ".whl":
        with zipfile.ZipFile(native) as left, zipfile.ZipFile(generated) as right:
            if left.namelist() != right.namelist():
                differences.append("member order or names")
            for name in set(left.namelist()) & set(right.namelist()):
                if left.read(name) != right.read(name):
                    differences.append("content: " + name)
                a, b = left.getinfo(name), right.getinfo(name)
                if (a.external_attr, a.date_time, a.compress_type) != (
                    b.external_attr,
                    b.date_time,
                    b.compress_type,
                ):
                    differences.append("attributes: " + name)
    else:
        with tarfile.open(native) as left, tarfile.open(generated) as right:
            if left.getnames() != right.getnames():
                differences.append("member order or names")
            for name in set(left.getnames()) & set(right.getnames()):
                a, b = left.getmember(name), right.getmember(name)
                if (
                    a.isfile()
                    and b.isfile()
                    and left.extractfile(a).read() != right.extractfile(b).read()
                ):
                    differences.append("content: " + name)
                if (a.mode, a.uid, a.gid, a.mtime, a.type, a.linkname) != (
                    b.mode,
                    b.uid,
                    b.gid,
                    b.mtime,
                    b.type,
                    b.linkname,
                ):
                    differences.append("attributes: " + name)
    report["differences"] = sorted(differences)
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument(
        "--reference",
        type=Path,
        default=Path.home() / "code/tmp/uv-build-slim-poc/target/debug/uv-build",
    )
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    source = args.source.resolve()
    scratch = Path.home() / "code/tmp/uv-build-slim-poc/tmp"
    scratch.mkdir(parents=True, exist_ok=True)
    report = {}
    with tempfile.TemporaryDirectory(dir=scratch, prefix="full-parity-") as temporary:
        root = Path(temporary)
        previous = Path.cwd()
        try:
            os.chdir(source)
            for command, hook in [
                ("build-wheel", backend.build_wheel),
                ("build-sdist", backend.build_sdist),
                ("build-editable", backend.build_editable),
            ]:
                native = root / (command + "-native")
                generated = root / (command + "-generated")
                native.mkdir()
                generated.mkdir()
                result = subprocess.run(
                    [str(args.reference), command, str(native)],
                    text=True,
                    capture_output=True,
                    check=True,
                )
                name = result.stdout.strip()
                started = time.monotonic()
                filename = hook(str(generated))
                elapsed = time.monotonic() - started
                if filename != name:
                    raise AssertionError((name, filename))
                report[command] = compare(native / name, generated / name)
                report[command]["seconds"] = elapsed
                print(command, json.dumps(report[command]), flush=True)
        finally:
            os.chdir(previous)
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    return not all(value["bytes_equal"] for value in report.values())


if __name__ == "__main__":
    raise SystemExit(main())
