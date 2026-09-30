#!/bin/sh
set -eu

exec "$UV_DIAGNOSTIC_UV" run --no-config --no-project --no-python-downloads \
    --python "$UV_DIAGNOSTIC_PYTHON" "$UV_DIAGNOSTIC_SCRIPT" rustc -- "$@"
