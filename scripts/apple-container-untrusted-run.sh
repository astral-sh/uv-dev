#!/bin/bash
# Compatibility wrapper for the Apple container untrusted runner.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$SCRIPT_DIR/apple-containers/run-untrusted.sh" "$@"
