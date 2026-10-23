#!/usr/bin/env python3
"""Replace a Python installation at one executable path and compare cached reuse.

The two input roots must be relocatable Python installations containing
`bin/python3`. They are copied into a new work directory; the originals are never
modified. A local pure-Python wheel makes the dependency resolution identical.

    python scripts/research/environment_interpreter_identity.py \
        --uv target/debug/uv --before-root /path/to/python-3.12 \
        --after-root /path/to/python-3.14 --work-dir work/interpreter-identity

The oracle requires the reused environment to import the package and report the
same Python version and package contents as a fresh cached environment.
Repeated runs must reuse the same environment. Restoring the original interpreter
must select its original cached environment and agree with a fresh installation.
"""

import argparse
import hashlib
import json
import shutil
import subprocess
import zipfile
from pathlib import Path


def replace_python(source: Path, destination: Path) -> Path:
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source, destination, symlinks=True)
    executable = destination / "bin/uv-probe-python"
    shutil.copy2((destination / "bin/python3").resolve(), executable)
    return executable


def wheel(root: Path) -> Path:
    path = root / "env_probe-1.0.0-py3-none-any.whl"
    files = {
        "env_probe.py": b"VALUE = 'cached dependency'\n",
        "env_probe-1.0.0.dist-info/METADATA": (
            b"Metadata-Version: 2.1\nName: env-probe\nVersion: 1.0.0\n"
        ),
        "env_probe-1.0.0.dist-info/WHEEL": (
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
        ),
    }
    files["env_probe-1.0.0.dist-info/RECORD"] = (
        "".join(f"{name},,\n" for name in files)
        + "env_probe-1.0.0.dist-info/RECORD,,\n"
    ).encode()
    with zipfile.ZipFile(path, "w") as archive:
        for name, contents in files.items():
            archive.writestr(name, contents)
    return path


def run(uv: str, executable: Path, artifact: Path, cache: Path) -> dict:
    probe = (
        "import env_probe, json, sys; "
        "print(json.dumps({'version': sys.version, 'value': env_probe.VALUE, "
        "'prefix': sys.prefix, 'module': env_probe.__file__}))"
    )
    result = subprocess.run(
        [
            uv,
            "--no-config",
            "--cache-dir",
            str(cache),
            "run",
            "--no-project",
            "--no-python-downloads",
            "--python",
            str(executable),
            "--no-index",
            "--with",
            str(artifact),
            "python",
            "-c",
            probe,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return {
        "returncode": result.returncode,
        "probe": json.loads(result.stdout) if result.returncode == 0 else None,
        "stderr": result.stderr,
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


def equivalent(left: dict, right: dict) -> bool:
    return left["returncode"] == right["returncode"] == 0 and all(
        left["probe"][key] == right["probe"][key] for key in ("version", "value")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--before-root", required=True, type=Path)
    parser.add_argument("--after-root", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--expect", choices=["correct", "stale"], default="correct")
    args = parser.parse_args()
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    for source in (args.before_root, args.after_root):
        if not (source / "bin/python3").is_file():
            parser.error(f"missing Python executable: {source / 'bin/python3'}")
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    artifact = wheel(root)
    executable = replace_python(args.before_root.resolve(), root / "python")
    canonical_before = executable.resolve()
    before = run(uv, executable, artifact, root / "shared-cache")
    if before["returncode"]:
        raise RuntimeError(f"initial environment failed: {before}")
    unchanged = run(uv, executable, artifact, root / "shared-cache")
    assert equivalent(before, unchanged), "unchanged interpreter changed the result"
    assert unchanged["probe"]["module"] == before["probe"]["module"], (
        "unchanged interpreter created a different environment"
    )

    executable = replace_python(args.after_root.resolve(), root / "python")
    assert executable.resolve() == canonical_before, "canonical executable path changed"
    reused = run(uv, executable, artifact, root / "shared-cache")
    fresh = run(uv, executable, artifact, root / "fresh-cache")
    if fresh["returncode"]:
        raise RuntimeError(f"fresh environment failed: {fresh}")
    assert before["probe"]["version"] != fresh["probe"]["version"], (
        "the Python versions must differ"
    )
    environment_changed = (
        reused["returncode"] == 0
        and reused["probe"]["module"] != before["probe"]["module"]
    )
    correct = equivalent(reused, fresh) and environment_changed
    repeated = run(uv, executable, artifact, root / "shared-cache")
    assert repeated["returncode"] == reused["returncode"]
    if correct:
        assert equivalent(repeated, fresh)
        assert repeated["probe"]["module"] == reused["probe"]["module"], (
            "replacement interpreter did not reuse its environment"
        )

    executable = replace_python(args.before_root.resolve(), root / "python")
    assert executable.resolve() == canonical_before, "restored executable path changed"
    restored = run(uv, executable, artifact, root / "shared-cache")
    restored_fresh = run(uv, executable, artifact, root / "restored-fresh-cache")
    assert equivalent(restored, restored_fresh), (
        "restored interpreter differs from fresh"
    )
    assert equivalent(restored, before), (
        "restored interpreter changed the package result"
    )
    assert restored["probe"]["module"] == before["probe"]["module"], (
        "restored interpreter did not reuse its original cached environment"
    )
    print(
        json.dumps(
            {
                "uv": binary_identity(uv),
                "canonical_executable": str(canonical_before),
                "before": before,
                "unchanged": unchanged,
                "reused": reused,
                "fresh": fresh,
                "repeated": repeated,
                "restored": restored,
                "restored_fresh": restored_fresh,
                "unchanged_and_restored_reuse_original_environment": True,
                "replacement_changes_cached_environment": environment_changed,
                "reuse_matches_fresh": correct,
            },
            indent=2,
        )
    )
    return 0 if (correct if args.expect == "correct" else not correct) else 1


if __name__ == "__main__":
    raise SystemExit(main())
