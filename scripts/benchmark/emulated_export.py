"""Compare native and QEMU user-mode execution of the same `uv` release on x86-64 Linux.

Supply the static x86-64 and 64-bit RISC-V Linux binaries and a locked project.
Both QEMU binaries should come from the same build. Each timed subprocess
includes process startup. No guest kernel is booted.

    python3 emulated_export.py PROJECT X86_UV RISCV_UV --scratch .
"""

import argparse
import hashlib
import itertools
import json
import os
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter_ns


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("project", type=Path)
    parser.add_argument("x86_uv", type=Path)
    parser.add_argument("riscv_uv", type=Path)
    parser.add_argument("--qemu-x86", default="qemu-x86_64")
    parser.add_argument("--qemu-riscv", default="qemu-riscv64")
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=12)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    x86 = args.x86_uv.resolve(strict=True)
    modes = {
        "native_x86": [x86],
        "qemu_x86": [args.qemu_x86, x86],
        "qemu_riscv": [args.qemu_riscv, args.riscv_uv.resolve(strict=True)],
    }
    options = {
        "version": ["--version"],
        "export": [
            "export",
            "--offline",
            "--frozen",
            "--no-header",
            "--no-hashes",
            "--no-emit-project",
        ],
    }
    orders = list(itertools.permutations(modes))
    samples = []
    with TemporaryDirectory(dir=args.scratch.resolve()) as cache:
        env = {
            **os.environ,
            "UV_CACHE_DIR": cache,
            "UV_PYTHON_DOWNLOADS": "never",
            "UV_NO_CONFIG": "1",
            "UV_NO_PROGRESS": "1",
            "RAYON_NUM_THREADS": "1",
        }
        reference = {}
        for index in range(-1, args.rounds):
            for mode in orders[index % len(orders)]:
                for operation, flags in options.items():
                    start = perf_counter_ns()
                    proc = subprocess.run(
                        [*modes[mode], *flags],
                        cwd=args.project,
                        env=env,
                        check=True,
                        capture_output=True,
                        timeout=60,
                    )
                    elapsed = (perf_counter_ns() - start) / 1e6
                    if proc.stderr:
                        raise ValueError(proc.stderr.decode(errors="replace"))
                    # Release binaries include the target triple in version output.
                    value = (
                        b" ".join(proc.stdout.split()[:2])
                        if operation == "version"
                        else proc.stdout
                    )
                    if not value or reference.setdefault(operation, value) != value:
                        raise ValueError(
                            f"{mode} produced different {operation} output"
                        )
                    if index >= 0:
                        samples.append(
                            {
                                "round": index + 1,
                                "mode": mode,
                                "operation": operation,
                                "ms": elapsed,
                            }
                        )
    print(
        json.dumps(
            {
                "version": reference["version"].decode(),
                "export_sha256": hashlib.sha256(reference["export"]).hexdigest(),
                "export_bytes": len(reference["export"]),
                "samples": samples,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
