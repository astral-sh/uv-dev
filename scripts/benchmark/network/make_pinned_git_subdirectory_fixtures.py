"""Create a pinned Git monorepo and replayable commit-lookup responses."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "git_fixtures", Path(__file__).with_name("make_git_fixtures.py")
)
assert spec is not None and spec.loader is not None
git_fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(git_fixtures)

REPOSITORY = "https://github.com/uv-network-benchmark/pinned-subdirectory.git"
API_PREFIX = "/github/uv-network-benchmark/pinned-subdirectory/commits/"


def create(directory: Path, packages: int) -> dict:
    descriptor = git_fixtures.make(directory, packages)
    commit = descriptor["commit"]
    commit_file = directory / "github-commit.txt"
    commit_file.write_text(commit)
    api_paths = [API_PREFIX + ref for ref in (commit, commit[:12], "main")]
    fixture = {
        "kind": "raw",
        "filename": commit_file.name,
        "url": commit_file.resolve().as_uri(),
        "sha256": hashlib.sha256(commit_file.read_bytes()).hexdigest(),
        "size": len(commit),
        "paths": api_paths,
    }
    manifest = [descriptor["bundle"], fixture]
    (directory / "pinned-git-fixtures.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    for selection, projects in (
        ("single", descriptor["projects"][:1]),
        ("many", descriptor["projects"]),
    ):
        dependencies = ",\n".join(f'    "{project["name"]}"' for project in projects)
        for reference, source in (
            ("commit", f'git = "{REPOSITORY}", rev = "{commit}"'),
            ("short", f'git = "{REPOSITORY}", rev = "{commit[:12]}"'),
            ("branch", f'git = "{REPOSITORY}", branch = "main"'),
            ("local", f'git = "{{base}}/git/monorepo.git", rev = "{commit}"'),
        ):
            sources = "\n".join(
                f"{project['name']} = {{ {source}, "
                f'subdirectory = "{project["subdirectory"]}" }}'
                for project in projects
            )
            (directory / f"pinned-git-{selection}-{reference}.toml").write_text(
                '[project]\nname = "gitbench-root"\nversion = "1.0.0"\n'
                'requires-python = ">=3.12"\ndependencies = [\n'
                + dependencies
                + "\n]\n\n[tool.uv.sources]\n"
                + sources
                + "\n"
            )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "api-slow": {
            "latency_ms": 50,
            "bytes_per_second": 1250000,
            "path_latency_ms": dict.fromkeys(api_paths, 1000),
        },
        "api-flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {path: {"status": 503, "count": 2} for path in api_paths},
        },
    }
    (directory / "pinned-git-profiles.json").write_text(
        json.dumps(profiles, indent=2) + "\n"
    )
    return {"commit": commit, "tree": descriptor["tree"], "packages": packages}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=8)
    args = parser.parse_args()
    print(json.dumps(create(args.directory, args.packages), indent=2))


if __name__ == "__main__":
    main()
