"""Create pinned direct-URL lockfiles and deterministic OSV query responses."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", Path(__file__).with_name("make_scheduling_fixtures.py")
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=3001)
    args = parser.parse_args()
    if not 1 <= args.packages <= 10000:
        parser.error("--packages must be between 1 and 10000")
    args.directory.mkdir(parents=True, exist_ok=True)
    names = [f"uv-bench-osv-{index:04d}" for index in range(args.packages)]
    wheels = []
    for name in names:
        wheel = scheduling.wheel(args.directory, name, 1, None)
        wheel["url"] = "https://example.invalid/files/" + wheel["filename"]
        wheels.append(wheel)
    for route in ("plain", "paginated"):
        configuration = {
            "dependencies": {
                name: {
                    "version": "1.0",
                    "pages": 2 if route == "paginated" and index < 2000 else 1,
                }
                for index, name in enumerate(names)
            }
        }
        path = args.directory / f"osv-{route}.json"
        path.write_text(json.dumps(configuration, indent=2) + "\n")
        entry = {
            "filename": path.name,
            "url": "https://example.invalid/" + path.name,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "kind": "osv",
        }
        (args.directory / f"osv-{route}-fixtures.json").write_text(
            json.dumps([entry, *wheels], indent=2) + "\n"
        )
    for count in sorted(
        {1, min(1000, args.packages), min(1001, args.packages), args.packages}
    ):
        selected = list(zip(names[:count], wheels[:count], strict=True))
        requirements = [
            name + " @ {base}/files/" + wheel["filename"] for name, wheel in selected
        ]
        project = (
            '[project]\nname = "uv-bench-osv-project"\nversion = "1.0"\n'
            'requires-python = ">=3.12"\ndependencies = '
            + json.dumps(requirements)
            + "\n"
        )
        lock = ['version = 1\nrevision = 3\nrequires-python = ">=3.12"\n']
        for name, wheel in selected:
            url = "{base}/files/" + wheel["filename"]
            lock.append(
                f'\n[[package]]\nname = "{name}"\nversion = "1.0"\n'
                f'source = {{ url = "{url}" }}\n'
                f'wheels = [{{ url = "{url}", hash = "sha256:{wheel["sha256"]}", '
                f"size = {wheel['size']} }}]\n"
            )
        lock.append(
            '\n[[package]]\nname = "uv-bench-osv-project"\nversion = "1.0"\n'
            'source = { virtual = "." }\ndependencies = [\n'
            + "".join(f'    {{ name = "{name}" }},\n' for name, _ in selected)
            + "]\n"
        )
        (args.directory / f"project-{count}.toml").write_text(project)
        (args.directory / f"lock-{count}.toml").write_text("".join(lock))
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "constrained": {"latency_ms": 250, "bytes_per_second": 125000},
        "uneven": {
            "latency_ms": 100,
            "bytes_per_second": 1250000,
            "osv_query_latency_ms": {f"{names[0]}:0": 600},
        },
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "osv_failures": {f"{names[0]}:0": {"status": 503, "count": 1}},
        },
        "jitter": {
            "latency_ms": 150,
            "jitter_ms": 75,
            "bytes_per_second": 1250000,
            "seed": 42,
        },
    }
    (args.directory / "osv-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    print(json.dumps({"packages": len(names), "directory": str(args.directory)}))


if __name__ == "__main__":
    main()
