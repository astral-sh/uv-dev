#!/usr/bin/env bash
## Verify that all release artifacts contain cargo-auditable SBOM data.
##
## Requires:
##   cargo install rust-audit-info --locked
##
## Usage:
##   scripts/check-release-artifact-sboms.sh <run-id>

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <github-actions-run-id>" >&2
    exit 1
fi

missing=""
command -v gh >/dev/null 2>&1 || missing="$missing gh"
command -v rust-audit-info >/dev/null 2>&1 || missing="$missing rust-audit-info"

if [ -n "$missing" ]; then
    echo "error: missing required tools:$missing" >&2
    exit 1
fi

RUN_ID="$1"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

PASS=0
FAIL=0
ARTIFACTS=0

pass() { echo "PASS $1"; PASS=$((PASS + 1)); }
fail() { echo "FAIL $1"; FAIL=$((FAIL + 1)); }

check() {
    local binary="$1"
    local label="$2"
    if rust-audit-info "$binary" >/dev/null 2>&1; then
        pass "$label"
    else
        fail "$label"
    fi
}

echo "Fetching artifacts for run $RUN_ID..."
ALL_ARTIFACTS=$(gh api "repos/{owner}/{repo}/actions/runs/$RUN_ID/artifacts" \
    --paginate --jq '.artifacts[].name')

echo ""

for artifact in $ALL_ARTIFACTS; do
    case "$artifact" in
        build-github-archives-*) ;;
        *) continue ;;
    esac
    ARTIFACTS=$((ARTIFACTS + 1))

    dest="$WORKDIR/$artifact"
    gh run download "$RUN_ID" -n "$artifact" -D "$dest"

    # Each target contributes exactly one GitHub release archive.
    archives=()
    for f in "$dest"/*.tar.gz "$dest"/*.zip; do
        [ -f "$f" ] && archives+=("$f")
    done
    if [ "${#archives[@]}" -ne 1 ]; then
        fail "$artifact / expected one release archive, found ${#archives[@]}"
        continue
    fi
    archive="${archives[0]}"
    # Only inspect binaries extracted from the selected release archive.
    extracted="$WORKDIR/extracted-$ARTIFACTS"
    mkdir "$extracted"
    case "$archive" in
        *.tar.gz) tar xzf "$archive" -C "$extracted" ;;
        *.zip) unzip -qo "$archive" -d "$extracted" ;;
    esac
    archive=$(basename "$archive")

    # Check uv and uvx binaries.
    for bin in uv uvx; do
        binary=$(find "$extracted" \( -name "$bin" -o -name "$bin.exe" \) -type f | head -1)
        if [ -n "$binary" ]; then
            check "$binary" "$archive / $(basename "$binary")"
        else
            fail "$archive / missing $bin"
        fi
    done
done

if [ "$ARTIFACTS" -eq 0 ]; then
    echo "error: No GitHub release archive artifacts found in run $RUN_ID" >&2
    exit 1
fi

echo ""
echo "PASS $PASS / FAIL $FAIL"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
