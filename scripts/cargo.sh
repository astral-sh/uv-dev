#!/usr/bin/env sh
## Wrapper script for locked Cargo commands and auditable release builds.
##
## Set `CARGO_LOCKED=1` to require an up-to-date `Cargo.lock`.
## Cargo extensions still need their own `--locked` flags.
## Set `CARGO_AUDITABLE=0` to disable SBOM embedding outside release builds.
## Use `scripts/install-cargo-extensions.sh` to install cargo-auditable.
##
## Usage:
##
##   CARGO_LOCKED=1 CARGO_AUDITABLE=0 scripts/cargo.sh build
##   CARGO="$PWD/scripts/cargo.sh" maturin build --release

set -eu

# Rustup requires `+toolchain` to precede Cargo's arguments.
toolchain=""
case "${1:-}" in
    +*)
        toolchain="$1"
        shift
        ;;
esac

locked="${CARGO_LOCKED:-0}"
auditable="${CARGO_AUDITABLE:-1}"
# Only inspect global options. Aliases can forward later arguments to a program.
skip_value=0
for argument in "$@"; do
    if [ "$skip_value" = "1" ]; then
        skip_value=0
        continue
    fi
    case "$argument" in
        --locked|--frozen) locked=0 ;;
        # Metadata and version queries do not compile artifacts to embed an SBOM in.
        --version|-V) auditable=0 ;;
        metadata)
            auditable=0
            break
            ;;
        --color|--config|-C|-Z) skip_value=1 ;;
        --|[!-]*|"") break ;;
    esac
done
if [ "$locked" = "1" ]; then
    set -- --locked "$@"
fi

if [ "$auditable" != "0" ]; then
    set -- auditable "$@"
fi

if [ -n "$toolchain" ]; then
    set -- "$toolchain" "$@"
fi

exec "${REAL_CARGO:-cargo}" "$@"
