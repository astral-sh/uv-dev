#!/usr/bin/env -S uv run --no-cache --no-managed-python --no-python-downloads --python python3 --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Compatibility wrapper for the Apple container egress proxy."""

from __future__ import annotations

import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
TARGET = os.path.join(SCRIPT_DIR, "apple-containers", "egress-proxy.py")

os.execvp(
    "uv",
    [
        "uv",
        "run",
        "--no-cache",
        "--no-managed-python",
        "--no-python-downloads",
        "--python",
        "python3",
        "--script",
        TARGET,
        *sys.argv[1:],
    ],
)
