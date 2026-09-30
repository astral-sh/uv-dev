"""Create a deterministic Git monorepo for shared-fetch measurements."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path


def make(directory: Path, packages: int) -> dict:
    if packages < 1:
        raise ValueError("At least one package is required")
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
    with tempfile.TemporaryDirectory(prefix="git-fixture-", dir=directory) as temporary:
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

        entries = []
        projects = []
        for number in range(packages):
            name = f"gitbench-{number}"
            subdirectory = f"package-{number}"
            pyproject = (
                f'[project]\nname = "{name}"\nversion = "1.0.0"\n'
                'requires-python = ">=3.8"\ndependencies = []\n\n'
                '[build-system]\nrequires = ["hatchling"]\nbuild-backend = "hatchling.build"\n'
            ).encode()
            package_tree = tree(
                [
                    ("100644", "blob", blob(pyproject), "pyproject.toml"),
                    (
                        "100644",
                        "blob",
                        blob(b'__version__ = "1.0.0"\n'),
                        name.replace("-", "_") + ".py",
                    ),
                ]
            )
            entries.append(("040000", "tree", package_tree, subdirectory))
            projects.append({"name": name, "subdirectory": subdirectory})
        root_tree = tree(entries)
        commit = (
            git(repository, "commit-tree", root_tree, input=b"Network fixture\n")
            .decode()
            .strip()
        )
        git(repository, "update-ref", "refs/heads/main", commit)
        git(repository, "bundle", "create", str(bundle.resolve()), "refs/heads/main")

    git_root = directory / "git"
    git_root.mkdir(exist_ok=True)
    repository = git_root / "monorepo.git"
    if repository.exists():
        actual = git(repository, "rev-parse", "refs/heads/main").decode().strip()
        if actual != commit:
            raise ValueError(f"Existing Git fixture has a different commit: {actual}")
    else:
        subprocess.run(
            ["git", "init", "--bare", "--initial-branch=main", str(repository)],
            env=env,
            capture_output=True,
            check=True,
        )
        git(
            repository,
            "fetch",
            str(bundle.resolve()),
            "refs/heads/main:refs/heads/main",
        )
    git(repository, "fsck", "--full", "--no-reflogs")
    item = {
        "kind": "raw",
        "filename": bundle.name,
        "url": bundle.resolve().as_uri(),
        "sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
        "size": bundle.stat().st_size,
    }
    descriptor = {
        "commit": commit,
        "tree": root_tree,
        "bundle": item,
        "projects": projects,
        "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
    }
    (directory / "git-fixtures.json").write_text(json.dumps([item], indent=2) + "\n")
    (directory / "git-descriptor.json").write_text(
        json.dumps(descriptor, indent=2) + "\n"
    )
    dependencies = ",\n".join(f'    "{project["name"]}"' for project in projects)
    sources = "\n".join(
        f'{project["name"]} = {{ git = "{{base}}/git/monorepo.git", branch = "main", '
        f'subdirectory = "{project["subdirectory"]}" }}'
        for project in projects
    )
    (directory / "git-project.toml").write_text(
        '[project]\nname = "gitbench-root"\nversion = "1.0.0"\n'
        'requires-python = ">=3.12"\ndependencies = [\n'
        + dependencies
        + "\n]\n\n[tool.uv.sources]\n"
        + sources
        + "\n"
    )
    return descriptor


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--packages", type=int, default=8)
    args = parser.parse_args()
    print(json.dumps(make(args.directory, args.packages), indent=2))


if __name__ == "__main__":
    main()
