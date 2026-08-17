"""Require the cache-denial action before other release job steps."""

from __future__ import annotations

import sys
from argparse import ArgumentParser
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
RELEASE = Path(".github/workflows/release.yml")
BUILD = Path(".github/workflows/build-release-binaries.yml")
ACTION = Path(".github/actions/disable-github-caches/action.yml")
USES = "astral-sh/uv-dev/.github/actions/disable-github-caches@c3892c0a9adbb81c11bbda1eb62e020455665b6a"
BUILD_ENABLED = "${{ !inputs.allow-cache }}"


def check_release_cache_proxy(root: Path) -> list[str]:
    root = root.resolve()
    errors: list[str] = []
    pending = [root / RELEASE]
    visited: set[Path] = set()

    def read(path: Path) -> dict:
        try:
            return yaml.load(path.read_text(), Loader=yaml.BaseLoader) or {}
        except (OSError, yaml.YAMLError) as error:
            errors.append(f"{path.relative_to(root)}: {error}")
            return {}

    action = read(root / ACTION)
    runs = action.get("runs", {})
    if (
        runs
        != {
            "using": "node24",
            "pre": "pre.cjs",
            "main": "main.cjs",
            "post": "post.cjs",
        }
        or action.get("inputs", {}).get("enabled", {}).get("default") != "true"
    ):
        errors.append(f"{ACTION}: cache-denial entry points or default changed")
    for name in ("pre.cjs", "main.cjs", "post.cjs"):
        if not (root / ACTION.parent / name).is_file():
            errors.append(f"{ACTION}: missing {name}")

    while pending:
        path = pending.pop()
        if path in visited:
            continue
        visited.add(path)
        document = read(path)
        for name, job in document.get("jobs", {}).items():
            location = f"{path.relative_to(root)}: jobs.{name}"
            if uses := job.get("uses"):
                if not uses.startswith(("./", "$/")):
                    errors.append(f"{location}: cannot inspect external workflow")
                    continue
                nested = (root / uses[2:]).resolve()
                if not nested.is_relative_to(root):
                    errors.append(f"{location}: workflow escapes the repository")
                    continue
                pending.append(nested)
                continue
            steps = job.get("steps", [])
            first = steps[0] if steps else {}
            if first.get("uses") != USES:
                errors.append(f"{location}: cache-denial action must be the first step")
                continue
            expected = BUILD_ENABLED if path == root / BUILD else "true"
            if (
                "if" in first
                or "continue-on-error" in first
                or first.get("with", {}).get("enabled", "true") != expected
            ):
                errors.append(
                    f"{location}: cache-denial action must be enabled for releases"
                )
    return errors


def main() -> int:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=ROOT)
    errors = check_release_cache_proxy(parser.parse_args().root)
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        return 1
    print("Release cache-denial action is required in every job.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
