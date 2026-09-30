"""Measure local metadata access for a previously verified direct wheel."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=bench.HERE / "fixtures.json")
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--filename", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profiles.read_text())[args.profile]
    fixtures = bench.Fixtures(args.manifest, args.directory, False)
    wheel = fixtures.files[args.filename]
    if wheel.suffix != ".whl":
        parser.error("select a wheel archive")
    expected = fixtures.metadata[args.filename + ".metadata"]
    # Fixture construction verifies the complete archive before the timed cache read.
    started = time.perf_counter()
    metadata = bench.distribution_metadata(wheel)
    seconds = time.perf_counter() - started
    if metadata != expected:
        raise ValueError("Cached wheel metadata differs")
    data = {
        "profile": profile,
        "netem": bench.netem_profile(),
        "manifest_sha256": bench.digest(args.manifest),
        "filename": args.filename,
        "identity_sha256": fixtures.hashes[args.filename],
        "metadata_sha256": hashlib.sha256(metadata).hexdigest(),
        "seconds": seconds,
        "required_bytes": 0,
        "required_waves": 0,
        "required_latency_ms": 0,
        "optimistic_network_floor_seconds": 0,
        "actual_bytes": 0,
        "requests": 0,
        "events": [],
        "scope": "A direct wheel has already been downloaded completely and verified against the required content hash. Its unchanged local metadata requires no network transfer. This reference times ZIP metadata access after fixture verification; its zero network floor excludes process startup, filesystem, and CPU costs.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps(data, indent=2))


if __name__ == "__main__":
    main()
