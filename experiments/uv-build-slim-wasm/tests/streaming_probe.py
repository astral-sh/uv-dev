"""Exercise the upstream 16 MiB ZIP streaming threshold against native Rust."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
import uv_build_slim_wasm as backend


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--program", type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    report_path = args.report.resolve()
    if args.program:
        backend._PROGRAM = backend.Program(args.program.resolve())
    scratch = Path(
        os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
    )
    with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
        root = Path(temporary)
        source = root / "source"
        (source / "src/streaming").mkdir(parents=True)
        (source / "src/streaming/__init__.py").write_bytes(b"")
        size = 16 * 1024 * 1024 + 1
        with (source / "src/streaming/payload.bin").open("wb") as stream:
            stream.truncate(size)
        (source / "pyproject.toml").write_text(
            '[project]\nname="streaming"\nversion="1.0"\n[build-system]\nrequires=["uv_build>=0.13,<0.14"]\nbuild-backend="uv_build"\n'
        )
        os.chdir(source)
        checks = []
        for operation in ["build-wheel", "build-sdist"]:
            native = root / (operation + "-native")
            generated = root / (operation + "-python")
            native.mkdir()
            generated.mkdir()
            started = time.monotonic()
            expected = subprocess.check_output(
                [str(args.reference), operation, str(native)], text=True
            ).strip()
            native_seconds = time.monotonic() - started
            started = time.monotonic()
            actual = backend.call([operation, str(generated)])
            python_seconds = time.monotonic() - started
            assert actual == expected
            left = (native / expected).read_bytes()
            right = (generated / actual).read_bytes()
            assert left == right, operation
            result = {
                "hook": operation,
                "equal": True,
                "sha256": hashlib.sha256(right).hexdigest(),
                "source_file_bytes": size,
                "artifact_bytes": len(right),
                "native_seconds": native_seconds,
                "python_seconds": python_seconds,
            }
            checks.append(result)
            print(json.dumps(result), flush=True)
            report_path.write_text(json.dumps({"checks": checks}, indent=2) + "\n")


if __name__ == "__main__":
    main()
