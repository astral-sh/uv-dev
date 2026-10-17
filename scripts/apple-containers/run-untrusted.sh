#!/bin/bash
# Run untrusted uv reproductions in an Apple container with allowlisted egress.
#
# By default, this mounts only the local uv build output volume at /target:ro,
# then runs in tmpfs-backed scratch space. To expose reproduction files, pass
# UV_UNTRUSTED_INPUT_DIR=/path/to/repro; it will be mounted read-only at /input.
# To collect generated output, pass UV_UNTRUSTED_OUTPUT_DIR=/path/to/output.
#
# The network is host-only. HTTP(S) egress must go through the local proxy, which
# defaults to allowing PyPI downloads only:
#
#   UV_UNTRUSTED_ALLOWED_DOMAINS=pypi.org,files.pythonhosted.org,releases.astral.sh \
#     ./scripts/apple-containers/run-untrusted.sh sh -c 'uv venv && uv pip install pytest'

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

IMAGE="${UV_UNTRUSTED_IMAGE:-ghcr.io/astral-sh/uv:python3.12-trixie-slim}"
NETWORK="${UV_UNTRUSTED_NETWORK:-uv-apple-untrusted}"
PROXY_HOST="${UV_UNTRUSTED_PROXY_HOST:-0.0.0.0}"
ALLOWED_DOMAINS="${UV_UNTRUSTED_ALLOWED_DOMAINS:-pypi.org,files.pythonhosted.org,releases.astral.sh}"
DENIED_DOMAINS="${UV_UNTRUSTED_DENIED_DOMAINS:-openai.com,*.openai.com,oaiusercontent.com,*.oaiusercontent.com}"
ALLOWED_PORTS="${UV_UNTRUSTED_ALLOWED_PORTS:-80,443}"
CPUS="${UV_UNTRUSTED_CPUS:-4}"
MEMORY="${UV_UNTRUSTED_MEMORY:-8g}"
UID_GID="${UV_UNTRUSTED_UID_GID:-65532}"
TARGET_VOLUME="${UV_APPLE_CONTAINER_TARGET_VOLUME:-uv-apple-target}"

if [ "$#" -eq 0 ]; then
    set -- sh
fi

if ! command -v uv >/dev/null 2>&1; then
    echo "uv is required to run egress-proxy.py as a PEP 723 script" >&2
    exit 1
fi

NETWORK_JSON="$(container network inspect "$NETWORK")"
if ! python3 -c 'import json, sys; raise SystemExit(0 if json.load(sys.stdin) else 1)' <<<"$NETWORK_JSON"; then
    container network create --internal "$NETWORK" >/dev/null
    NETWORK_JSON="$(container network inspect "$NETWORK")"
fi

GATEWAY="$(
    python3 -c 'import json, sys; print(json.load(sys.stdin)[0]["status"]["ipv4Gateway"])' <<<"$NETWORK_JSON"
)"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/uv-apple-untrusted.XXXXXX")"
READY_FILE="$TMP_DIR/proxy-ready.json"
PROXY_LOG="$TMP_DIR/proxy.log"

cleanup() {
    local status=$?
    if [ -n "${PROXY_PID:-}" ] && kill -0 "$PROXY_PID" 2>/dev/null; then
        kill "$PROXY_PID" 2>/dev/null || true
        wait "$PROXY_PID" 2>/dev/null || true
    fi
    if [ "${UV_UNTRUSTED_SHOW_PROXY_LOG:-0}" = "1" ] && [ -s "$PROXY_LOG" ]; then
        sed 's/^/[egress] /' "$PROXY_LOG" >&2
    fi
    rm -rf "$TMP_DIR"
    exit "$status"
}
trap cleanup EXIT INT TERM

uv run --no-cache --no-managed-python --no-python-downloads --python python3 --script "$SCRIPT_DIR/egress-proxy.py" \
    --host "$PROXY_HOST" \
    --port 0 \
    --allow "$ALLOWED_DOMAINS" \
    --deny "$DENIED_DOMAINS" \
    --ports "$ALLOWED_PORTS" \
    --ready-file "$READY_FILE" \
    >"$PROXY_LOG" 2>&1 &
PROXY_PID=$!

for _ in {1..100}; do
    if [ -s "$READY_FILE" ]; then
        break
    fi
    if ! kill -0 "$PROXY_PID" 2>/dev/null; then
        cat "$PROXY_LOG" >&2
        exit 1
    fi
    sleep 0.05
done

if [ ! -s "$READY_FILE" ]; then
    echo "Timed out waiting for egress proxy to start" >&2
    cat "$PROXY_LOG" >&2
    exit 1
fi

PROXY_PORT="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["port"])' "$READY_FILE")"
PROXY_URL="http://$GATEWAY:$PROXY_PORT"

MOUNTS=(
    --mount "type=volume,source=$TARGET_VOLUME,target=/target,readonly"
)

if [ -n "${UV_UNTRUSTED_INPUT_DIR:-}" ]; then
    MOUNTS+=(--mount "type=bind,source=$UV_UNTRUSTED_INPUT_DIR,target=/input,readonly")
fi

if [ -n "${UV_UNTRUSTED_OUTPUT_DIR:-}" ]; then
    mkdir -p "$UV_UNTRUSTED_OUTPUT_DIR"
    MOUNTS+=(--mount "type=bind,source=$UV_UNTRUSTED_OUTPUT_DIR,target=/output")
fi

# shellcheck disable=SC2016
container run --rm \
    --network "$NETWORK" \
    --cpus "$CPUS" \
    --memory "$MEMORY" \
    --cap-drop ALL \
    --read-only \
    --tmpfs /tmp \
    --uid "$UID_GID" \
    --gid "$UID_GID" \
    -e "HOME=/tmp/home" \
    -e "PATH=/target/debug:/usr/local/bin:/usr/bin:/bin" \
    -e "HTTP_PROXY=$PROXY_URL" \
    -e "HTTPS_PROXY=$PROXY_URL" \
    -e "http_proxy=$PROXY_URL" \
    -e "https_proxy=$PROXY_URL" \
    -e "NO_PROXY=localhost,127.0.0.1,::1" \
    -e "no_proxy=localhost,127.0.0.1,::1" \
    -e "UV_CACHE_DIR=/tmp/uv-cache" \
    -e "UV_LINK_MODE=copy" \
    "${MOUNTS[@]}" \
    -w /tmp \
    "$IMAGE" \
    sh -c 'mkdir -p "$HOME" "$UV_CACHE_DIR" /tmp/work && cd /tmp/work && exec "$@"' \
    -- "$@"
