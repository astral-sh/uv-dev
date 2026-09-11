# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Plan and finalize uv releases without invoking a distribution generator.

The public `dist-manifest.json` filename and artifact schema are retained for
consumers such as the versions index, release recovery, and mirror publication.
"""

import argparse
import copy
import hashlib
import json
import re
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
VERSION = r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
GLOBALS = {
    "source.tar.gz": {"kind": "source-tarball", "checksum": "source.tar.gz.sha256"},
    "source.tar.gz.sha256": {"kind": "checksum"},
    "uv-installer.sh": {"kind": "installer"},
    "uv-installer.ps1": {"kind": "installer"},
    "sha256.sum": {"kind": "unified-checksum"},
}


def configuration(root: Path = ROOT) -> dict[str, Any]:
    """Read and validate the complete standalone release target inventory."""
    config = tomllib.loads((root / "release-targets.toml").read_text(encoding="utf-8"))
    targets = config["targets"]
    if not targets or len(targets) != len(set(targets)):
        raise ValueError("Release targets must be nonempty and unique")
    if any(not re.fullmatch(r"[a-z0-9_]+(?:-[a-z0-9_]+)+", t) for t in targets):
        raise ValueError("Invalid release target")
    return config


def changelog(root: Path, version: str) -> str:
    """Read only the changelog section for the version being released."""
    contents = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    section = re.search(
        rf"^## {re.escape(version)}\s*$\n(.*?)(?=^## |\Z)",
        contents,
        re.MULTILINE | re.DOTALL,
    )
    if section is None:
        raise ValueError(f"No changelog section for {version}")
    return section[1].strip()


def release_plan(
    tag: str | None = None, repository: str | None = None, root: Path = ROOT
) -> dict[str, Any]:
    """Describe the release's publication inventory without building or publishing."""
    config = configuration(root)
    version = tomllib.loads(
        (root / "crates/uv/Cargo.toml").read_text(encoding="utf-8")
    )["package"]["version"]
    if not re.fullmatch(VERSION, version):
        raise ValueError(f"Invalid package version: {version!r}")
    implicit = tag is None
    tag = tag or f"v{version}"
    if not re.fullmatch(rf"v?{VERSION}", tag) or tag.removeprefix("v") != version:
        raise ValueError(
            f"Release tag {tag!r} does not match package version {version}"
        )
    if repository is None:
        repository = (
            tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))[
                "workspace"
            ]["package"]["repository"]
            .removeprefix("https://github.com/")
            .removesuffix(".git")
        )
    if not re.fullmatch(r"[A-Za-z0-9-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError(f"Invalid release repository: {repository!r}")
    owner, repo = repository.split("/")
    download_path = f"/{repository}/releases/download/{tag}"
    github_url = f"https://github.com{download_path}"
    download_url = (
        f"https://releases.astral.sh/github/uv/releases/download/{tag}"
        if repository == "astral-sh/uv"
        else github_url
    )
    artifacts: dict[str, Any] = {
        name: {"name": name, **metadata} for name, metadata in GLOBALS.items()
    }
    names = list(artifacts)
    for target in sorted(config["targets"]):
        windows = target.endswith("-pc-windows-msvc")
        extension = "zip" if windows else "tar.gz"
        suffix = ".exe" if windows else ""
        name = f"uv-{target}.{extension}"
        checksum = f"{name}.sha256"
        artifacts[name] = {
            "name": name,
            "kind": "executable-zip",
            "target_triples": [target],
            "assets": [
                {
                    "id": f"uv-{target}-exe-{binary}",
                    "name": binary,
                    "path": f"{binary}{suffix}",
                    "kind": "executable",
                }
                for binary in (("uv", "uvw", "uvx") if windows else ("uv", "uvx"))
            ],
            "checksum": checksum,
        }
        artifacts[checksum] = {
            "name": checksum,
            "kind": "checksum",
            "target_triples": [target],
        }
        names.extend((name, checksum))
    shell_hint = (
        f"curl --proto '=https' --tlsv1.2 -LsSf {download_url}/uv-installer.sh | sh"
    )
    powershell_hint = f'powershell -ExecutionPolicy Bypass -c "irm {download_url}/uv-installer.ps1 | iex"'
    artifacts["uv-installer.sh"].update(
        target_triples=sorted(config["targets"]),
        install_hint=shell_hint,
        description="Install prebuilt binaries via shell script",
    )
    artifacts["uv-installer.ps1"].update(
        target_triples=sorted(
            t for t in config["targets"] if t.endswith("-pc-windows-msvc")
        ),
        install_hint=powershell_hint,
        description="Install prebuilt binaries via powershell script",
    )
    notes = changelog(root, version)
    rows = "\n".join(
        f"| [{name}]({download_url}/{name}) | `{artifact['target_triples'][0]}` | [checksum]({download_url}/{artifact['checksum']}) |"
        for name, artifact in artifacts.items()
        if artifact["kind"] == "executable-zip"
    )
    body = f"""## Release Notes

{notes}

## Install uv {version}

```sh
{shell_hint}
```

```powershell
{powershell_hint}
```

## Download uv {version}

| File | Platform | Checksum |
| --- | --- | --- |
{rows}

## Verifying GitHub Artifact Attestations

```sh
gh attestation verify <file-path> --repo {repository}
```
"""
    return {
        "uv_release_manifest_version": 1,
        "announcement_tag": tag,
        "announcement_tag_is_implicit": implicit,
        "announcement_is_prerelease": "-" in version.partition("+")[0],
        "announcement_title": version,
        "announcement_changelog": notes,
        "announcement_github_body": body,
        "publish_prereleases": False,
        "force_latest": False,
        "releases": [
            {
                "app_name": "uv",
                "app_version": version,
                "display_name": "uv",
                "display": True,
                "artifacts": names,
                "hosting": {
                    "order": ["simple", "github"],
                    "github": {
                        "artifact_base_url": "https://github.com",
                        "artifact_download_path": download_path,
                        "owner": owner,
                        "repo": repo,
                    },
                    "simple": {"download_url": download_url},
                },
            }
        ],
        "artifacts": artifacts,
        "ci": {"github": {"pr_run_mode": "skip"}},
        "linkage": [],
        "upload_files": [],
        "github_attestations": True,
        "github_attestations_filters": ["*.json", "*.sh", "*.ps1", "*.zip", "*.tar.gz"],
        "github_attestations_phase": "announce",
    }


def publication_contract(manifest: dict[str, Any]) -> dict[str, Any]:
    """Project the fields used by uv's build, signing, and publishing consumers."""
    return {
        **{
            key: manifest[key]
            for key in (
                "announcement_tag",
                "announcement_tag_is_implicit",
                "announcement_is_prerelease",
                "announcement_title",
                "announcement_changelog",
                "publish_prereleases",
                "force_latest",
            )
        },
        "releases": [
            {
                key: release[key]
                for key in ("app_name", "app_version", "artifacts", "hosting")
            }
            for release in manifest["releases"]
        ],
        "artifacts": {
            name: {
                key: artifact[key]
                for key in ("kind", "checksum", "assets", "target_triples")
                if key in artifact
                and (key != "target_triples" or artifact["kind"] != "installer")
            }
            for name, artifact in manifest["artifacts"].items()
        },
    }


def compare_plans(native: dict[str, Any], legacy: dict[str, Any]) -> None:
    """Fail before a cutover if an existing publication contract changes."""
    new = publication_contract(native)
    old = publication_contract(legacy)
    differences = [key for key in new if new[key] != old[key]]
    if differences:
        raise ValueError(
            f"Release publication contract differs: {', '.join(differences)}"
        )


def digest(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def finalize(manifest: dict[str, Any], directory: Path) -> dict[str, Any]:
    """Verify the exact asset inventory and checksums before exposing publication jobs."""
    manifest = copy.deepcopy(manifest)
    (release,) = manifest["releases"]
    names = release["artifacts"]
    if (
        release["app_name"] != "uv"
        or len(names) != len(set(names))
        or set(names) != set(manifest["artifacts"])
    ):
        raise ValueError("Unexpected release artifact inventory")
    sums = {}
    for name in names:
        path = directory / name
        if Path(name).name != name or not path.is_file() or path.is_symlink():
            raise ValueError(f"Expected a regular release asset: {name}")
        artifact = manifest["artifacts"][name]
        if checksum_name := artifact.get("checksum"):
            if checksum_name not in names:
                raise ValueError(f"Unplanned checksum for {name}")
            checksum = digest(path)
            sidecar = (directory / checksum_name).read_text(encoding="utf-8")
            if sidecar.replace(" *", "  ").split() != [checksum, name]:
                raise ValueError(f"Archive checksum differs from input: {name}")
            if artifact.get("checksums", {}).get("sha256") != checksum:
                raise ValueError(f"Manifest checksum differs from input: {name}")
            sums[name] = checksum
        if artifact["kind"] == "installer":
            artifact["checksums"] = {"sha256": digest(path)}
    expected = "".join(
        f"{checksum}  {name}\n" for name, checksum in sorted(sums.items())
    )
    if (directory / "sha256.sum").read_text(encoding="utf-8") != expected:
        raise ValueError("Unified checksum file differs from release inventory")
    manifest["upload_files"] = [str(directory / name) for name in names]
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag")
    parser.add_argument("--repository")
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--finalize", type=Path)
    parser.add_argument("--artifacts-dir", type=Path)
    args = parser.parse_args()
    if args.finalize:
        if not args.artifacts_dir or args.tag or args.repository or args.compare:
            parser.error("--finalize requires --artifacts-dir and no planning options")
        manifest = finalize(
            json.loads(args.finalize.read_text(encoding="utf-8")), args.artifacts_dir
        )
    else:
        if args.artifacts_dir:
            parser.error("--artifacts-dir requires --finalize")
        manifest = release_plan(args.tag, args.repository)
        if args.compare:
            compare_plans(
                manifest, json.loads(args.compare.read_text(encoding="utf-8"))
            )
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
