"""Write a CSV of unfinished targets without modifying prior corpus runs.

Use a network-disabled container. Mount the harness CSV and previous run
directories read-only. Redirect stdout to a fresh file for run.py --input.
"""

import argparse
import csv
import hashlib
import io
import json
import math
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument(
        "runs", type=Path, nargs="+", help="original run, then any sequential resumes"
    )
    args = parser.parse_args()
    if not Path("/.dockerenv").exists():
        parser.error("inspect package output only in a container")
    with args.manifest.open() as file:
        rows = list(csv.DictReader(file))
    last_hash = hashlib.sha256(args.manifest.read_bytes()).hexdigest()
    # resolve_all excludes these names after selecting the input manifest.
    excluded = {
        "nucliadb",
        "tf-models-nightly",
        "mtmtrain",
        "llm-dialog-manager",
        "python-must",
        "edx-enterprise",
        "kcli",
        "emmet-api",
    }
    rows = [row for row in rows if row["project"] not in excluded]
    reference = None
    for run in args.runs:
        receipt = json.loads((run / "run.json").read_text())
        if (
            receipt["project"] != "corpus"
            or receipt["limit"] is not None
            or receipt["input_sha256"] != last_hash
        ):
            parser.error(
                f"{run} was not run against the expected full manifest or resume shard"
            )
        controls = {
            key: receipt.get(key)
            for key in (
                "image",
                "binary_sha256",
                "exclude_newer",
                "harness",
                "preview",
                "settings",
                "workers",
                "offline",
            )
        }
        if reference is None:
            reference = controls
        elif controls != reference:
            parser.error(f"mixed build, interpreter or resolver settings in {run}")

        remaining = []
        for row in rows:
            name = row["project"]
            if Path(name).name != name or name in ("", ".", ".."):
                parser.error("invalid project name in manifest")
            directory = run / "results" / "corpus" / name
            summary = directory / "summary.json"
            if directory.is_symlink() or not summary.is_file() or summary.is_symlink():
                remaining.append(row)
                continue
            try:
                data = json.loads(summary.read_text())
            except (OSError, ValueError):
                remaining.append(row)
                continue
            valid = data.get("package") == name and isinstance(
                data.get("exit_code"), int
            )
            duration = data.get("time")
            valid = (
                valid
                and isinstance(duration, (int, float))
                and math.isfinite(duration)
                and duration > 0
            )
            files = ["stdout.txt", "stderr.txt"]
            if data.get("exit_code") == 0:
                files.append("uv.lock")
            if not valid or not all(
                (directory / file).is_file() and not (directory / file).is_symlink()
                for file in files
            ):
                remaining.append(row)
        rows = remaining
        # An interrupted resume can itself be resumed. Reproduce the exact CSV
        # emitted for its input to validate the next run in the chain.
        last_hash = hashlib.sha256(encode(rows)).hexdigest()
    sys.stdout.buffer.write(encode(rows))


def encode(rows):
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(
        buffer, fieldnames=["project"], extrasaction="ignore", lineterminator="\n"
    )
    writer.writeheader()
    writer.writerows(rows)
    return buffer.getvalue().encode()


if __name__ == "__main__":
    main()
