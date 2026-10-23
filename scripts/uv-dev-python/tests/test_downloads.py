import json
from pathlib import Path

import pytest
from uv_dev import python_download_keys


def test_catalog_aliases() -> None:
    metadata = {
        "riscv": {
            "name": "cpython",
            "arch": {"family": "riscv64", "variant": None},
            "os": "linux",
            "libc": "gnu",
            "major": 3,
            "minor": 15,
            "patch": 0,
            "prerelease": "rc2",
            "variant": "freethreaded+debug",
        },
        "macos": {
            "name": "cpython",
            "arch": {"family": "aarch64", "variant": None},
            "os": "darwin",
            "libc": "none",
            "major": 3,
            "minor": 14,
            "patch": 0,
        },
    }
    assert python_download_keys(json.dumps(metadata)) == {
        "riscv": "cpython-3.15.0rc2+freethreaded+debug-linux-riscv64gc-gnu",
        "macos": "cpython-3.14.0-macos-aarch64-none",
    }


def test_invalid_variant_reports_entry() -> None:
    metadata = {
        "broken": {
            "name": "cpython",
            "arch": {"family": "x86_64"},
            "os": "linux",
            "libc": "gnu",
            "major": 3,
            "minor": 14,
            "patch": 0,
            "variant": "unknown",
        }
    }
    with pytest.raises(
        ValueError, match="Invalid download `broken`: Unknown Python variant"
    ):
        python_download_keys(json.dumps(metadata))


def test_checked_in_catalog() -> None:
    root = Path(__file__).resolve().parents[3]
    path = root / "crates/uv-python-managed/download-metadata.json"
    metadata = path.read_text(encoding="utf-8")
    assert python_download_keys(metadata).keys() == json.loads(metadata).keys()
