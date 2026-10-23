#!/usr/bin/env python3
"""Compare warm and fresh builds after deleting a declared glob input.

    python scripts/research/source_cache_membership.py --uv target/debug/uv \
        --work-dir work/source-membership

The local backend has no build dependencies and embeds the declared input names
in its wheel. Removing the older input leaves the maximum file timestamp intact.
The oracle compares the installed package from the warm cache with a fresh build.
Unchanged and unmatched inputs must reuse the wheel; deleting the last match
must produce an empty payload and then remain reusable.
"""

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

BACKEND = r"""
import pathlib
import zipfile

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    inputs = sorted(path.name for path in pathlib.Path("inputs").glob("*.txt"))
    with pathlib.Path("invocations.log").open("a") as output:
        output.write(repr(inputs) + "\n")
    files = {
        "source_probe.py": f"INPUTS = {inputs!r}\n",
        "source_probe-1.0.0.dist-info/METADATA":
            "Metadata-Version: 2.1\nName: source-probe\nVersion: 1.0.0\n",
        "source_probe-1.0.0.dist-info/WHEEL":
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    }
    files["source_probe-1.0.0.dist-info/RECORD"] = (
        "".join(f"{name},,\n" for name in files)
        + "source_probe-1.0.0.dist-info/RECORD,,\n"
    )
    name = "source_probe-1.0.0-py3-none-any.whl"
    with zipfile.ZipFile(pathlib.Path(wheel_directory) / name, "w") as archive:
        for path, contents in files.items():
            archive.writestr(path, contents)
    return name
"""


def install(uv: str, project: Path, target: Path, cache: Path) -> dict:
    result = subprocess.run(
        [
            uv,
            "--no-config",
            "--cache-dir",
            str(cache),
            "pip",
            "install",
            "--no-index",
            "--no-deps",
            "--python",
            sys.executable,
            "--target",
            str(target),
            "--requirement",
            str(project.parent / "requirements.txt"),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    probe = subprocess.check_output(
        [
            sys.executable,
            "-c",
            (
                "import json, sys; sys.path.insert(0, sys.argv[1]); "
                "import source_probe; print(json.dumps(source_probe.INPUTS))"
            ),
            str(target),
        ],
        text=True,
    )
    return {
        "inputs": json.loads(probe),
        "stderr": result.stderr,
        "backend_invocations": len(
            (project / "invocations.log").read_text().splitlines()
        ),
    }


def binary_identity(executable: str) -> dict:
    with Path(executable).open("rb") as binary:
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    return {
        "path": str(Path(executable).resolve()),
        "sha256": digest,
        "version": subprocess.check_output(
            [executable, "--version"], text=True
        ).strip(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--expect", choices=["correct", "stale"], default="correct")
    args = parser.parse_args()
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / "project"
    (project / "inputs").mkdir(parents=True)
    (project / "pyproject.toml").write_text(
        textwrap.dedent(
            """\
            [project]
            name = "source-probe"
            version = "1.0.0"
            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]
            [tool.uv]
            cache-keys = ["inputs/*.txt"]
            """
        )
    )
    (project / "backend.py").write_text(BACKEND)
    # Command-line source directories force a rebuild, so request this dependency via a file.
    (root / "requirements.txt").write_text(f"source-probe @ {project.as_uri()}\n")
    older = project / "inputs/older.txt"
    older.write_text("older input")
    (project / "inputs/newer.txt").write_text("newer input")
    before = install(uv, project, root / "before", root / "shared-cache")
    unchanged = install(uv, project, root / "unchanged", root / "shared-cache")
    assert unchanged["inputs"] == before["inputs"]
    assert unchanged["backend_invocations"] == before["backend_invocations"]
    (project / "inputs/ignored.dat").write_text("not a declared input")
    unmatched = install(uv, project, root / "unmatched", root / "shared-cache")
    assert unmatched["inputs"] == before["inputs"]
    assert unmatched["backend_invocations"] == before["backend_invocations"]
    older.unlink()
    warm = install(uv, project, root / "warm", root / "shared-cache")
    fresh = install(uv, project, root / "fresh", root / "fresh-cache")
    assert before["inputs"] == ["newer.txt", "older.txt"]
    assert fresh["inputs"] == ["newer.txt"]
    correct = warm["inputs"] == fresh["inputs"]
    warm_again = install(uv, project, root / "warm-again", root / "shared-cache")
    assert warm_again["inputs"] == warm["inputs"]
    assert warm_again["backend_invocations"] == fresh["backend_invocations"]
    (project / "inputs/newer.txt").unlink()
    empty = install(uv, project, root / "empty", root / "shared-cache")
    empty_fresh = install(uv, project, root / "empty-fresh", root / "empty-cache")
    empty_again = install(uv, project, root / "empty-again", root / "shared-cache")
    assert empty["inputs"] == empty_fresh["inputs"] == empty_again["inputs"] == []
    assert empty_again["backend_invocations"] == empty_fresh["backend_invocations"]
    print(
        json.dumps(
            {
                "uv": binary_identity(uv),
                "before": before,
                "unchanged": unchanged,
                "unmatched": unmatched,
                "warm": warm,
                "fresh": fresh,
                "warm_again": warm_again,
                "empty": empty,
                "empty_fresh": empty_fresh,
                "empty_again": empty_again,
                "backend_invocations": len(
                    (project / "invocations.log").read_text().splitlines()
                ),
                "warm_matches_fresh": correct,
            },
            indent=2,
        )
    )
    return 0 if (correct if args.expect == "correct" else not correct) else 1


if __name__ == "__main__":
    raise SystemExit(main())
