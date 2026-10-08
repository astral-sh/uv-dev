"""Package and restore generated documentation across the publication boundary."""

import argparse
import hashlib
import io
import json
import os
import re
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


def site_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if (
        not path.parts
        or path.is_absolute()
        or path.as_posix() != name
        or any(
            part == ".." or part.lower() == ".git" or "\\" in part
            for part in path.parts
        )
    ):
        raise ValueError(f"Invalid documentation path: {name!r}")
    return path


def add_file(archive: tarfile.TarFile, name: str, data: bytes) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(data)
    member.mode = 0o644
    archive.addfile(member, io.BytesIO(data))


def pack(site: Path, output: Path, source_commit: str, version: str) -> None:
    if re.fullmatch(r"[0-9a-f]{40}", source_commit) is None:
        raise ValueError("Expected the full documentation source commit")
    if site.is_symlink() or not site.is_dir():
        raise ValueError("Expected a generated documentation directory")
    files = {}
    with tarfile.open(output, "x") as archive:
        for path in sorted(site.rglob("*")):
            name = path.relative_to(site).as_posix()
            site_path(name)
            if path.is_symlink():
                raise ValueError(f"Documentation contains a symlink: {name}")
            if path.is_dir():
                continue
            if not path.is_file():
                raise ValueError(f"Documentation contains a non-file: {name}")
            data = path.read_bytes()
            files[name] = hashlib.sha256(data).hexdigest()
            add_file(archive, f"site/{name}", data)
        if not files:
            raise ValueError("Generated documentation is empty")
        manifest = {
            "schema": 1,
            "source_commit": source_commit,
            "version": version,
            "files": files,
        }
        add_file(
            archive, "manifest.json", json.dumps(manifest, sort_keys=True).encode()
        )


def read_file(archive: tarfile.TarFile, name: str) -> bytes:
    stream = archive.extractfile(name)
    if stream is None:
        raise ValueError(f"Missing documentation file: {name}")
    with stream:
        return stream.read()


def unpack(artifact: Path, destination: Path, source_commit: str, version: str) -> None:
    if os.path.lexists(destination):
        raise ValueError("The documentation destination already exists")
    with tarfile.open(artifact, "r") as archive:
        members = archive.getmembers()
        names = {member.name for member in members}
        if len(names) != len(members) or "manifest.json" not in names:
            raise ValueError(
                "Missing manifest or duplicate documentation archive entries"
            )
        if any(
            member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE) for member in members
        ):
            raise ValueError("Documentation archive entries must be regular files")
        if archive.getmember("manifest.json").size > 8 * 1024 * 1024:
            raise ValueError("Documentation manifest is too large")
        manifest = json.loads(read_file(archive, "manifest.json"))
        if (
            set(manifest) != {"schema", "source_commit", "version", "files"}
            or manifest["schema"] != 1
            or manifest["source_commit"] != source_commit
            or manifest["version"] != version
        ):
            raise ValueError(
                "Documentation manifest does not match the selected source and version"
            )
        files = manifest["files"]
        if not isinstance(files, dict):
            raise TypeError("Expected a documentation file inventory")
        if not files or names != {"manifest.json", *(f"site/{name}" for name in files)}:
            raise ValueError("Documentation archive differs from its file inventory")
        for name, digest in files.items():
            site_path(name)
            if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
                raise ValueError(f"Invalid documentation digest for {name}")

        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix=".docs-", dir=destination.parent
        ) as directory:
            staged = Path(directory) / "site"
            staged.mkdir()
            for name, digest in files.items():
                data = read_file(archive, f"site/{name}")
                if hashlib.sha256(data).hexdigest() != digest:
                    raise ValueError(f"Documentation digest mismatch for {name}")
                path = staged / site_path(name)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            staged.rename(destination)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["pack", "unpack"])
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    operation = pack if args.operation == "pack" else unpack
    operation(args.input, args.output, args.source_commit, args.version)


if __name__ == "__main__":
    main()
