#!/bin/bash
# Inner half of build-trampolines.sh: runs inside the uv-trampoline-builder
# container.
#
# Bind-mounts:
# * `/source`: read-only workspace
# * `/output`: directory for the produced .exe files
# Arguments:
# 1. rust nightly toolchain channel

set -euo pipefail

TOOLCHAIN="$1"

export CARGO_TARGET_DIR=/tmp/target

# Copy workspace to a writable location.
cp -r /source /workspace

# Normalize crate versions to reduce version-only changes to cargo's Strict
# Version Hash in the binaries. Keep uv-version's actual application version
# and workspace requirement: environment-variable metadata validation compares
# its introduction versions against this semantically consumed version.
find /workspace -name Cargo.toml ! -path /workspace/crates/uv-version/Cargo.toml \
    -exec sed -i 's/^version = .*/version = "0.0.0"/' {} +
sed -i -E '/^uv-version = /!s/version = "[^"]+"(, path = ")/version = "0.0.0"\1/g' /workspace/Cargo.toml

# The working directory must be the trampoline crate so cargo picks up
# `.cargo/config.toml`, which enables build-std.
cd /workspace/crates/uv-trampoline

cargo +"$TOOLCHAIN" xwin build --xwin-arch x86 --release --target i686-pc-windows-msvc
cargo +"$TOOLCHAIN" xwin build --release --target x86_64-pc-windows-msvc
cargo +"$TOOLCHAIN" xwin build --release --target aarch64-pc-windows-msvc

for arch in i686 x86_64 aarch64; do
    for variant in console gui; do
        cp "/tmp/target/$arch-pc-windows-msvc/release/uv-trampoline-$variant.exe" \
            "/output/uv-trampoline-$arch-$variant.exe"
    done
done
