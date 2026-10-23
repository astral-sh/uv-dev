"""Validate the identity fields of Python download metadata with uv's parser."""

import argparse
from pathlib import Path

from . import python_download_keys


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    args = parser.parse_args()
    try:
        keys = python_download_keys(args.metadata.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Validated {len(keys)} Python download keys")
