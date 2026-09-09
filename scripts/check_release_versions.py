"""Check that uv's release packages use the same version within each format."""

# /// script
# requires-python = ">=3.12"
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///

import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_GROUPS = (
    ("project", ("pyproject.toml", "crates/uv-build/pyproject.toml")),
    (
        "package",
        (
            "crates/uv/Cargo.toml",
            "crates/uv-build/Cargo.toml",
            "crates/uv-version/Cargo.toml",
        ),
    ),
)


def check_release_versions(root: Path) -> str:
    matched_versions = []
    # Cargo and Python use different spellings for prerelease versions.
    for section, manifests in VERSION_GROUPS:
        versions = []
        for manifest in manifests:
            with (root / manifest).open("rb") as file:
                versions.append((manifest, tomllib.load(file)[section]["version"]))

        reference, expected = versions[0]
        mismatches = [
            f"  {manifest}: {version}"
            for manifest, version in versions[1:]
            if version != expected
        ]
        if mismatches:
            raise ValueError(
                f"Release versions do not match {reference} ({expected}):\n"
                + "\n".join(mismatches)
            )
        matched_versions.append(expected)
    return matched_versions[0]


def main() -> int:
    try:
        version = check_release_versions(ROOT)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"Release versions match: {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
