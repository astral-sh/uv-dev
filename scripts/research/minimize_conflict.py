"""Budgeted root-conflict minimization with uv over a frozen local wheelhouse."""

import argparse
import base64
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import tempfile
import time
import zipfile
from email.parser import BytesParser
from pathlib import Path


class BudgetExceeded(Exception):
    pass


def demo_wheel(
    directory: Path,
    name: str,
    version: int,
    dependency: str | None = None,
    *,
    dist_info: str | None = None,
) -> None:
    info = dist_info or f"{name}-{version}.dist-info"
    metadata = f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\nRequires-Python: >=3.12\n"
    if dependency:
        metadata += f"Requires-Dist: {dependency}\n"
    files = {
        f"{info}/METADATA": metadata.encode(),
        f"{info}/WHEEL": b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
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
    with zipfile.ZipFile(
        directory / f"{name}-{version}-py3-none-any.whl", "w"
    ) as archive:
        for path, data in files.items():
            archive.writestr(zipfile.ZipInfo(path, (2020, 1, 1, 0, 0, 0)), data)


def fingerprint(directory: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(directory.iterdir()):
        digest.update(path.name.encode())
        with path.open("rb") as file:
            while chunk := file.read(64 * 1024):
                digest.update(chunk)
    return digest.hexdigest()


class Oracle:
    def __init__(
        self,
        uv: Path,
        directory: Path,
        roots: list[str],
        max_solves: int,
        seconds: float,
        background: list[str] | None = None,
    ):
        self.uv = uv
        self.directory = directory
        self.roots = roots
        self.background = background or []
        self.max_solves = max_solves
        self.deadline = time.monotonic() + seconds
        self.cache: dict[tuple[int, ...], bool] = {}
        self.solves = 0
        self.cache_hits = 0
        self.smallest_unsat: tuple[int, ...] | None = None
        self.environment = {
            key: value for key, value in os.environ.items() if not key.startswith("UV_")
        }
        self.environment.update(
            UV_NO_CONFIG="1",
            UV_PYTHON_DOWNLOADS="never",
            NO_COLOR="1",
            TMPDIR=str(directory),
        )

    def unsat(self, subset: tuple[int, ...]) -> bool:
        key = tuple(sorted(set(subset)))
        if key in self.cache:
            self.cache_hits += 1
            return self.cache[key]
        remaining = self.deadline - time.monotonic()
        if self.solves >= self.max_solves or remaining <= 0:
            raise BudgetExceeded
        requirements = self.directory / "requirements.in"
        requirements.write_text(
            "".join(
                f"{requirement}\n"
                for requirement in [
                    *self.background,
                    *(self.roots[index] for index in key),
                ]
            )
        )
        self.solves += 1
        try:
            result = subprocess.run(
                [
                    str(self.uv),
                    "pip",
                    "compile",
                    str(requirements),
                    "--offline",
                    "--no-index",
                    "--find-links",
                    str(self.directory / "wheels"),
                    "--no-build",
                    "--no-cache",
                    "--python-version",
                    "3.12",
                    "--python-platform",
                    "linux",
                    "--prerelease",
                    "allow",
                    "--color",
                    "never",
                ],
                cwd=self.directory,
                env=self.environment,
                capture_output=True,
                text=True,
                timeout=remaining,
                check=False,
            )
        except subprocess.TimeoutExpired as error:
            raise BudgetExceeded from error
        if result.returncode == 0:
            unsat = False
        elif any(
            diagnostic in result.stderr
            for diagnostic in (
                "invalid metadata",
                "inconsistent metadata",
                "could not be parsed",
                "invalid package format",
            )
        ):
            # uv can represent an unreadable candidate as unavailable in a no-solution report.
            # That is not evidence of a conflict in the intended frozen dependency model.
            raise RuntimeError(
                f"malformed candidate artifact prevents conflict classification:\n{result.stderr}"
            )
        elif (
            result.returncode == 1
            and "No solution found when resolving dependencies" in result.stderr
        ):
            unsat = True
        else:
            raise RuntimeError(
                f"uv failed without a recognized resolution conflict:\n{result.stderr}"
            )
        self.cache[key] = unsat
        if unsat and (
            self.smallest_unsat is None or len(key) < len(self.smallest_unsat)
        ):
            self.smallest_unsat = key
        return unsat

    def quickxplain(
        self, background: tuple[int, ...], candidates: tuple[int, ...]
    ) -> tuple[int, ...]:
        if not candidates or self.unsat(background):
            return ()
        if len(candidates) == 1:
            return candidates
        middle = len(candidates) // 2
        left, right = candidates[:middle], candidates[middle:]
        left_conflict = self.quickxplain(background + right, left)
        right_conflict = self.quickxplain(background + left_conflict, right)
        return left_conflict + right_conflict


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument(
        "--demo",
        nargs="?",
        const="single",
        choices=("single", "multiple", "invalid-metadata", "invalid-structure"),
    )
    source.add_argument("--requirements", type=Path)
    parser.add_argument("--wheelhouse", type=Path)
    parser.add_argument(
        "--background",
        type=Path,
        help="Fixed requirements present in every solve and excluded from minimization",
    )
    parser.add_argument("--max-solves", type=int, default=64)
    parser.add_argument("--seconds", type=float, default=30)
    args = parser.parse_args()
    if args.max_solves < 1 or not math.isfinite(args.seconds) or args.seconds <= 0:
        parser.error("budgets must be positive")
    if args.requirements and not args.wheelhouse:
        parser.error("--requirements requires --wheelhouse")
    if args.demo and args.wheelhouse:
        parser.error("--demo does not accept --wheelhouse")
    uv = args.uv.resolve(strict=True)
    with uv.open("rb") as executable:
        binary_sha256 = hashlib.file_digest(executable, "sha256").hexdigest()
    args.work_dir = args.work_dir.resolve()
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="root-conflict-", dir=args.work_dir
    ) as temporary:
        directory = Path(temporary)
        wheelhouse = directory / "wheels"
        wheelhouse.mkdir()
        if args.demo:
            roots = ["noise1==1", "left==1", "noise2==1", "right==1"]
            demo_wheel(wheelhouse, "left", 1, "common==1")
            demo_wheel(wheelhouse, "right", 1, "common==2")
            demo_wheel(wheelhouse, "common", 1)
            demo_wheel(wheelhouse, "common", 2)
            demo_wheel(wheelhouse, "noise1", 1)
            demo_wheel(wheelhouse, "noise2", 1)
            if args.demo == "multiple":
                demo_wheel(wheelhouse, "otherleft", 1, "other==1")
                demo_wheel(wheelhouse, "otherright", 1, "other==2")
                demo_wheel(wheelhouse, "other", 1)
                demo_wheel(wheelhouse, "other", 2)
                roots.extend(["otherleft==1", "otherright==1"])
            elif args.demo == "invalid-metadata":
                demo_wheel(wheelhouse, "broken", 1, "??invalid??")
                roots = ["broken==1"]
            elif args.demo == "invalid-structure":
                demo_wheel(wheelhouse, "broken", 1, dist_info="wrong-1.dist-info")
                roots = ["broken==1"]
        else:
            roots = [
                line.strip()
                for line in args.requirements.read_text().splitlines()
                if line.strip() and not line.lstrip().startswith("#")
            ]
            for path in sorted(args.wheelhouse.glob("*.whl")):
                shutil.copyfile(path, wheelhouse / path.name)
        background = (
            []
            if args.background is None
            else [
                line.strip()
                for line in args.background.read_text().splitlines()
                if line.strip() and not line.lstrip().startswith("#")
            ]
        )
        if not roots or len(roots) > 256 or len(set(roots)) != len(roots):
            parser.error("provide between 1 and 256 distinct root requirements")
        if any(
            re.fullmatch(
                r"[A-Za-z0-9][A-Za-z0-9_.-]*(?:[<>=!~][A-Za-z0-9.*+,!<>=~ -]+)?", root
            )
            is None
            for root in [*roots, *background]
        ):
            parser.error(
                "roots must be package names with optional version specifiers; no options, markers, extras, or URLs"
            )
        if not any(wheelhouse.iterdir()):
            parser.error("wheelhouse is empty")
        for path in wheelhouse.iterdir():
            with zipfile.ZipFile(path) as archive:
                metadata_paths = [
                    name
                    for name in archive.namelist()
                    if name.endswith(".dist-info/METADATA")
                ]
                if len(metadata_paths) != 1:
                    parser.error(f"expected exactly one METADATA in {path.name}")
                metadata = BytesParser().parsebytes(archive.read(metadata_paths[0]))
                if any("@" in value for value in metadata.get_all("Requires-Dist", [])):
                    parser.error(
                        "direct references in wheel dependencies are outside the frozen corpus"
                    )
        digest = fingerprint(wheelhouse)
        oracle = Oracle(uv, directory, roots, args.max_solves, args.seconds, background)
        started = time.monotonic()
        status = "budget_exhausted"
        minimal = False
        conflict = None
        background_consistent = None
        try:
            all_roots = tuple(range(len(roots)))
            background_consistent = not oracle.unsat(())
            if not background_consistent:
                status = "background_conflict"
                conflict = ()
            elif not oracle.unsat(all_roots):
                status = "satisfiable"
            else:
                conflict = oracle.quickxplain((), all_roots)
                if not oracle.unsat(conflict):
                    raise RuntimeError("minimizer returned a satisfiable set")
                for index in range(len(conflict)):
                    if oracle.unsat(conflict[:index] + conflict[index + 1 :]):
                        raise RuntimeError("minimizer returned a nonminimal set")
                minimal = True
                status = "minimal_conflict"
        except BudgetExceeded:
            conflict = oracle.smallest_unsat
        if fingerprint(wheelhouse) != digest:
            raise RuntimeError("frozen wheelhouse changed during minimization")
        print(
            json.dumps(
                {
                    "uv": str(uv),
                    "binary_sha256": binary_sha256,
                    "version": subprocess.check_output(
                        [str(uv), "--version"], text=True, env=oracle.environment
                    ).strip(),
                    "fixture_sha256": digest,
                    "python_version": "3.12",
                    "python_platform": "linux",
                    "prerelease": "allow",
                    "max_solves": args.max_solves,
                    "seconds": args.seconds,
                    "status": status,
                    "subset_minimal": minimal,
                    "roots": roots,
                    "background": background,
                    "background_consistent": background_consistent,
                    "conflict": None
                    if conflict is None
                    else [roots[index] for index in conflict],
                    "solves": oracle.solves,
                    "cache_hits": oracle.cache_hits,
                    "elapsed_seconds": time.monotonic() - started,
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
