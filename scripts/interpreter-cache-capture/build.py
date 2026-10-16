"""Prepare two pinned worktrees and retain their source and build identities."""

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

from prepare import BASELINE, PROPOSAL, prepare

FEATURES = "test-python,test-pypi,test-python-managed,test-windows-registry,native-auth,windows-native"
PROFILE = "fast-build-nightly"


def build_commands(nextest):
    return {
        "uv": [
            "cargo",
            "build",
            "--locked",
            "--profile",
            PROFILE,
            "--no-default-features",
            "--features",
            FEATURES,
            "--workspace",
            "--bin",
            "uv",
            "-Z",
            "checksum-freshness",
        ],
        "probe": [
            "cargo",
            "build",
            "--locked",
            "--profile",
            PROFILE,
            "-p",
            "uv-python-interpreter",
            "--example",
            "interpreter-cache-probe",
            "-Z",
            "checksum-freshness",
        ],
        "tests": [
            str(nextest.resolve()),
            "nextest",
            "list",
            "--list-type",
            "binaries-only",
            "--message-format",
            "json",
            "--locked",
            "--cargo-profile",
            PROFILE,
            "-Z",
            "panic-abort-tests",
            "-Z",
            "checksum-freshness",
            "--no-default-features",
            "--features",
            FEATURES,
            "--workspace",
            "--test",
            "project",
            "--test",
            "sync",
        ],
        "metadata": ["cargo", "metadata", "--locked", "--format-version", "1"],
    }


def build_environment(source):
    environment = dict(os.environ)
    environment["CARGO_TARGET_DIR"] = str((source / "target").resolve())
    environment.pop("CARGO_BUILD_TARGET", None)
    return environment


def verify_metadata_paths(metadata, source):
    for field, expected in (
        ("workspace_root", source),
        ("target_directory", source / "target"),
    ):
        if Path(metadata[field]).resolve() != expected.resolve():
            raise RuntimeError(f"Cargo {field} does not match the pinned arm")


def execute(arguments, cwd, output, environment=None):
    with output.open("wb") as stream:
        subprocess.run(
            arguments,
            cwd=cwd,
            stdout=stream,
            stderr=subprocess.STDOUT,
            check=True,
            timeout=900,
            env=environment,
        )


def write_manifest(output, manifest):
    (output / "build-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--nextest", type=Path, required=True)
    arguments = parser.parse_args()
    root = Path.cwd()
    sources, output = arguments.sources.resolve(), arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    expected = os.environ["GITHUB_SHA"]
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != expected:
        raise RuntimeError("Diagnostic checkout does not match the dispatched commit")
    execute(["git", "reset", "--hard", expected], root, output / "checkout-reset.log")
    if subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"], text=True
    ).strip():
        raise RuntimeError("Pinned diagnostic checkout is not clean")
    sources.mkdir()
    for arm, revision in (("baseline", BASELINE), ("proposal", PROPOSAL)):
        execute(
            ["git", "cat-file", "-e", revision + "^{commit}"],
            root,
            output / f"{arm}-object-check.log",
        )
        execute(
            ["git", "worktree", "add", "--detach", str(sources / arm), revision],
            root,
            output / f"{arm}-checkout.log",
        )
    manifest = {"diagnostic_revision": actual, "arms": {}}
    for arm, revision in (("baseline", BASELINE), ("proposal", PROPOSAL)):
        source = sources / arm
        environment = build_environment(source)
        commands = build_commands(arguments.nextest)
        record = prepare(source, revision, root)
        record["commands"] = commands
        record["profile"] = PROFILE
        record["uv_and_tests_features"] = FEATURES.split(",")
        record["probe_features"] = "uv-python-interpreter default features"
        record["build_environment"] = {
            key: environment.get(key)
            for key in (
                "CARGO_TARGET_DIR",
                "CARGO_INCREMENTAL",
                "RUSTC_BOOTSTRAP",
                "UV_INTERNAL__BUILD_GIT_INFO",
                "RUSTFLAGS",
                "CARGO_ENCODED_RUSTFLAGS",
            )
        }
        patch = subprocess.check_output(
            ["git", "diff", "--binary", revision], cwd=source
        )
        (output / f"{arm}.patch").write_bytes(patch)
        files = [
            "crates/uv-python-interpreter/src/interpreter.rs",
            "crates/uv-python-interpreter/src/interpreter_capture.rs",
            "crates/uv-python-interpreter/examples/interpreter-cache-probe.rs",
            "crates/uv/tests/project/run.rs",
            "crates/uv/tests/sync/sync.rs",
            "crates/uv-test/src/lib.rs",
            "Cargo.lock",
            "rust-toolchain.toml",
        ]
        record["files"] = {
            path: hashlib.sha256((source / path).read_bytes()).hexdigest()
            for path in files
        }
        record["patch_sha256"] = hashlib.sha256(patch).hexdigest()
        record["source_tree"] = subprocess.check_output(
            ["git", "rev-parse", "HEAD^{tree}"], cwd=source, text=True
        ).strip()
        record["rustc"] = subprocess.check_output(
            ["rustc", "--version", "--verbose"], cwd=source, text=True, env=environment
        )
        manifest["arms"][arm] = record
        write_manifest(output, manifest)
        metadata = subprocess.check_output(
            commands["metadata"], cwd=source, env=environment, timeout=60
        )
        verify_metadata_paths(json.loads(metadata), source)
        record["cargo_paths_verified"] = True
        (source / "capture-cargo-metadata.json").write_bytes(metadata)
        (output / f"{arm}-cargo-metadata.json").write_bytes(metadata)
        execute(commands["uv"], source, output / f"{arm}-uv-build.log", environment)
        execute(
            commands["probe"], source, output / f"{arm}-probe-build.log", environment
        )
        with (
            (source / "capture-binaries.json").open("wb") as stdout,
            (output / f"{arm}-test-build.log").open("wb") as stderr,
        ):
            subprocess.run(
                commands["tests"],
                cwd=source,
                stdout=stdout,
                stderr=stderr,
                check=True,
                timeout=900,
                env=environment,
            )
        (output / f"{arm}-binaries.json").write_bytes(
            (source / "capture-binaries.json").read_bytes()
        )
        binaries = source / "target" / PROFILE
        record["uv_version"] = subprocess.check_output(
            [str(binaries / "uv.exe"), "--version"], text=True, env=environment
        ).strip()
        record["binary_sha256"] = {
            name: hashlib.sha256((binaries / name).read_bytes()).hexdigest()
            for name in ("uv.exe", "examples/interpreter-cache-probe.exe")
        }
        write_manifest(output, manifest)


if __name__ == "__main__":
    main()
