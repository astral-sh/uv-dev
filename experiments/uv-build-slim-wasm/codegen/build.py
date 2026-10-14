"""Rebuild the complete generated backend from pinned upstream sources."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tarfile
from pathlib import Path

UV_REVISION = "bd39416351244f273aeff89d495d4b0804b765a5"
PYWASM_REVISION = "47abba6c94ecf74c3c173344684891f2ca29c2e3"
ROOT = Path(__file__).resolve().parents[1]


def run(arguments, **kwargs):
    subprocess.run(list(map(str, arguments)), check=True, **kwargs)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def registry_remappings(cargo_home):
    return [
        "--remap-path-prefix=" + str(directory) + "=/cargo/registry/src"
        for directory in sorted((cargo_home / "registry/src").iterdir())
        if directory.is_dir()
    ]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv-source", required=True, type=Path)
    parser.add_argument("--pywasm-source", required=True, type=Path)
    parser.add_argument(
        "--work", type=Path, default=Path.home() / "code/tmp/uv-build-slim-regenerate"
    )
    parser.add_argument("--output", type=Path, default=ROOT / "src/uv_build_slim_wasm")
    args = parser.parse_args()
    original, pywasm, work, output = map(
        Path.resolve, (args.uv_source, args.pywasm_source, args.work, args.output)
    )
    actual = subprocess.check_output(
        ["git", "-C", str(pywasm), "rev-parse", "HEAD"], text=True
    ).strip()
    if actual != PYWASM_REVISION:
        parser.error("pywasm must be checked out at " + PYWASM_REVISION)
    run(["git", "-C", pywasm, "diff", "--exit-code", "HEAD", "--", "pywasm"])
    run(["git", "-C", original, "cat-file", "-e", UV_REVISION + "^{commit}"])
    work.mkdir(parents=True, exist_ok=True)
    source = work / "uv"
    source.mkdir()
    archive = work / "uv-source.tar"
    with archive.open("wb") as stream:
        run(["git", "-C", original, "archive", UV_REVISION], stdout=stream)
    with tarfile.open(archive) as stream:
        stream.extractall(source, filter="data")
    patch = ROOT / "codegen/wasi-port.patch"
    run(["git", "apply", "--check", patch], cwd=source)
    run(["git", "apply", patch], cwd=source)
    sysroot = work / "rust-sysroot"
    run([sys.executable, ROOT / "codegen/prepare_sysroot.py", "--destination", sysroot])
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
    flags = " ".join(
        [
            "--cfg wasi_ext",
            "--cfg tokio_unstable",
            "--remap-path-prefix=" + str(source) + "=/uv",
            "--remap-path-prefix=" + str(cargo_home) + "=/cargo",
            "--remap-path-prefix=" + str(sysroot) + "=/rust-sysroot",
            *registry_remappings(cargo_home),
        ]
    )
    environment = dict(os.environ)
    environment.update(
        {
            "RUSTC": str(sysroot / "rustc-with-sysroot"),
            "RUSTC_BOOTSTRAP": "1",
            "RUSTFLAGS": flags,
            "CARGO_TARGET_DIR": str(work / "target"),
            "CARGO_PROFILE_DEV_OPT_LEVEL": "z",
            "CARGO_PROFILE_DEV_DEBUG": "0",
            "CARGO_PROFILE_DEV_LTO": "fat",
            "CARGO_PROFILE_DEV_CODEGEN_UNITS": "1",
            "CARGO_PROFILE_DEV_STRIP": "symbols",
            "TMPDIR": str(work / "tmp"),
        }
    )
    (work / "tmp").mkdir(exist_ok=True)
    run(
        [
            "cargo",
            "+1.99.0",
            "build",
            "-Zbuild-std=std,panic_abort",
            "--locked",
            "-p",
            "uv-build",
            "--target",
            "wasm32-wasip1",
        ],
        cwd=source,
        env=environment,
    )
    wasm = work / "target/wasm32-wasip1/debug/uv-build.wasm"
    output.mkdir(parents=True, exist_ok=True)
    run(
        [
            sys.executable,
            ROOT / "codegen/generate.py",
            "--wasm",
            wasm,
            "--pywasm",
            pywasm,
            "--output",
            output,
        ]
    )
    run(
        [
            sys.executable,
            ROOT / "codegen/vendor_wasi.py",
            "--source",
            pywasm / "pywasm/wasi.py",
            "--output",
            output / "_wasi_impl.py",
        ]
    )
    provenance = {
        "uv_revision": UV_REVISION,
        "uv_version": "0.13.0",
        "pywasm_revision": PYWASM_REVISION,
        "rust_toolchain": "1.99.0",
        "target": "wasm32-wasip1",
        "profile": "dev",
        "optimization": "z",
        "lto": "fat",
        "codegen_units": 1,
        "wasi_port_sha256": digest(patch),
        "sysroot_adapter_sha256": digest(ROOT / "codegen/prepare_sysroot.py"),
        "generator_sha256": digest(ROOT / "codegen/generate.py"),
        "wasi_adapter_generator_sha256": digest(ROOT / "codegen/vendor_wasi.py"),
        "cargo_lock_sha256": digest(source / "Cargo.lock"),
        "wasm_sha256": digest(wasm),
    }
    (output / "_provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")


if __name__ == "__main__":
    main()
