"""Measure universal resolver split clones over a frozen, wheel-only fixture."""

import argparse
import base64
import hashlib
import json
import os
import re
import statistics
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path


def wheel(directory: Path, name: str, version: int, dependencies: list[str]) -> None:
    info = f"{name}-{version}.dist-info"
    metadata = f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n"
    metadata += "Requires-Python: >=3.12\n"
    metadata += "".join(f"Requires-Dist: {dependency}\n" for dependency in dependencies)
    files = {
        f"{info}/METADATA": metadata.encode(),
        f"{info}/WHEEL": b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    }
    record = ""
    for name_in_wheel, data in files.items():
        digest = (
            base64.urlsafe_b64encode(hashlib.sha256(data).digest())
            .rstrip(b"=")
            .decode()
        )
        record += f"{name_in_wheel},sha256={digest},{len(data)}\n"
    files[f"{info}/RECORD"] = f"{record}{info}/RECORD,,\n".encode()
    with zipfile.ZipFile(
        directory / f"{name}-{version}-py3-none-any.whl", "w"
    ) as archive:
        for name_in_wheel, data in files.items():
            archive.writestr(
                zipfile.ZipInfo(name_in_wheel, (2020, 1, 1, 0, 0, 0)), data
            )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--forks", type=int, default=8)
    parser.add_argument("--depth", type=int, default=32)
    parser.add_argument("--repeat", type=int, default=3)
    args = parser.parse_args()
    if (
        min(args.forks, args.depth, args.repeat) < 1
        or args.forks > 128
        or args.depth > 512
    ):
        parser.error("positive counts are required; forks <= 128 and depth <= 512")
    args.work_dir = args.work_dir.resolve()
    args.work_dir.mkdir(parents=True, exist_ok=True)
    uv = args.uv.resolve(strict=True)
    with uv.open("rb") as executable:
        binary_sha256 = hashlib.file_digest(executable, "sha256").hexdigest()
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    environment.update(UV_NO_CONFIG="1", UV_PYTHON_DOWNLOADS="never", NO_COLOR="1")
    with tempfile.TemporaryDirectory(
        prefix="fork-clone-", dir=args.work_dir
    ) as temporary:
        directory = Path(temporary)
        environment["TMPDIR"] = str(directory)
        wheelhouse = directory / "wheels"
        wheelhouse.mkdir()
        for index in range(args.depth):
            dependency = (
                f"chain{index + 1}==1" if index + 1 < args.depth else "brancher==1"
            )
            wheel(wheelhouse, f"chain{index}", 1, [dependency])
        wheel(
            wheelhouse,
            "brancher",
            1,
            [
                f"selected=={index + 1}; sys_platform == 'platform{index}'"
                for index in range(args.forks)
            ],
        )
        for version in range(1, args.forks + 1):
            wheel(wheelhouse, "selected", version, [])
        requirements = directory / "requirements.in"
        requirements.write_text("chain0==1\n")
        fingerprint = hashlib.sha256()
        for path in sorted(wheelhouse.iterdir()):
            fingerprint.update(path.name.encode())
            fingerprint.update(path.read_bytes())
        command = [
            str(uv),
            "pip",
            "compile",
            str(requirements),
            "--universal",
            "--python-version",
            "3.12",
            "--offline",
            "--no-index",
            "--find-links",
            str(wheelhouse),
            "--no-build",
            "--no-cache",
            "--no-header",
            "--no-annotate",
            "--color",
            "never",
            "-vv",
        ]
        runs = []
        for _ in range(args.repeat):
            started = time.perf_counter()
            result = subprocess.run(
                command,
                cwd=directory,
                env=environment,
                capture_output=True,
                text=True,
                timeout=120,
                check=False,
            )
            elapsed = time.perf_counter() - started
            if result.returncode:
                raise RuntimeError(result.stderr)
            events = []
            for line in result.stderr.splitlines():
                if "Cloned resolver fork state" in line:
                    fields = dict(
                        re.findall(
                            r"(clone_us|tracked_packages|incompatibility_references)=(\d+)",
                            line,
                        )
                    )
                    if len(fields) != 3:
                        raise RuntimeError(f"unrecognized clone event: {line}")
                    events.append({key: int(value) for key, value in fields.items()})
            if not events:
                raise RuntimeError(
                    "no clone events; use a uv binary built with fork-clone instrumentation"
                )
            runs.append(
                {
                    "elapsed_seconds": elapsed,
                    "clone_count": len(events),
                    "clone_microseconds": sum(event["clone_us"] for event in events),
                    "clones": events,
                    "solution_sha256": hashlib.sha256(
                        result.stdout.encode()
                    ).hexdigest(),
                }
            )
        version = subprocess.check_output(
            [str(uv), "--version"], text=True, env=environment
        ).strip()
        print(
            json.dumps(
                {
                    "uv": str(uv),
                    "binary_sha256": binary_sha256,
                    "version": version,
                    "forks": args.forks,
                    "depth": args.depth,
                    "fixture_sha256": fingerprint.hexdigest(),
                    "runs": runs,
                    "median_elapsed_seconds": statistics.median(
                        run["elapsed_seconds"] for run in runs
                    ),
                    "scope": "Two resolver split sites; debug logging enabled; references are not unique incompatibilities or bytes.",
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
