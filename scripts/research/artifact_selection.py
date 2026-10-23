"""Compare actual uv wheel selection across tag, availability, and build-tag boundaries."""

import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import sysconfig
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Artifact:
    tag: str
    build: int | None = None

    @property
    def filename(self) -> str:
        build = "" if self.build is None else f"{self.build}-"
        return f"artifact-1-{build}{self.tag}.whl"

    def write(self, directory: Path) -> str:
        info = "artifact-1.dist-info"
        purelib = "true" if self.tag == "py3-none-any" else "false"
        wheel_metadata = (
            f"Wheel-Version: 1.0\nRoot-Is-Purelib: {purelib}\nTag: {self.tag}\n"
        )
        if self.build is not None:
            wheel_metadata += f"Build: {self.build}\n"
        files = {
            f"{info}/METADATA": b"Metadata-Version: 2.3\nName: artifact\nVersion: 1\nRequires-Python: >=3.11\n",
            f"{info}/WHEEL": wheel_metadata.encode(),
        }
        record = ""
        for path, data in files.items():
            digest = (
                base64.urlsafe_b64encode(hashlib.sha256(data).digest())
                .rstrip(b"=")
                .decode()
            )
            record += f"{path},sha256={digest},{len(data)}\n"
        files[f"{info}/RECORD"] = f"{record}{info}/RECORD,,\n".encode()
        path = directory / self.filename
        with zipfile.ZipFile(path, "w") as archive:
            for name, data in files.items():
                archive.writestr(zipfile.ZipInfo(name, (2020, 1, 1, 0, 0, 0)), data)
        return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    args = parser.parse_args()
    if sys.implementation.name != "cpython" or sysconfig.get_config_var(
        "Py_GIL_DISABLED"
    ):
        parser.error("Run this experiment with a GIL-enabled CPython interpreter")
    interpreter = Path(sys.executable).resolve(strict=True)
    uv = args.uv.resolve(strict=True)
    with uv.open("rb") as executable:
        binary_sha256 = hashlib.file_digest(executable, "sha256").hexdigest()
    args.work_dir = args.work_dir.resolve()
    args.work_dir.mkdir(parents=True, exist_ok=True)
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    environment.update(UV_NO_CONFIG="1", UV_PYTHON_DOWNLOADS="never", NO_COLOR="1")
    environment.pop("RUST_LOG", None)

    pure = Artifact("py3-none-any")
    glibc217 = Artifact("cp311-cp311-manylinux_2_17_x86_64")
    glibc228 = Artifact("cp311-cp311-manylinux_2_28_x86_64")
    stable = Artifact("cp311-abi3-manylinux_2_17_x86_64")
    arm = Artifact("cp311-cp311-manylinux_2_17_aarch64")
    build1 = Artifact(glibc228.tag, 1)
    build2 = Artifact(glibc228.tag, 2)
    stable9 = Artifact(stable.tag, 9)
    candidates = (pure, glibc217, glibc228, stable, arm)
    scenarios = [
        ("glibc-floor", "3.11", "x86_64-manylinux_2_17", candidates, glibc217),
        ("newer-glibc", "3.11", "x86_64-manylinux_2_28", candidates, glibc228),
        ("stable-abi", "3.12", "x86_64-manylinux_2_28", candidates, stable),
        (
            "preferred-absent",
            "3.11",
            "x86_64-manylinux_2_28",
            (pure, glibc217, stable, arm),
            glibc217,
        ),
        (
            "portable-fallback",
            "3.12",
            "x86_64-manylinux_2_28",
            (pure, glibc217, glibc228, arm),
            pure,
        ),
        ("no-compatible-artifact", "3.11", "x86_64-manylinux_2_28", (arm,), None),
        (
            "build-tag-tiebreak",
            "3.11",
            "x86_64-manylinux_2_28",
            (build1, build2, pure),
            build2,
        ),
        (
            "tag-priority-before-build",
            "3.11",
            "x86_64-manylinux_2_28",
            (build1, stable9, pure),
            build1,
        ),
    ]
    results = []
    with tempfile.TemporaryDirectory(
        prefix="artifact-selection-", dir=args.work_dir
    ) as temporary:
        directory = Path(temporary)
        environment["TMPDIR"] = str(directory)
        for name, python, platform, artifacts, expected in scenarios:
            case = directory / name
            wheels = case / "wheels"
            wheels.mkdir(parents=True)
            fingerprints = {
                artifact.filename: artifact.write(wheels) for artifact in artifacts
            }
            target = case / "target"
            result = subprocess.run(
                [
                    str(uv),
                    "pip",
                    "install",
                    "artifact==1",
                    "--dry-run",
                    "--target",
                    str(target),
                    "--python",
                    str(interpreter),
                    "--python-version",
                    python,
                    "--python-platform",
                    platform,
                    "--offline",
                    "--no-index",
                    "--find-links",
                    str(wheels),
                    "--no-build",
                    "--no-cache",
                    "--color",
                    "never",
                    "-vv",
                ],
                cwd=case,
                env=environment,
                text=True,
                capture_output=True,
                timeout=60,
                check=False,
            )
            selected = re.findall(
                r"Selecting: artifact==1 \[compatible\] \((artifact-[^\s)]+\.whl)\)",
                result.stderr,
            )
            if expected is None:
                if (
                    result.returncode != 1
                    or "No solution found when resolving dependencies"
                    not in result.stderr
                    or selected
                ):
                    raise RuntimeError(
                        f"{name}: expected a resolution conflict\n{result.stderr}"
                    )
            elif result.returncode or selected != [expected.filename]:
                raise RuntimeError(
                    f"{name}: expected {expected.filename}, selected {selected}\n{result.stderr}"
                )
            if any(path.name.endswith(".dist-info") for path in target.rglob("*")):
                raise RuntimeError(f"{name}: dry run installed package metadata")
            results.append(
                {
                    "scenario": name,
                    "python": python,
                    "platform": platform,
                    "artifact_sha256": fingerprints,
                    "selected": selected[0] if selected else None,
                    "expected": None if expected is None else expected.filename,
                    "status": "unsatisfiable" if expected is None else "selected",
                }
            )
        print(
            json.dumps(
                {
                    "uv": str(uv),
                    "binary_sha256": binary_sha256,
                    "version": subprocess.check_output(
                        [str(uv), "--version"], text=True, env=environment
                    ).strip(),
                    "interpreter": {
                        "executable": str(interpreter),
                        "version": sys.version,
                        "implementation": sys.implementation.name,
                        "gil_disabled": False,
                    },
                    "scenarios": results,
                    "scope": "Metadata-only wheels; actual resolver selection in dry-run mode; no artifact execution or runtime ABI proof.",
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
