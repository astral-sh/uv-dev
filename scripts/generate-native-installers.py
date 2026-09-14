# /// script
# requires-python = ">=3.12"
# dependencies = ["chevron-blue==0.3.0"]
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Generate native uv bootstrap installers from the assembled release archives.

The input is the release plan already consumed by the signing and publishing jobs.
The output retains that manifest's wire format, but its installer checksums come
only from the final, signed GitHub archives.
"""

import argparse
import copy
import hashlib
import json
import re
import subprocess
import tomllib
from pathlib import Path
from typing import Any

import chevron_blue

ROOT = Path(__file__).resolve().parent.parent
GLOBALS = (
    "source.tar.gz",
    "source.tar.gz.sha256",
    "uv-installer.sh",
    "uv-installer.ps1",
    "sha256.sum",
)


def digest(path: Path) -> str:
    """Hash a release asset without loading it into memory."""
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def validate_repository(repository: str) -> None:
    """Require a GitHub owner/repository identifier before rendering scripts."""
    if not re.fullmatch(r"[A-Za-z0-9-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError(f"Invalid release repository: {repository!r}")


def render_installers(
    tag: str, repository: str, checksums: dict[str, str]
) -> dict[str, str]:
    """Render both bootstrap scripts with a fixed version and archive inventory."""
    if not re.fullmatch(r"v?\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", tag):
        raise ValueError(f"Invalid release tag: {tag!r}")
    validate_repository(repository)
    minimums = tomllib.loads(
        (ROOT / "release-targets.toml").read_text(encoding="utf-8")
    )["min-glibc-version"]
    for minimum in minimums.values():
        if not re.fullmatch(r"2\.[0-9]+", minimum):
            raise ValueError(f"Unsupported minimum glibc version: {minimum!r}")
    for target, checksum in checksums.items():
        if not re.fullmatch(r"[a-z0-9_]+(?:-[a-z0-9_]+)+", target):
            raise ValueError(f"Invalid release target: {target!r}")
        if not re.fullmatch(r"[0-9a-f]{64}", checksum):
            raise ValueError(f"Invalid checksum for {target}")
    data = {
        "version": tag,
        "repository": repository,
        "glibc_default": minimums["*"].removeprefix("2."),
        "glibc_aarch64": minimums["aarch64-unknown-linux-gnu"].removeprefix("2."),
        "glibc_riscv64": minimums["riscv64gc-unknown-linux-gnu"].removeprefix("2."),
        "checksums": [
            {"target": target, "checksum": checksum}
            for target, checksum in sorted(checksums.items())
        ],
        "windows_checksums": [
            {"target": target, "checksum": checksum}
            for target, checksum in sorted(checksums.items())
            if target.endswith("-pc-windows-msvc")
        ],
    }
    rendered = {}
    for filename in ("uv-installer.sh", "uv-installer.ps1"):
        template = (ROOT / "scripts" / f"{filename}.in").read_text(encoding="utf-8")
        rendered[filename] = chevron_blue.render(
            template,
            data,
            no_escape=True,
            on_missing_key="error",
            partials_path=None,
        )
    return rendered


def release_archive_checksums(
    plan: dict[str, Any], directory: Path
) -> tuple[dict[str, str], dict[str, str]]:
    """Verify the complete planned archive inventory and its SHA-256 sidecars."""
    releases = [release for release in plan["releases"] if release["app_name"] == "uv"]
    if len(releases) != 1:
        raise ValueError("Expected exactly one uv release")
    targets = {}
    files = {}
    for name in releases[0]["artifacts"]:
        artifact = plan["artifacts"][name]
        if artifact["kind"] != "executable-zip":
            continue
        (target,) = artifact["target_triples"]
        if not re.fullmatch(r"[a-z0-9_]+(?:-[a-z0-9_]+)+", target):
            raise ValueError(f"Invalid release target: {target!r}")
        extension = "zip" if target.endswith("-pc-windows-msvc") else "tar.gz"
        if (
            name != f"uv-{target}.{extension}"
            or artifact["checksum"] != f"{name}.sha256"
        ):
            raise ValueError(f"Unexpected release archive: {name!r}")
        if target in targets:
            raise ValueError(f"Duplicate release target: {target}")
        for path in (directory / name, directory / artifact["checksum"]):
            if not path.is_file() or path.is_symlink():
                raise ValueError(f"Expected a regular release asset: {path.name}")
        checksum = digest(directory / name)
        sidecar = (directory / artifact["checksum"]).read_text(encoding="utf-8")
        if sidecar.replace(" *", "  ").split() != [checksum, name]:
            raise ValueError(f"Archive checksum differs from input: {name}")
        targets[target] = checksum
        files[name] = checksum
    if not targets:
        raise ValueError("Release has no executable archives")
    return targets, files


def generate(
    plan: dict[str, Any], directory: Path, source_root: Path = ROOT
) -> dict[str, Any]:
    """Generate global assets while preserving the release plan's publication contract."""
    manifest = copy.deepcopy(plan)
    (release,) = [item for item in manifest["releases"] if item["app_name"] == "uv"]
    if not set(GLOBALS) <= set(release["artifacts"]):
        raise ValueError("Release is missing a required global artifact")
    targets, files = release_archive_checksums(manifest, directory)
    global_names = {
        name
        for name in release["artifacts"]
        if manifest["artifacts"][name]["kind"] not in {"executable-zip", "checksum"}
    }
    if global_names != {
        "source.tar.gz",
        "uv-installer.sh",
        "uv-installer.ps1",
        "sha256.sum",
    }:
        raise ValueError(f"Unexpected global release assets: {sorted(global_names)}")
    github = release["hosting"]["github"]
    repository = f"{github['owner']}/{github['repo']}"
    scripts = render_installers(manifest["announcement_tag"], repository, targets)
    for name, contents in scripts.items():
        (directory / name).write_text(contents, encoding="utf-8", newline="\n")
    version = release["app_version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", version):
        raise ValueError(f"Invalid release version: {version!r}")
    subprocess.run(
        [
            "git",
            "archive",
            "--format=tar.gz",
            f"--prefix=uv-{version}/",
            "--output",
            str((directory / "source.tar.gz").resolve()),
            "HEAD",
        ],
        cwd=source_root,
        check=True,
    )
    files["source.tar.gz"] = digest(directory / "source.tar.gz")
    (directory / "source.tar.gz.sha256").write_text(
        f"{files['source.tar.gz']}  source.tar.gz\n", encoding="utf-8"
    )
    (directory / "sha256.sum").write_text(
        "".join(f"{checksum}  {name}\n" for name, checksum in sorted(files.items())),
        encoding="utf-8",
    )
    for name, checksum in files.items():
        manifest["artifacts"][name]["checksums"] = {"sha256": checksum}
    manifest["upload_files"] = [str(directory / name) for name in GLOBALS]
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--artifacts-dir", type=Path, required=True)
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text(encoding="utf-8"))
    print(json.dumps(generate(plan, args.artifacts_dir), indent=2))


if __name__ == "__main__":
    main()
