"""Create locked tools and pinned OSV responses for connection benchmarks."""

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
    parser.add_argument("--packages", type=int, default=12)
    parser.add_argument("--shared-dependencies", type=int, default=0)
    parser.add_argument("--index-groups", type=int, default=2)
    parser.add_argument("--source", choices=("local", "registry"), default="local")
    args = parser.parse_args()
    if not 1 <= args.packages <= 1000:
        parser.error("--packages must be between 1 and 1000")
    if not 0 <= args.shared_dependencies <= 100:
        parser.error("--shared-dependencies must be between 0 and 100")
    if not 1 <= args.index_groups <= 1000:
        parser.error("--index-groups must be between 1 and 1000")
    args.directory.mkdir(parents=True, exist_ok=True)
    names = [f"uv-bench-audit-tool-{number:03d}" for number in range(args.packages)]
    shared = [
        f"uv-bench-audit-shared-{number:03d}"
        for number in range(args.shared_dependencies)
    ]
    requirements = [f"{name}==1.0" for name in shared]
    wheels = [
        scheduling.wheel(args.directory, name, 1, requirements, console_script=True)
        for name in names
    ]
    setup = [
        [
            "tool",
            "install",
            "--preview-features",
            "tool-install-locks",
            "--no-index",
            *(["--find-links", "{fixtures}"] if shared else []),
            "--python",
            "{python}",
            "{fixtures}/" + wheel["filename"],
        ]
        for wheel in wheels
    ]
    aliases = {
        f"/audit-index-{number % args.index_groups}/{name}/": f"/simple/{name}/"
        for number, name in enumerate(names)
    }
    aliases.update(
        {
            f"/audit-index-{group}/{name}/": f"/simple/{name}/"
            for group in range(args.index_groups)
            for name in shared
        }
    )
    if args.source == "registry":
        setup = [
            [
                "tool",
                "install",
                "--preview-features",
                "tool-install-locks",
                "--default-index",
                "{base}/audit-index-" + str(number % args.index_groups),
                "--python",
                "{python}",
                name + "==1.0",
            ]
            for number, name in enumerate(names)
        ]
    wheels.extend(scheduling.wheel(args.directory, name, 1, None) for name in shared)
    dependencies = sorted([*names, *shared])
    for route, pages in (("plain", 1), ("paginated", 2)):
        configuration = {
            "dependencies": {
                name: {"version": "1.0", "pages": pages} for name in dependencies
            },
            "tool_dependencies": {name: [name, *shared] for name in names},
            "tool_registry_projects": {
                name: [
                    {"name": project, "index": number % args.index_groups}
                    for project in [name, *shared]
                ]
                if args.source == "registry"
                else []
                for number, name in enumerate(names)
            },
        }
        path = args.directory / f"tool-audit-{route}.json"
        path.write_text(json.dumps(configuration, indent=2) + "\n")
        entry = {
            "filename": path.name,
            "url": "https://example.invalid/" + path.name,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "kind": "osv",
        }
        (args.directory / f"tool-audit-{route}-fixtures.json").write_text(
            json.dumps([entry, *wheels], indent=2) + "\n"
        )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "connection": {
            "latency_ms": 20,
            "bytes_per_second": 1250000,
            "connection_latency_ms": 300,
        },
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "osv_failures": {f"{dependencies[0]}:0": {"status": 503, "count": 1}},
        },
    }
    if args.source == "registry":
        for profile in profiles.values():
            profile.update(pep658=True, cache_control="no-store", path_aliases=aliases)
        profiles["flaky"]["path_failures"] = {
            next(iter(aliases)): {"status": 503, "count": 1}
        }
        profiles["cached"] = dict(
            profiles["connection"], cache_control="public, max-age=3600"
        )
    for name, value in (
        ("tool-audit-profiles.json", profiles),
        ("tool-audit-setup.json", setup),
        ("tool-audit-one-setup.json", setup[:1]),
        ("tool-audit-empty-setup.json", []),
    ):
        (args.directory / name).write_text(json.dumps(value, indent=2) + "\n")
    print(
        json.dumps(
            {
                "tools": args.packages,
                "source": args.source,
                "shared_dependencies": args.shared_dependencies,
                "index_groups": args.index_groups,
                "directory": str(args.directory),
            }
        )
    )


if __name__ == "__main__":
    main()
