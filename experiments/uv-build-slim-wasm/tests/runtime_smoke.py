"""Cross-version checks of the full backend against native Rust artifacts."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
import uv_build_slim_wasm as backend


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--program", type=Path)
    args = parser.parse_args()
    report_path = args.report.resolve()
    if args.program:
        backend._PROGRAM = backend.Program(args.program.resolve())
    os.chdir(args.source.resolve())
    scratch = Path(
        os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
    )
    report = {"python": sys.version, "checks": []}
    with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
        root = Path(temporary)
        for command, hook in [
            ("build-wheel", backend.build_wheel),
            ("build-sdist", backend.build_sdist),
            ("build-editable", backend.build_editable),
            (
                "prepare-metadata-for-build-wheel",
                backend.prepare_metadata_for_build_wheel,
            ),
            (
                "prepare-metadata-for-build-editable",
                backend.prepare_metadata_for_build_editable,
            ),
        ]:
            native = root / (command + "-native")
            generated = root / (command + "-python")
            native.mkdir()
            generated.mkdir()
            expected = subprocess.check_output(
                [str(args.reference), command, str(native)], text=True
            ).strip()
            started = time.monotonic()
            actual = hook(str(generated))
            assert actual == expected, (actual, expected)
            if (native / expected).is_file():
                assert (native / expected).read_bytes() == (
                    generated / actual
                ).read_bytes(), command
                assert (native / expected).stat().st_mode == (
                    generated / actual
                ).stat().st_mode, command
                digest = hashlib.sha256((generated / actual).read_bytes()).hexdigest()
            else:
                left = {
                    str(p.relative_to(native)): p.read_bytes()
                    for p in native.rglob("*")
                    if p.is_file()
                }
                right = {
                    str(p.relative_to(generated)): p.read_bytes()
                    for p in generated.rglob("*")
                    if p.is_file()
                }
                assert left == right, command
                digest = None
            result = {
                "hook": command,
                "equal": True,
                "sha256": digest,
                "seconds": time.monotonic() - started,
            }
            report["checks"].append(result)
            print(json.dumps(result), flush=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
