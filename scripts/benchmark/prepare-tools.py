"""Prime pinned, realistic Python CLI environments for CodSpeed workloads."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

PYTHON = "3.12.11"
EXCLUDE_NEWER = "2025-07-01T00:00:00Z"


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, default=root / "target/profiling/uv")
    parser.add_argument("--refresh-locks", action="store_true")
    parser.add_argument("--individual-caches", action="store_true")
    args = parser.parse_args()
    cache = root / ".cache"
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("UV_") and name not in {"VIRTUAL_ENV", "CONDA_PREFIX"}
    }
    environment["UV_PYTHON_INSTALL_DIR"] = str(cache / "bench-python")
    environment["UV_PYTHON_DOWNLOADS"] = "never"
    command = [str(args.uv.resolve()), "--no-config"]
    workloads = json.loads(Path(__file__).with_name("tools.json").read_text())
    locks = Path(__file__).with_name("tool-locks")
    if args.refresh_locks:
        locks.mkdir(exist_ok=True)
        for workload in workloads:
            subprocess.run(
                [
                    *command,
                    "--cache-dir",
                    str(cache),
                    "pip",
                    "compile",
                    "--universal",
                    "--python-version",
                    "3.12",
                    "--no-build",
                    "--exclude-newer",
                    EXCLUDE_NEWER,
                    "--generate-hashes",
                    "--no-annotate",
                    "--custom-compile-command",
                    "python3 scripts/benchmark/prepare-tools.py --refresh-locks",
                    "--output-file",
                    str(locks / f"{workload['name']}.txt"),
                    "-",
                ],
                input=f"{workload['name']}=={workload['version']}\n",
                text=True,
                env=environment,
                stdout=subprocess.DEVNULL,
                check=True,
            )
        return

    temporary_root = root / "target"
    temporary_root.mkdir(exist_ok=True)
    selected = [(cache, workloads)]
    if args.individual_caches:
        for workload in workloads:
            if workload.get("cached-run"):
                tool_cache = cache / "bench-tool-caches" / workload["name"]
                if tool_cache.exists():
                    shutil.rmtree(tool_cache)
                selected.append((tool_cache, [workload]))
    for tool_cache, selected_tools in selected:
        with tempfile.TemporaryDirectory(
            prefix="bench-tools-", dir=temporary_root
        ) as temporary:
            tool_environment = environment | {
                "UV_TOOL_DIR": str(Path(temporary) / "tools"),
                "UV_TOOL_BIN_DIR": str(Path(temporary) / "bin"),
            }
            for workload in selected_tools:
                subprocess.run(
                    [
                        *command,
                        "--cache-dir",
                        str(tool_cache),
                        "tool",
                        "install",
                        "--no-build",
                        "--managed-python",
                        "--python",
                        PYTHON,
                        "--constraints",
                        str(locks / f"{workload['name']}.txt"),
                        f"{workload['name']}=={workload['version']}",
                    ],
                    env=tool_environment,
                    check=True,
                )


if __name__ == "__main__":
    main()
