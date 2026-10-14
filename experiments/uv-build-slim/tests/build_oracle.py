"""Stage a native differential oracle without modifying the uv checkout."""

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv-source", required=True, type=Path)
    parser.add_argument(
        "--scratch", type=Path, default=Path.home() / "code/tmp/uv-build-slim-poc"
    )
    args = parser.parse_args()
    source = args.uv_source.resolve()
    expected = json.loads(
        (Path(__file__).resolve().parents[1] / "vendor/manifest.json").read_text()
    )["revision"]
    actual = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    if actual != expected:
        raise SystemExit(f"Reference revision must be {expected}, got {actual}")
    if subprocess.check_output(
        ["git", "-C", str(source), "status", "--porcelain"], text=True
    ):
        raise SystemExit("Reference checkout must be clean")
    oracle = args.scratch / "oracle"
    (oracle / "src").mkdir(parents=True, exist_ok=True)
    (args.scratch / "tmp").mkdir(parents=True, exist_ok=True)
    shutil.copyfile(Path(__file__).with_name("oracle.rs"), oracle / "src/main.rs")
    # JSON string quoting is compatible with TOML basic strings for these paths.
    (oracle / "Cargo.toml").write_text(
        '[package]\nname = "uv-build-slim-oracle"\nversion = "0.1.0"\nedition = "2024"\n'
        '[dependencies]\nserde_json = "=1.0.151"\n'
        f"uv-normalize = {{path = {json.dumps(str(source / 'crates/uv-normalize'))}}}\n"
        f"uv-pypi-types = {{path = {json.dumps(str(source / 'crates/uv-pypi-types'))}}}\n"
        "[workspace]\n"
    )
    environment = dict(
        os.environ,
        CARGO_TARGET_DIR=str(args.scratch / "target"),
        TMPDIR=str(args.scratch / "tmp"),
    )
    subprocess.run(
        [
            "cargo",
            "+1.99.0",
            "build",
            "--offline",
            "--manifest-path",
            str(oracle / "Cargo.toml"),
        ],
        env=environment,
        check=True,
    )
    print(args.scratch / "target/debug/uv-build-slim-oracle")


if __name__ == "__main__":
    main()
