#!/bin/bash
# Run commands for uv development in Apple's container runtime.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$REPO_ROOT/rust-toolchain.toml")"
IMAGE="${UV_APPLE_CONTAINER_IMAGE:-rust:${TOOLCHAIN}-bookworm}"
DNS="${UV_APPLE_CONTAINER_DNS:-1.1.1.1}"
CPUS="${UV_APPLE_CONTAINER_CPUS:-8}"
MEMORY="${UV_APPLE_CONTAINER_MEMORY:-16g}"

CARGO_REGISTRY_VOLUME="${UV_APPLE_CONTAINER_CARGO_REGISTRY_VOLUME:-uv-apple-cargo-registry}"
CARGO_GIT_VOLUME="${UV_APPLE_CONTAINER_CARGO_GIT_VOLUME:-uv-apple-cargo-git}"
TARGET_VOLUME="${UV_APPLE_CONTAINER_TARGET_VOLUME:-uv-apple-target}"
UV_CACHE_VOLUME="${UV_APPLE_CONTAINER_UV_CACHE_VOLUME:-uv-apple-uv-cache}"

create_volume() {
    local name="$1"
    local size="$2"

    if ! container volume inspect "$name" >/dev/null 2>&1; then
        container volume create -s "$size" "$name" >/dev/null
    fi
}

if [ "$#" -eq 0 ]; then
    set -- bash
fi

create_volume "$CARGO_REGISTRY_VOLUME" 8g
create_volume "$CARGO_GIT_VOLUME" 4g
create_volume "$TARGET_VOLUME" 40g
create_volume "$UV_CACHE_VOLUME" 8g

exec container run --rm \
    --dns "$DNS" \
    --cpus "$CPUS" \
    --memory "$MEMORY" \
    -e CARGO_NET_GIT_FETCH_WITH_CLI=true \
    -e UV_LINK_MODE="${UV_LINK_MODE:-copy}" \
    -v "$REPO_ROOT:/workspace" \
    -v "$CARGO_REGISTRY_VOLUME:/usr/local/cargo/registry" \
    -v "$CARGO_GIT_VOLUME:/usr/local/cargo/git" \
    -v "$TARGET_VOLUME:/workspace/target" \
    -v "$UV_CACHE_VOLUME:/root/.cache/uv" \
    -w /workspace \
    "$IMAGE" \
    "$@"
