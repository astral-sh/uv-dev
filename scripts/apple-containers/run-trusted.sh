#!/bin/bash
# Run trusted uv reproductions in Apple's container runtime.
#
# This mode is for reproductions from sources we trust enough to allow normal
# network access and a writable ephemeral container filesystem. It does not
# mount the repository by default; it only exposes the local uv build output
# volume at /target:ro. Use run-dev.sh when you want the full checkout and
# Cargo caches.

set -euo pipefail

IMAGE="${UV_TRUSTED_IMAGE:-ghcr.io/astral-sh/uv:python3.12-trixie-slim}"
DNS="${UV_TRUSTED_DNS:-1.1.1.1}"
CPUS="${UV_TRUSTED_CPUS:-4}"
MEMORY="${UV_TRUSTED_MEMORY:-8g}"
UID_GID="${UV_TRUSTED_UID_GID:-}"
TARGET_VOLUME="${UV_APPLE_CONTAINER_TARGET_VOLUME:-uv-apple-target}"

if [ "$#" -eq 0 ]; then
    set -- sh
fi

MOUNTS=(
    --mount "type=volume,source=$TARGET_VOLUME,target=/target,readonly"
)

if [ -n "${UV_TRUSTED_INPUT_DIR:-}" ]; then
    MOUNTS+=(--mount "type=bind,source=$UV_TRUSTED_INPUT_DIR,target=/input,readonly")
fi

if [ -n "${UV_TRUSTED_OUTPUT_DIR:-}" ]; then
    mkdir -p "$UV_TRUSTED_OUTPUT_DIR"
    MOUNTS+=(--mount "type=bind,source=$UV_TRUSTED_OUTPUT_DIR,target=/output")
fi

RUN_ARGS=(
    --dns "$DNS"
    --cpus "$CPUS"
    --memory "$MEMORY"
    --cap-drop ALL
)

if [ -n "$UID_GID" ]; then
    RUN_ARGS+=(--uid "$UID_GID" --gid "$UID_GID")
fi

# shellcheck disable=SC2016
container run --rm \
    "${RUN_ARGS[@]}" \
    -e "HOME=/tmp/home" \
    -e "PATH=/target/debug:/usr/local/bin:/usr/bin:/bin" \
    -e "UV_CACHE_DIR=/tmp/uv-cache" \
    -e "UV_LINK_MODE=copy" \
    "${MOUNTS[@]}" \
    -w /tmp \
    "$IMAGE" \
    sh -c 'mkdir -p "$HOME" "$UV_CACHE_DIR" /tmp/work && cd /tmp/work && exec "$@"' \
    -- "$@"
