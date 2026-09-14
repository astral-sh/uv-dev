"""Prime real, unsatisfiable dependency requirements before CPU measurement."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

CUTOFF = "2024-12-01T00:00:00Z"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/debug/uv")
    args = parser.parse_args()
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    temporary_root = root / "target"
    temporary_root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=temporary_root) as temporary:
        requirements = Path(temporary) / "requirements.in"
        for fixture in json.loads(
            Path(__file__).with_name("resolver-errors.json").read_text()
        ):
            requirements.write_text("\n".join(fixture["requirements"]) + "\n")
            result = subprocess.run(
                [
                    str(args.uv.resolve()),
                    "--no-config",
                    "--cache-dir",
                    str(root / ".cache"),
                    "pip",
                    "compile",
                    "--python-version",
                    fixture["python"],
                    "--python-platform",
                    "aarch64-apple-darwin",
                    "--exclude-newer",
                    CUTOFF,
                    str(requirements),
                ],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode != 1 or "No solution found" not in result.stderr:
                raise RuntimeError(
                    f"Expected an unsatisfiable {fixture['name']} graph:\n{result.stderr}"
                )
            print(f"Primed {fixture['name']}")


if __name__ == "__main__":
    main()
