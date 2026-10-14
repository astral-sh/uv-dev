"""Collect license texts for the pinned Rust dependency closure."""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv-source", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "LICENSES")
    parser.add_argument("--target", action="append")
    args = parser.parse_args()
    targets = args.target or ["wasm32-wasip1", "x86_64-pc-windows-gnu"]
    trees = []
    for target in targets:
        trees.append(
            subprocess.check_output(
                [
                    "cargo",
                    "+1.99.0",
                    "tree",
                    "--locked",
                    "--offline",
                    "-p",
                    "uv-build",
                    "--target",
                    target,
                    "--edges",
                    "normal,build",
                    "--prefix",
                    "none",
                    "--format",
                    "{p}",
                    "--no-dedupe",
                ],
                cwd=args.uv_source,
                text=True,
            )
        )
    packages = sorted(set(re.findall(r"^(\S+) v(\S+)", "\n".join(trees), re.MULTILINE)))
    registry = (
        Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry/src"
    )
    workspace = tomllib.loads((args.uv_source / "Cargo.toml").read_text())["workspace"][
        "package"
    ]
    inventory = []
    args.output.mkdir(parents=True, exist_ok=True)
    for name, version in packages:
        if (args.uv_source / "crates" / name / "Cargo.toml").exists():
            directory = args.uv_source / "crates" / name
            license_root = args.uv_source
        else:
            candidates = sorted(registry.glob("*/" + name + "-" + version))
            if not candidates:
                raise FileNotFoundError(name + "-" + version)
            directory = license_root = candidates[0]
        package = tomllib.loads((directory / "Cargo.toml").read_text())["package"]
        license_ = package.get("license")
        if isinstance(license_, dict):
            license_ = workspace["license"]
        texts = [
            path
            for path in license_root.iterdir()
            if path.is_file()
            and re.match(r"(?i)(license|copying|copyright|notice)", path.name)
        ]
        if package.get("license-file"):
            texts.append(directory / package["license-file"])
        if not texts:
            family = (
                "tar-codec"
                if name in {"archive-trait", "tar-framing", "tar-codec"}
                else name
            )
            override = ROOT / "codegen/license-overrides" / family
            if override.is_dir():
                texts = [path for path in override.iterdir() if path.is_file()]
        if not texts:
            raise ValueError("No license text found for " + name)
        destination = args.output / (name + "-" + version)
        destination.mkdir(exist_ok=True)
        files = []
        for path in sorted(set(texts)):
            shutil.copyfile(path, destination / path.name)
            files.append(
                {
                    "path": str((destination / path.name).relative_to(args.output)),
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                }
            )
        inventory.append(
            {
                "name": name,
                "version": version,
                "license": license_,
                "license_files": files,
            }
        )
    sysroot = Path(
        subprocess.check_output(
            ["rustc", "+1.99.0", "--print", "sysroot"], text=True
        ).strip()
    )
    shutil.copyfile(
        sysroot / "share/doc/rust/COPYRIGHT-library.html",
        args.output / "RUST-STANDARD-LIBRARY.html",
    )
    (args.output / "inventory.json").write_text(
        json.dumps(
            {
                "dependency_edges": ["normal", "build"],
                "targets": targets,
                "packages": inventory,
                "rust_standard_library": "RUST-STANDARD-LIBRARY.html",
            },
            indent=2,
        )
        + "\n"
    )
    print(
        json.dumps(
            {
                "packages": len(inventory),
                "license_files": sum(len(p["license_files"]) for p in inventory),
            }
        )
    )


if __name__ == "__main__":
    main()
