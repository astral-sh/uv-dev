"""Select a released uv once, or download a binary from a saved release manifest."""

import argparse
import json
import os
import re
import shutil
import tarfile
import tempfile
import urllib.request
from pathlib import Path

from package_fixtures import sha256


def select(version):
    endpoint = "latest" if version == "latest" else "tags/" + version
    request = urllib.request.Request(
        "https://api.github.com/repos/astral-sh/uv/releases/" + endpoint,
        headers={"Accept": "application/vnd.github+json"},
    )
    if os.environ.get("GH_TOKEN"):
        request.add_header("Authorization", "Bearer " + os.environ["GH_TOKEN"])
    with urllib.request.urlopen(request, timeout=60) as response:
        release = json.load(response)
    artifacts = {}
    for target in ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin"):
        name = "uv-" + target + ".tar.gz"
        asset = next(asset for asset in release["assets"] if asset["name"] == name)
        digest = asset.get("digest", "")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            raise ValueError("Missing release archive digest: " + name)
        artifacts[target] = {
            "url": asset["browser_download_url"],
            "sha256": digest.removeprefix("sha256:"),
        }
    return {"version": release["tag_name"], "artifacts": artifacts}


def download(manifest, target, destination):
    artifact = manifest["artifacts"][target]
    with tempfile.TemporaryDirectory() as temporary:
        archive = Path(temporary) / "uv.tar.gz"
        with (
            urllib.request.urlopen(artifact["url"], timeout=60) as source,
            archive.open("wb") as output,
        ):
            shutil.copyfileobj(source, output)
        if sha256(archive) != artifact["sha256"]:
            raise ValueError("Release archive hash mismatch")
        with tarfile.open(archive) as source:
            member = source.getmember("uv-" + target + "/uv")
            if not member.isfile():
                raise ValueError("Expected a regular uv executable")
            with source.extractfile(member) as binary, destination.open("wb") as output:
                shutil.copyfileobj(binary, output)
        destination.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    selection = commands.add_parser("select")
    selection.add_argument("--version", default="latest")
    selection.add_argument("--manifest", type=Path, required=True)
    fetching = commands.add_parser("download")
    fetching.add_argument("--manifest", type=Path, required=True)
    fetching.add_argument("--target", required=True)
    fetching.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "select":
        manifest = select(args.version)
        args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        print("Selected uv " + manifest["version"])
    else:
        download(json.loads(args.manifest.read_text()), args.target, args.output)


if __name__ == "__main__":
    main()
