"""Create deterministic, dependency-free Git tools for upgrade checks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

REPOSITORY_PREFIX = "https://github.com/uv-network-benchmark/"
BACKEND = """import base64
import csv
import hashlib
import io
import pathlib
import tomllib
import zipfile

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    project = tomllib.loads(pathlib.Path("pyproject.toml").read_text())["project"]
    name = project["name"].replace("-", "_")
    version = project["version"]
    dist_info = f"{name}-{version}.dist-info"
    filename = f"{name}-{version}-py3-none-any.whl"
    files = {
        f"{name}.py": b"def main():\\n    return 0\\n",
        f"{dist_info}/METADATA": f"Metadata-Version: 2.3\\nName: {project['name']}\\nVersion: {version}\\n".encode(),
        f"{dist_info}/WHEEL": b"Wheel-Version: 1.0\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n",
        f"{dist_info}/entry_points.txt": f"[console_scripts]\\n{project['name']} = {name}:main\\n".encode(),
    }
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\\n")
    for path, contents in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(contents).digest()).rstrip(b"=").decode()
        writer.writerow((path, "sha256=" + digest, len(contents)))
    writer.writerow((f"{dist_info}/RECORD", "", ""))
    files[f"{dist_info}/RECORD"] = record.getvalue().encode()
    with zipfile.ZipFile(pathlib.Path(wheel_directory) / filename, "w") as wheel:
        for path, contents in sorted(files.items()):
            info = zipfile.ZipInfo(path, (2000, 1, 1, 0, 0, 0))
            info.external_attr = 0o100644 << 16
            wheel.writestr(info, contents)
    return filename
"""


def make(directory: Path, packages: int) -> dict:
    if packages < 8:
        raise ValueError("At least eight tools are required")
    directory.mkdir(parents=True, exist_ok=True)
    env = {
        key: value for key, value in os.environ.items() if not key.startswith("GIT_")
    }
    env.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_DEFAULT_HASH="sha1",
        GIT_AUTHOR_NAME="uv network benchmark",
        GIT_AUTHOR_EMAIL="uv-benchmark@example.com",
        GIT_COMMITTER_NAME="uv network benchmark",
        GIT_COMMITTER_EMAIL="uv-benchmark@example.com",
        GIT_AUTHOR_DATE="2000-01-01T00:00:00+0000",
        GIT_COMMITTER_DATE="2000-01-01T00:00:00+0000",
    )

    def git(repository: Path, *args: str, input: bytes | None = None) -> bytes:
        return subprocess.check_output(
            ["git", "--git-dir", str(repository), *args], input=input, env=env
        )

    bundle = directory / "monorepo.bundle"
    projects = [
        {
            "name": f"uv-bench-git-tool-{number:03d}",
            "subdirectory": f"tool-{number:03d}",
        }
        for number in range(packages)
    ]
    with tempfile.TemporaryDirectory(prefix="tool-git-", dir=directory) as temporary:
        repository = Path(temporary) / "source.git"
        subprocess.run(
            ["git", "init", "--bare", "--initial-branch=main", str(repository)],
            env=env,
            capture_output=True,
            check=True,
        )

        def blob(data: bytes) -> str:
            return (
                git(repository, "hash-object", "-w", "--stdin", input=data)
                .decode()
                .strip()
            )

        def tree(entries: list[tuple[str, str, str, str]]) -> str:
            contents = "".join(
                f"{mode} {kind} {oid}\t{name}\n"
                for mode, kind, oid, name in sorted(entries, key=lambda item: item[3])
            )
            return git(repository, "mktree", input=contents.encode()).decode().strip()

        backend = blob(BACKEND.encode())
        entries = []
        for project in projects:
            name = project["name"]
            pyproject = (
                f'[project]\nname = "{name}"\nversion = "1.0.0"\n'
                'requires-python = ">=3.12"\ndependencies = []\n'
                f'[project.scripts]\n{name} = "{name.replace("-", "_")}:main"\n'
                '[build-system]\nrequires = []\nbuild-backend = "backend"\n'
                'backend-path = ["."]\n'
            )
            project_tree = tree(
                [
                    ("100644", "blob", backend, "backend.py"),
                    ("100644", "blob", blob(pyproject.encode()), "pyproject.toml"),
                ]
            )
            entries.append(("040000", "tree", project_tree, project["subdirectory"]))
        root_tree = tree(entries)
        commit = (
            git(repository, "commit-tree", root_tree, input=b"Git tool fixtures\n")
            .decode()
            .strip()
        )
        for branch in ("main", "alternate"):
            git(repository, "update-ref", "refs/heads/" + branch, commit)
        git(repository, "bundle", "create", str(bundle.resolve()), "--branches")

    git_root = directory / "git"
    git_root.mkdir(exist_ok=True)
    repositories = ["monorepo", *(f"separate-{number:03d}" for number in range(8))]
    for name in repositories:
        repository = git_root / (name + ".git")
        if repository.exists():
            assert (
                git(repository, "rev-parse", "refs/heads/main").decode().strip()
                == commit
            )
        else:
            subprocess.run(
                ["git", "init", "--bare", "--initial-branch=main", str(repository)],
                env=env,
                capture_output=True,
                check=True,
            )
            git(
                repository, "fetch", str(bundle.resolve()), "+refs/heads/*:refs/heads/*"
            )
        git(repository, "fsck", "--full", "--no-reflogs")

    bundle_item = {
        "kind": "raw",
        "filename": bundle.name,
        "url": bundle.resolve().as_uri(),
        "sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
        "size": bundle.stat().st_size,
    }
    commit_file = directory / "github-commit.txt"
    commit_file.write_text(commit)
    api_paths = [
        f"/github/uv-network-benchmark/{name}/commits/{ref}"
        for name in repositories
        for ref in ("main", "alternate", commit, commit[:12])
    ]
    commit_item = {
        "kind": "raw",
        "filename": commit_file.name,
        "url": commit_file.resolve().as_uri(),
        "sha256": hashlib.sha256(commit_file.read_bytes()).hexdigest(),
        "size": len(commit),
        "paths": api_paths,
    }
    selections = {
        "single": [(projects[0], "monorepo", "main")],
        "eight": [(project, "monorepo", "main") for project in projects[:8]],
        "wide": [(project, "monorepo", "main") for project in projects],
        "separate": [
            (project, f"separate-{number:03d}", "main")
            for number, project in enumerate(projects[:8])
        ],
        "two-refs": [
            (project, "monorepo", "alternate" if number % 2 else "main")
            for number, project in enumerate(projects[:8])
        ],
        "pinned": [(project, "monorepo", commit) for project in projects[:8]],
        "short": [(project, "monorepo", commit[:12]) for project in projects[:8]],
        "empty": [],
    }
    selection_paths = {}
    for selection, entries in selections.items():
        setup = [
            [
                "tool",
                "install",
                "--python",
                "{python}",
                "--no-index",
                f"{project['name']} @ git+{REPOSITORY_PREFIX}{repository}.git@{ref}#subdirectory={project['subdirectory']}",
            ]
            for project, repository, ref in entries
        ]
        selection_paths[selection] = sorted(
            {
                f"/github/uv-network-benchmark/{repository}/commits/{ref}"
                for _, repository, ref in entries
            }
        )
        (directory / f"tool-git-{selection}-setup.json").write_text(
            json.dumps(setup, indent=2) + "\n"
        )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 150, "bytes_per_second": 1250000},
        "api-slow": {
            "latency_ms": 20,
            "bytes_per_second": 1250000,
            "path_latency_ms": dict.fromkeys(api_paths, 500),
        },
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {selection_paths["eight"][0]: {"status": 503, "count": 1}},
        },
    }
    descriptor = {
        "commit": commit,
        "tree": root_tree,
        "bundle": bundle_item,
        "projects": projects,
        "api_paths": selection_paths,
        "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
    }
    for name, value in (
        ("git-descriptor.json", descriptor),
        ("git-fixtures.json", [bundle_item]),
        ("tool-git-fixtures.json", [bundle_item, commit_item]),
        ("tool-git-profiles.json", profiles),
    ):
        (directory / name).write_text(json.dumps(value, indent=2) + "\n")
    return {
        "commit": commit,
        "tree": root_tree,
        "packages": packages,
        "repositories": len(repositories),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=32)
    args = parser.parse_args()
    print(json.dumps(make(args.directory, args.packages), indent=2))


if __name__ == "__main__":
    main()
