"""Compile the pinned Windows Rust backend into generated Python.

The custom target retains Windows configuration, UTF-16 paths, and the Windows
standard library. DLL imports cross a Python ctypes boundary at execution time.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

import tomllib
from build import PYWASM_REVISION, ROOT, UV_REVISION, digest, registry_remappings, run


def apply(patch, directory):
    run(["git", "apply", "--check", patch], cwd=directory)
    run(["git", "apply", patch], cwd=directory)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv-source", required=True, type=Path)
    parser.add_argument("--pywasm-source", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--pointer-width", type=int, choices=[32, 64], default=64)
    args = parser.parse_args()
    source_input, pywasm, work, output = map(
        Path.resolve, (args.uv_source, args.pywasm_source, args.work, args.output)
    )
    if work.exists():
        parser.error("--work must name a new directory")
    revision = subprocess.check_output(
        ["git", "-C", str(pywasm), "rev-parse", "HEAD"], text=True
    ).strip()
    if revision != PYWASM_REVISION:
        parser.error("pywasm must be checked out at " + PYWASM_REVISION)
    run(["git", "-C", pywasm, "diff", "--exit-code", "HEAD", "--", "pywasm"])
    work.mkdir(parents=True)
    source = work / "uv"
    source.mkdir()
    archive = work / "uv-source.tar"
    with archive.open("wb") as stream:
        run(["git", "-C", source_input, "archive", UV_REVISION], stdout=stream)
    with tarfile.open(archive) as stream:
        stream.extractall(source, filter="data")
    locked = tomllib.loads((source / "Cargo.lock").read_text())["package"]
    locked_packages = {(p["name"], p["version"]): p for p in locked}
    port = ROOT / (
        "codegen/windows-port32" if args.pointer_width == 32 else "codegen/windows-port"
    )
    apply(ROOT / "codegen/wasi-port.patch", source)
    apply(port / "uv.patch", source)

    sysroot = work / "rust-sysroot"
    run([sys.executable, ROOT / "codegen/prepare_sysroot.py", "--destination", sysroot])
    apply(port / "sysroot.patch", sysroot / "lib/rustlib/src/rust/library")
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
    crates = work / "crates"
    crates.mkdir()
    dependencies = json.loads((port / "dependencies.json").read_text())
    dependency_hashes = {}
    configuration = ["[patch.crates-io]"]
    for directory_name in dependencies:
        candidates = list(
            (cargo_home / "registry/cache").glob("*/" + directory_name + ".crate")
        )
        if len(candidates) != 1:
            raise ValueError(
                "Fetch the pinned registry dependency first: " + directory_name
            )
        crate_archive = candidates[0]
        package, version = directory_name.rsplit("-", 1)
        checksum = digest(crate_archive)
        if checksum != locked_packages[package, version]["checksum"]:
            raise ValueError(
                "Dependency differs from the pinned Cargo.lock: " + directory_name
            )
        dependency_hashes[directory_name] = checksum
        destination = crates / directory_name
        with tarfile.open(crate_archive) as stream:
            stream.extractall(crates, filter="data")
        if directory_name.startswith(("windows-sys-", "windows-0.")):
            architecture = "x86" if args.pointer_width == 32 else "x86_64"
            for file in destination.rglob("*.rs"):
                text = file.read_text()
                changed = text.replace(
                    'target_arch = "' + architecture + '"',
                    'any(target_arch = "'
                    + architecture
                    + '", target_arch = "wasm'
                    + str(args.pointer_width)
                    + '")',
                )
                if changed != text:
                    file.write_text(changed)
        configuration.append(
            json.dumps(directory_name)
            + " = { package = "
            + json.dumps(package)
            + ", path = "
            + json.dumps(str(destination))
            + " }"
        )
    apply(port / "dependencies.patch", crates)
    config = work / "patches.toml"
    config.write_text("\n".join(configuration) + "\n")
    target = work / ("wasm" + str(args.pointer_width) + "-pc-windows-gnu.json")
    shutil.copyfile(port / "target.json", target)
    imports = work / "import-libraries"
    imports.mkdir()
    for name in (
        "kernel32 ntdll userenv ws2_32 dbghelp bcrypt bcryptprimitives advapi32 "
        "gdi32 ucrt msvcrt rpcrt4 ole32 oleaut32 shell32 synchronization"
    ).split():
        # Symbols are imported through wasm_import_module; these empty archives
        # satisfy dependency metadata that still names native import libraries.
        (imports / ("lib" + name + ".a")).write_bytes(b"!<arch>\n")
    flags = [
        "-Zunstable-options",
        "-Lnative=" + str(imports),
        "-C",
        "link-arg=--allow-undefined",
        "-C",
        "link-arg=--export=main",
        "-C",
        "link-arg=--export=uv_build_slim_initialize_self_replace",
        "--remap-path-prefix=" + str(source) + "=/uv",
        "--remap-path-prefix=" + str(crates) + "=/windows-crates",
        "--remap-path-prefix=" + str(cargo_home) + "=/cargo",
        "--remap-path-prefix=" + str(sysroot) + "=/rust-sysroot",
        *registry_remappings(cargo_home),
    ]
    environment = dict(os.environ)
    environment.update(
        {
            "RUSTC": str(sysroot / "rustc-with-sysroot"),
            "RUSTC_BOOTSTRAP": "1",
            "RUSTFLAGS": " ".join(flags),
            "CARGO_TARGET_DIR": str(work / "target"),
            "CARGO_PROFILE_DEV_OPT_LEVEL": "z",
            "CARGO_PROFILE_DEV_DEBUG": "0",
            "TMPDIR": str(work / "tmp"),
        }
    )
    (work / "tmp").mkdir()
    run(
        [
            "cargo",
            "+1.99.0",
            "build",
            "--config",
            config,
            "-Zjson-target-spec",
            "-Zbuild-std=std,panic_abort",
            "-Zbuild-std-features=",
            "--offline",
            "-p",
            "uv-build",
            "--target",
            target,
        ],
        cwd=source,
        env=environment,
    )
    resolved = tomllib.loads((source / "Cargo.lock").read_text())["package"]
    if {(p["name"], p["version"]) for p in resolved} != set(locked_packages):
        raise ValueError("The platform port changed the pinned dependency versions")
    wasm = work / "target" / target.stem / "debug/uv-build.wasm"
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
            "--entry",
            "main",
            "--initializer",
            "uv_build_slim_initialize_self_replace",
        ]
    )
    provenance = {
        "uv_revision": UV_REVISION,
        "pywasm_revision": PYWASM_REVISION,
        "rust_toolchain": "1.99.0",
        "target": target.stem,
        "profile": "dev",
        "optimization": "z",
        "wasm_sha256": digest(wasm),
        "cargo_lock_sha256": digest(source / "Cargo.lock"),
        "registry_packages": dependency_hashes,
        "port_files": {
            p.name: digest(p) for p in sorted(port.iterdir()) if p.is_file()
        },
        "generator_sha256": digest(ROOT / "codegen/generate.py"),
        "build_script_sha256": digest(Path(__file__)),
        "full_parity_established": False,
    }
    (output / "_provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")


if __name__ == "__main__":
    main()
