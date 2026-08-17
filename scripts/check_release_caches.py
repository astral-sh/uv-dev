"""Reject configured GitHub cache use in the release workflow graph.

Require the cache-disabled environment in every workflow, including reusable
workflows, and retain explicit opt-outs for older or independent cache clients.
This checks declared actions and their inputs, not arbitrary shell commands or
the implementation of third-party actions. It is not a cache-service permission
boundary. Release preparation is separate and is intentionally outside this graph.
"""

from __future__ import annotations

import re
import sys
from argparse import ArgumentParser
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parent.parent
RELEASE_WORKFLOW = Path(".github/workflows/release.yml")

# Review new action types for implicit GitHub cache use before adding them here.
# Version pins are checked separately by zizmor.
REVIEWED_ACTIONS = {
    "actions/attest-build-provenance",
    "actions/checkout",
    "actions/download-artifact",
    "actions/setup-python",
    "actions/upload-artifact",
    "astral-sh/setup-uv",
    "depot/build-push-action",
    "depot/setup-action",
    "docker/login-action",
    "docker/metadata-action",
    "open-security-tools/ost-simple-sts",
    "pyo3/maturin-action",
    "rust-lang/crates-io-auth-action",
    "uraimo/run-on-arch-action",
}

DISABLED_CACHE_INPUTS = {
    "actions/setup-python": ("cache", ""),
    "astral-sh/setup-uv": ("enable-cache", "false"),
    "pyo3/maturin-action": ("sccache", "false"),
}

GITHUB_CACHE_BACKEND = re.compile(r"\btype\s*=\s*gha\b", re.IGNORECASE)


def check_release_caches(root: Path) -> tuple[list[str], int]:
    root = root.resolve()
    pending = [(root / RELEASE_WORKFLOW, True)]
    visited: set[Path] = set()
    errors: list[str] = []

    def check_cache_mode(
        settings: dict[str, Any], location: str, *, required: bool = False
    ) -> None:
        environment = settings.get("env", {})
        if (required or "ACTIONS_CACHE_MODE" in environment) and environment.get(
            "ACTIONS_CACHE_MODE"
        ) != "none":
            errors.append(f"{location}: env.ACTIONS_CACHE_MODE must be 'none'")
        if "cache-mode" in settings and settings["cache-mode"] != "none":
            errors.append(f"{location}: cache-mode must be 'none'")

    def check_local(uses: str, location: str, *, workflow: bool) -> None:
        path = (root / uses[2:]).resolve()
        if not path.is_relative_to(root):
            errors.append(f"{location}: local reference escapes the repository")
            return
        if not workflow:
            path = next(
                (
                    path / name
                    for name in ("action.yml", "action.yaml")
                    if (path / name).is_file()
                ),
                path / "action.yml",
            )
        pending.append((path, workflow))

    def check_step(step: dict[str, Any], location: str) -> None:
        check_cache_mode(step, location)
        if not (uses := step.get("uses")):
            return
        if uses.startswith(("./", "$/")):
            check_local(uses, location, workflow=False)
            return

        action = uses.partition("@")[0].lower()
        if action == "swatinem/rust-cache" or action.split("/")[:2] == [
            "actions",
            "cache",
        ]:
            errors.append(f"{location}: {action} uses GitHub caches")
            return
        if action not in REVIEWED_ACTIONS:
            errors.append(f"{location}: review {action} for GitHub cache use")
            return

        inputs = step.get("with", {})
        if action in DISABLED_CACHE_INPUTS:
            name, value = DISABLED_CACHE_INPUTS[action]
            if inputs.get(name) != value:
                errors.append(f"{location}: {action} must set {name} to {value!r}")

        if action == "depot/build-push-action":
            for name in ("cache-from", "cache-to"):
                value = inputs.get(name, "")
                if "${{" in value or GITHUB_CACHE_BACKEND.search(value):
                    errors.append(
                        f"{location}: {name} may use the GitHub cache backend"
                    )

    while pending:
        path, workflow = pending.pop()
        if path in visited:
            continue
        visited.add(path)
        location = str(path.relative_to(root))
        try:
            # BaseLoader preserves GitHub's `on` key and treats action inputs as strings.
            document = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
        except (OSError, yaml.YAMLError) as error:
            errors.append(f"{location}: {error}")
            continue

        if workflow:
            # A caller's workflow environment is not inherited by a reusable workflow.
            check_cache_mode(document, location, required=True)
            for name, job in document.get("jobs", {}).items():
                job_location = f"{location}: jobs.{name}"
                check_cache_mode(job, job_location)
                if uses := job.get("uses"):
                    if uses.startswith(("./", "$/")):
                        check_local(uses, job_location, workflow=True)
                    else:
                        errors.append(
                            f"{job_location}: cannot inspect external workflow {uses}"
                        )
                for index, step in enumerate(job.get("steps", [])):
                    check_step(step, f"{job_location}.steps[{index}]")
        else:
            runs = document.get("runs", {})
            if runs.get("using") != "composite":
                errors.append(f"{location}: cannot inspect non-composite local action")
                continue
            for index, step in enumerate(runs.get("steps", [])):
                check_step(step, f"{location}: runs.steps[{index}]")

    return errors, len(visited)


def main() -> int:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=ROOT)
    arguments = parser.parse_args()
    errors, count = check_release_caches(arguments.root)
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        return 1
    print(f"Checked {count} release workflow/action files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
