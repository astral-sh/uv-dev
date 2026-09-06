#!/usr/bin/env bash
# Runs inside the pinned builder. See README.md in this directory.

set -euo pipefail

MODE="$1"
BUILD_ROOT="$PWD"
TARGET=x86_64-unknown-linux-musl
ARCHIVE_NAME="uv-$TARGET"

export CARGO_HOME="$BUILD_ROOT/cargo"
export CARGO_TARGET_DIR="$BUILD_ROOT/target"
export CARGO_INCREMENTAL=0
export LC_ALL=C
export TZ=UTC
umask 022

mkdir -p "$BUILD_ROOT/source" "$CARGO_HOME"
tar -xf /input/source.tar -C "$BUILD_ROOT/source"
cd "$BUILD_ROOT/source"

# Fail instead of silently using a different compiler after a toolchain update.
TOOLCHAIN=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
test "$(rustc --version | cut -d ' ' -f 2)" = "$TOOLCHAIN"

if [[ "$MODE" == fetch ]]; then
    cargo vendor --locked --versioned-dirs /vendor > /output/vendor-config.toml
    exit 0
fi
if [[ "$MODE" != build ]]; then
    echo "Unknown mode: $MODE" >&2
    exit 1
fi

# The source archive has no .git directory. The caller supplies the exact
# compile-time version metadata instead of depending on local tags or history.
: "${UV_COMMIT_HASH:?}" "${UV_COMMIT_SHORT_HASH:?}" "${UV_COMMIT_DATE:?}"
: "${SOURCE_DATE_EPOCH:?}" "${REPRO_SOURCE_MTIME:?}"
cat /input/vendor-config.toml >> .cargo/config.toml
find . -exec touch -h -d "@$REPRO_SOURCE_MTIME" {} +

export RUSTFLAGS="--remap-path-prefix=$BUILD_ROOT/source=/uv --remap-path-prefix=$CARGO_TARGET_DIR=/uv-target --remap-path-prefix=$CARGO_HOME=/cargo --remap-path-prefix=/vendor=/cargo/vendor"
export CFLAGS="-ffile-prefix-map=$BUILD_ROOT=/build -ffile-prefix-map=/vendor=/cargo/vendor"
export CXXFLAGS="$CFLAGS"

{
    rustc -Vv
    cargo -V
    "$TARGET_CC" --version
    "$TARGET_CC" -print-sysroot
    cat /build-tools/cargo-extensions.txt
} > /output/toolchain.txt

cargo auditable build --frozen --release --package uv \
    --bin uv --bin uvx --features self-update --target "$TARGET"

mkdir -p "/output/$ARCHIVE_NAME"
for binary in uv uvx; do
    install -m 755 "$CARGO_TARGET_DIR/$TARGET/release/$binary" "/output/$ARCHIVE_NAME/$binary"
    readelf --wide --program-headers --dynamic "/output/$ARCHIVE_NAME/$binary" > "/output/$binary.elf.txt"
    if grep -Eq 'INTERP|\(NEEDED\)' "/output/$binary.elf.txt"; then
        echo "$binary must be statically linked" >&2
        exit 1
    fi
    readelf --wide --sections "/output/$ARCHIVE_NAME/$binary" > "/output/$binary.sections.txt"
    grep -F '.dep-v0' "/output/$binary.sections.txt"
    "/output/$ARCHIVE_NAME/$binary" --version
done
"/output/$ARCHIVE_NAME/uv" self update --help > /dev/null

tar --sort=name --format=ustar --mtime="@$SOURCE_DATE_EPOCH" \
    --owner=0 --group=0 --numeric-owner \
    -cf - -C /output "$ARCHIVE_NAME" \
    | gzip -n -9 > "/output/$ARCHIVE_NAME.tar.gz"
cd /output
sha256sum "$ARCHIVE_NAME/uv" "$ARCHIVE_NAME/uvx" "$ARCHIVE_NAME.tar.gz" > SHA256SUMS
cat SHA256SUMS
