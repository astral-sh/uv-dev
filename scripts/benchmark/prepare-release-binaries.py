"""Build comparable release-profile uv binaries with and without production PGO."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target-dir", type=Path, default=root / "target/bench-release"
    )
    parser.add_argument("--output", type=Path, default=root / ".cache/bench-release")
    args = parser.parse_args()
    build = args.target_dir.resolve()
    destination = args.output.resolve()
    spec = importlib.util.spec_from_file_location(
        "uv_benchmark_pgo", root / "scripts/build_uv_pgo.py"
    )
    if spec is None or spec.loader is None:
        raise RuntimeError("Could not load the release PGO builder")
    pgo = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = pgo
    spec.loader.exec_module(pgo)
    target = pgo.rustc_host()
    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("UV_")
    }
    overrides = sorted(
        name for name in environment if name.startswith("CARGO_PROFILE_RELEASE_")
    )
    if overrides or environment.get("CARGO_ENCODED_RUSTFLAGS"):
        parser.error("Remove Cargo release-profile and encoded-rustflags overrides")
    environment["CARGO_INCREMENTAL"] = "0"
    # Retain function symbols for walltime profiles without changing release code generation.
    environment["CARGO_PROFILE_RELEASE_STRIP"] = "false"
    flags = environment.get("RUSTFLAGS", "")
    if "profile-generate" in flags or "profile-use" in flags:
        parser.error("RUSTFLAGS must not contain an existing PGO configuration")
    if target == "aarch64-unknown-linux-gnu":
        sysroot = subprocess.check_output(
            ["rustc", "--print", "sysroot"], cwd=root, text=True
        ).strip()
        linker = Path(sysroot) / "lib/rustlib" / target / "bin/gcc-ld"
        if not (linker / "ld.lld").is_file():
            raise FileNotFoundError("The release ARM64 PGO build requires bundled LLD")
        flags = pgo.append_flags(
            flags, f"-C link-arg=-B{linker} -C link-arg=-fuse-ld=lld"
        )
        environment["JEMALLOC_SYS_WITH_LG_PAGE"] = "16"
    elif target == "aarch64-apple-darwin":
        flags = pgo.append_flags(
            flags, "-C linker=rust-lld -C linker-flavor=ld64.lld -C link-arg=--icf=safe"
        )
    elif target.endswith("-pc-windows-msvc") and "+crt-static" not in flags:
        flags = pgo.append_flags(flags, "-C target-feature=+crt-static")
    environment["RUSTFLAGS"] = flags
    subprocess.run(
        [
            sys.executable,
            str(root / "scripts/build_uv_pgo.py"),
            "--target",
            target,
            "--target-dir",
            str(build / "pgo"),
        ],
        cwd=root,
        env=environment,
        check=True,
    )
    baseline_environment = environment | {"CARGO_TARGET_DIR": str(build / "baseline")}
    if target.endswith("-apple-darwin"):
        for variable in ("CFLAGS", "CXXFLAGS"):
            baseline_environment[variable] = pgo.append_flags(
                environment.get(variable), "-fno-profile-generate -fno-profile-use"
            )
    subprocess.run(
        pgo.cargo_command(target), cwd=root, env=baseline_environment, check=True
    )
    suffix = ".exe" if target.endswith("-pc-windows-msvc") else ""
    binaries = {}
    for mode in ("baseline", "pgo"):
        path = destination / mode / f"uv{suffix}"
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(build / mode / target / "release" / path.name, path)
        binaries[mode] = {
            "sha256": digest(path),
            "version": subprocess.check_output(
                [str(path), "--version"], text=True
            ).strip(),
        }
    metadata = {
        "commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip(),
        "target": target,
        "rustc": subprocess.check_output(["rustc", "-Vv"], cwd=root, text=True).strip(),
        "rustflags": flags,
        "release_strip": False,
        "profile_sha256": digest(build / "pgo/uv.profdata"),
        "training_projects": [project.name for project in pgo.CORPUS_PROJECTS],
        "training_cutoff": pgo.DEPENDENCY_EXCLUDE_NEWER,
        "binaries": binaries,
    }
    (destination / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
