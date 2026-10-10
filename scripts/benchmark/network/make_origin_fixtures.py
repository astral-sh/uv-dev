"""Create a dependency chain whose wheels are served by different artifact hosts."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", HERE / "make_scheduling_fixtures.py"
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--source-manifest", type=Path, default=HERE / "fixtures.json")
    parser.add_argument(
        "--filename",
        default="numpy-2.2.6-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
    )
    args = parser.parse_args()
    target = next(
        item
        for item in json.loads(args.source_manifest.read_text())
        if item["filename"] == args.filename
    )
    if not args.filename.endswith(".whl"):
        parser.error("the target must be a wheel")
    name, version, *_ = args.filename.split("-")
    args.directory.mkdir(parents=True, exist_ok=True)
    gate = scheduling.wheel(
        args.directory,
        "uv-bench-origin-gate",
        1,
        f"{name.replace('_', '-')}=={version}",
    )
    profiles = {}
    for label, latency, rate, main_ranges, gate_ranges, pep658, separate in (
        ("mixed-fast", 0, 0, True, False, False, True),
        ("mixed-slow", 150, 1250000, True, False, False, True),
        ("mixed-constrained", 200, 125000, True, False, False, True),
        ("all-ranges-slow", 150, 1250000, True, True, False, True),
        ("no-ranges-slow", 150, 1250000, False, False, False, True),
        ("same-origin-no-ranges", 150, 1250000, False, False, False, False),
        ("sidecars-slow", 150, 1250000, True, False, True, True),
    ):
        profile = {
            "latency_ms": latency,
            "bytes_per_second": rate,
            "pep658": pep658,
            "ranges": main_ranges,
        }
        if separate:
            profile["artifact_origins"] = {
                "gate": {
                    "filenames": [gate["filename"]],
                    "profile": {"ranges": gate_ranges},
                }
            }
        profiles[label] = profile
    for filename, contents in (
        ("origin-fixtures.json", [gate, target]),
        ("origin-profiles.json", profiles),
    ):
        (args.directory / filename).write_text(json.dumps(contents, indent=2) + "\n")
    print(json.dumps({"gate": gate["filename"], "target": target["filename"]}))


if __name__ == "__main__":
    main()
