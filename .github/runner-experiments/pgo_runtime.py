"""Compare final PGO binaries with the same offline ecosystem workloads."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("pgo", ROOT / "scripts/build_uv_pgo.py")
assert spec is not None and spec.loader is not None
pgo = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = pgo
spec.loader.exec_module(pgo)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fat", required=True, type=Path)
    parser.add_argument("--thin", required=True, type=Path)
    parser.add_argument("--scratch", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    corpus = args.scratch.resolve() / "corpus"
    pgo.prepare_corpus(corpus)
    suffix = ".exe" if os.name == "nt" else ""
    binaries = {}
    for variant, directory in (("fat", args.fat), ("thin", args.thin)):
        found = list(directory.rglob(f"uv{suffix}"))
        if len(found) != 1:
            raise RuntimeError(f"Expected one uv executable in {directory}: {found}")
        binary = found[0].resolve()
        binaries[variant] = (binary, binary.with_name(f"uvx{suffix}"))

    records = []
    outputs = []
    provenance = {
        "python": sys.version,
        "source": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "binaries": {
            variant: {
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "version": subprocess.check_output(
                    [str(binary), "--version"], text=True
                ).strip(),
            }
            for variant, (binary, _) in binaries.items()
        },
    }
    current = {}

    def run(command, *, environment, allowed_exit_codes=(0,)):
        env = environment.copy()
        if current["round"]:
            env["UV_OFFLINE"] = "1"
        # Recreate mutable outputs, while retaining the already-populated metadata,
        # wheel, and managed-Python caches shared by both binaries.
        if len(command) > 1 and command[1] == "lock":
            project = Path(command[command.index("--project") + 1])
            (project / "uv.lock").unlink(missing_ok=True)
        if len(command) > 2 and command[1:3] == ["pip", "install"]:
            shutil.rmtree(command[command.index("--target") + 1], ignore_errors=True)
        if len(command) > 1 and command[1] == "sync":
            project = Path(command[command.index("--project") + 1])
            shutil.rmtree(project / ".venv", ignore_errors=True)
        start = time.perf_counter()
        completed = subprocess.run(command, cwd=ROOT, env=env, check=False)
        elapsed = time.perf_counter() - start
        if completed.returncode not in allowed_exit_codes:
            raise subprocess.CalledProcessError(completed.returncode, command)
        records.append(current | {"command": command[1:], "seconds": elapsed})

    pgo.run = run
    # Four measured rounds give each binary the first position twice.
    for round_index in range(5):
        order = ("fat", "thin") if round_index % 2 == 0 else ("thin", "fat")
        for variant in order:
            current = {"round": round_index, "variant": variant}
            binary, launcher = binaries[variant]
            pgo.run_workloads(binary, launcher, corpus, os.environ.copy())
            hashes = {}
            for project in pgo.CORPUS_PROJECTS:
                for filename in (
                    "requirements.txt",
                    "universal-requirements.txt",
                    "uv.lock",
                    "exported-requirements.txt",
                ):
                    path = corpus / project.name / filename
                    content = "\n".join(
                        line
                        for line in path.read_text().splitlines()
                        if line and not line.startswith("#")
                    )
                    hashes[f"{project.name}/{filename}"] = hashlib.sha256(
                        content.encode()
                    ).hexdigest()
            outputs.append(current | {"hashes": hashes})
        if round_index and outputs[-1]["hashes"] != outputs[-2]["hashes"]:
            raise RuntimeError("Final PGO binaries produced different resolver outputs")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {"provenance": provenance, "measurements": records, "outputs": outputs},
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
