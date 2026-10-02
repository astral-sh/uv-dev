"""Create deterministic registry tools for real upgrade measurements."""

from __future__ import annotations

import argparse
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
    parser.add_argument("--packages", type=int, default=8)
    parser.add_argument("--shared-dependencies", type=int, default=0)
    parser.add_argument("--index-groups", type=int, default=1)
    parser.add_argument("--payload-bytes", type=int, default=0)
    args = parser.parse_args()
    if not 1 <= args.packages <= 1000:
        parser.error("--packages must be between 1 and 1000")
    if not 0 <= args.shared_dependencies <= 100:
        parser.error("--shared-dependencies must be between 0 and 100")
    if not 1 <= args.index_groups <= 1000 or args.payload_bytes < 0:
        parser.error("index groups must be positive and payload bytes nonnegative")
    args.directory.mkdir(parents=True, exist_ok=True)
    names = [f"uv-bench-upgrade-tool-{number:03d}" for number in range(args.packages)]
    shared = [
        f"uv-bench-upgrade-shared-{number:03d}"
        for number in range(args.shared_dependencies)
    ]
    manifest = []
    for name in [*names, *shared]:
        for version in (1, 2):
            manifest.append(
                scheduling.wheel(
                    args.directory,
                    name,
                    version,
                    [f"{dependency}>=1" for dependency in shared]
                    if name in names
                    else None,
                    console_script=name in names,
                    payload_bytes=args.payload_bytes,
                )
            )
    aliases = {
        f"/upgrade-index-{group}/{name}/": f"/simple/{name}/"
        for group in range(args.index_groups)
        for name in [*names, *shared]
    }
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
            "path_failures": {
                f"/upgrade-index-0/{names[0]}/": {"status": 503, "count": 1}
            },
        },
    }
    for profile in profiles.values():
        profile.update(pep658=True, cache_control="no-store", path_aliases=aliases)
    profiles["cached"] = dict(
        profiles["connection"], cache_control="public, max-age=3600"
    )
    modes = {
        "upgrade": (names, 1, 2, "registry"),
        "current": (names, 2, 2, "registry"),
        "pinned": (names, 1, 1, "pinned"),
        "one": (names[:1], 1, 2, "registry"),
        "mixed": (names, 1, 2, "registry"),
        "local": (names, 1, 1, "local"),
        "empty": ([], 1, 1, "registry"),
    }
    descriptor = {"tools": names, "shared": shared, "scenarios": {}}
    for mode, (tools, initial, final, source) in modes.items():
        setup = []
        projects = set()
        selected = set()
        for number, name in enumerate(tools):
            group = number % args.index_groups
            command = [
                "tool",
                "install",
                "--quiet",
                "--no-cache",
                "--python",
                "3.13" if mode == "mixed" and number % 2 else "{python}",
            ]
            if source == "local":
                command += [
                    "--no-index",
                    "--find-links",
                    "{fixtures}",
                    "{fixtures}/" + name.replace("-", "_") + "-1.0-py3-none-any.whl",
                ]
                for dependency in shared:
                    command += ["--with", dependency + "==1.0"]
            else:
                command += ["--default-index", "{base}/upgrade-index-" + str(group)]
                command += ["--resolution", "lowest" if initial == 1 else "highest"]
                if source == "pinned":
                    for dependency in shared:
                        command += ["--with", dependency + "==1.0"]
                command += [name + "==1.0" if source == "pinned" else name]
                projects.update((group, project) for project in [name, *shared])
            setup.append(command)
            installed_versions = {
                project: f"{initial}.0" for project in [name, *shared]
            }
            setup.append(
                [
                    "run",
                    "--no-project",
                    "--no-sync",
                    "--python",
                    "{work}/state/tools/" + name + "/bin/python",
                    "--quiet",
                    "--",
                    "python",
                    "-B",
                    "-I",
                    "-c",
                    "from importlib.metadata import version; "
                    + f"expected = {installed_versions!r}; "
                    + "assert {name: version(name) for name in expected} == expected",
                ]
            )
            selected.update(
                project.replace("-", "_") + f"-{final}.0-py3-none-any.whl"
                for project in [name, *shared]
            )
        descriptor["scenarios"][mode] = {
            "tools": tools,
            "python_versions": {
                tool: "3.13" if mode == "mixed" and number % 2 else "3.12"
                for number, tool in enumerate(tools)
            },
            "initial_version": f"{initial}.0",
            "final_version": f"{final}.0",
            "source": source,
            "projects": [
                {"index": group, "name": project} for group, project in sorted(projects)
            ],
            "selected_wheels": sorted(selected),
            "command": [
                "tool",
                "upgrade",
                "--all",
                "--quiet",
                "--resolution",
                "highest",
            ]
            + (["--no-index"] if source == "local" else []),
        }
        (args.directory / f"tool-upgrade-{mode}-setup.json").write_text(
            json.dumps(setup, indent=2) + "\n"
        )
    for filename, value in (
        ("tool-upgrade-fixtures.json", manifest),
        ("tool-upgrade-profiles.json", profiles),
        ("tool-upgrade-descriptor.json", descriptor),
    ):
        (args.directory / filename).write_text(json.dumps(value, indent=2) + "\n")
    print(
        json.dumps(
            {
                "tools": len(names),
                "shared": len(shared),
                "index_groups": args.index_groups,
                "wheels": len(manifest),
            }
        )
    )


if __name__ == "__main__":
    main()
