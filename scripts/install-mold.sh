#!/usr/bin/env bash
# Install mold linker and make it the default linker.
#
# Retries on transient HTTP errors (e.g., 500) that the `rui314/setup-mold`
# GitHub Action does not handle.
#
# Verifies the downloaded archive against a repository-pinned SHA-256 digest.
# For ad hoc `MOLD_VERSION` overrides, set `MOLD_SHA256` explicitly.

set -euo pipefail

MOLD_VERSION="${MOLD_VERSION:-2.40.4}"
MOLD_SHA256="${MOLD_SHA256:-}"

arch="$(uname -m)"
archive="mold-${MOLD_VERSION}-${arch}-linux.tar.gz"
url="https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/${archive}"

# Release assets are mutable, so new versions require reviewed SHA-256 digests.
resolve_mold_sha256() {
    if [ -n "$MOLD_SHA256" ]; then
        printf '%s\n' "$MOLD_SHA256"
        return 0
    fi

    case "$archive" in
        mold-2.40.4-aarch64-linux.tar.gz)
            printf '%s\n' "c799b9ccae8728793da2186718fbe53b76400a9da396184fac0c64aa3298ec37"
            ;;
        mold-2.40.4-arm-linux.tar.gz)
            printf '%s\n' "d82792748a81202423ddd2496fc8719404fe694493abdef691cc080392ee44bf"
            ;;
        mold-2.40.4-loongarch64-linux.tar.gz)
            printf '%s\n' "b129c787d6825271c073b09e9751225dd4681593087ba43402c1dae781817425"
            ;;
        mold-2.40.4-ppc64le-linux.tar.gz)
            printf '%s\n' "81e6a2531d4e6b3a62163de04d63fc5f845a5f00ad13fde8b89856206c93a9f9"
            ;;
        mold-2.40.4-riscv64-linux.tar.gz)
            printf '%s\n' "3f9f1c8d69b05a81e799421f1b7b3f9be854b6d8a5417958ecc28de90803a1fe"
            ;;
        mold-2.40.4-s390x-linux.tar.gz)
            printf '%s\n' "79cc0a7e596dfbb8b05835f91222c24468278438369ec4a7afa70abb4a84158b"
            ;;
        mold-2.40.4-x86_64-linux.tar.gz)
            printf '%s\n' "4c999e19ffa31afa5aa429c679b665d5e2ca5a6b6832ad4b79668e8dcf3d8ec1"
            ;;
        *)
            echo "No pinned SHA-256 is available for ${archive}. Set MOLD_SHA256 to authenticate this download." >&2
            return 1
            ;;
    esac
}

if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
else
    SUDO="sudo"
fi

expected_sha256="$(resolve_mold_sha256)"

echo "Installing mold ${MOLD_VERSION} (${arch})..."

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

wget -O "$tmpdir/$archive" \
    --timeout=10 \
    --tries=5 \
    --waitretry=3 \
    --retry-connrefused \
    --retry-on-http-error=429,500,502,503,504 \
    --progress=dot:mega \
    "$url"

printf '%s  %s\n' "$expected_sha256" "$archive" >"$tmpdir/sha256sum.entry"
(cd "$tmpdir" && sha256sum --check --status sha256sum.entry)

$SUDO tar -C /usr/local --strip-components=1 --no-overwrite-dir -xzf "$tmpdir/$archive"

# Make mold the default linker
current_ld="$(realpath /usr/bin/ld)"
if [ "$current_ld" != /usr/local/bin/mold ]; then
    $SUDO ln -sf /usr/local/bin/mold "$current_ld"
fi

echo "mold ${MOLD_VERSION} installed successfully"
mold --version
