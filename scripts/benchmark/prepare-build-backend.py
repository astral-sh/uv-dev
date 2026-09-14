"""Build the current uv_build wheel for offline PEP 517 benchmark workloads."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

MATURIN = "1.14.1"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    args = parser.parse_args()
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment.update(
        UV_PYTHON_INSTALL_DIR=str(root / ".cache/bench-python"),
        UV_PYTHON_DOWNLOADS="never",
    )
    subprocess.run(
        [
            str(args.uv.resolve()),
            "--no-config",
            "--cache-dir",
            str(root / ".cache"),
            "run",
            "--isolated",
            "--no-project",
            "--managed-python",
            "--python",
            "3.11.13",
            "--with",
            f"maturin=={MATURIN}",
            "maturin",
            "build",
            "--locked",
            "--profile",
            "profiling",
            "--manifest-path",
            str(root / "crates/uv-build/Cargo.toml"),
            "--out",
            str(root / ".cache/bench-build-backend"),
            *(["--compatibility", "linux"] if sys.platform == "linux" else []),
        ],
        cwd=root,
        env=environment,
        check=True,
    )


if __name__ == "__main__":
    main()
