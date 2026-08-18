"""Build the committed Linux musl release binaries in a pinned container."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import tomllib
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
TARGET = "x86_64-unknown-linux-musl"
RECIPE = "scripts/reproducible-builds"


def run(arguments: list[str]) -> None:
    subprocess.run(arguments, check=True)


def git(*arguments: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(REPOSITORY_ROOT), *arguments], text=True
    ).strip()


def sha256(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--revision", default="HEAD", help="Committed revision to build"
    )
    parser.add_argument("--out", type=Path, required=True, help="New output directory")
    parser.add_argument(
        "--check",
        action="store_true",
        help="Build twice and compare binaries and archives",
    )
    args = parser.parse_args()

    revision = git("rev-parse", "--verify", f"{args.revision}^{{commit}}")
    print(f"Building {revision} for {TARGET}", flush=True)
    epoch = git("show", "-s", "--format=%ct", revision)
    version = tomllib.loads(git("show", f"{revision}:pyproject.toml"))["project"][
        "version"
    ]
    environment = {
        "SOURCE_DATE_EPOCH": epoch,
        "UV_COMMIT_HASH": revision,
        "UV_COMMIT_SHORT_HASH": revision[:9],
        "UV_COMMIT_DATE": git("show", "-s", "--format=%cs", revision),
    }
    # Only an exact version tag is part of the recipe. Nearest-tag descriptions
    # otherwise change with clone depth and the set of locally available tags.
    tags = git("tag", "--points-at", revision).splitlines()
    if version in tags:
        environment["UV_LAST_TAG"] = version
        environment["UV_LAST_TAG_DISTANCE"] = "0"

    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    temporary_root = Path.home() / "code" / "tmp"
    temporary_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="uv-reproducible-", dir=temporary_root
    ) as temporary:
        scratch = Path(temporary)
        inputs = scratch / "inputs"
        inputs.mkdir()
        source_archive = inputs / "source.tar"
        git("archive", "--format=tar", f"--output={source_archive}", revision)

        # Take the recipe from the requested revision too, not the working tree.
        recipe = scratch / "recipe"
        recipe.mkdir()
        for name in ("Dockerfile", "build.sh"):
            (recipe / name).write_text(
                git("show", f"{revision}:{RECIPE}/{name}") + "\n"
            )

        environment_file = inputs / "build.env"
        environment_file.write_text(
            "".join(f"{key}={value}\n" for key, value in environment.items())
        )
        image_file = scratch / "image-id"
        run(
            [
                "docker",
                "build",
                "--platform",
                "linux/amd64",
                "--iidfile",
                str(image_file),
                str(recipe),
            ]
        )
        image = image_file.read_text().strip()
        (output / "build-inputs.json").write_text(
            json.dumps(
                {
                    "revision": revision,
                    "source_sha256": sha256(source_archive),
                    "dockerfile_sha256": sha256(recipe / "Dockerfile"),
                    "image": image,
                    "target": TARGET,
                    "environment": environment,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )

        vendor = scratch / "vendor"
        vendor.mkdir()

        def container(
            mode: str,
            destination: Path,
            build_root: str,
            *,
            source_mtime: int = 0,
        ) -> None:
            work = scratch / f"work-{destination.name}"
            work.mkdir()
            arguments = [
                "docker",
                "run",
                "--rm",
                "--platform",
                "linux/amd64",
                "--user",
                f"{os.getuid()}:{os.getgid()}",
                "--workdir",
                build_root,
                "--mount",
                f"type=bind,source={work},target={build_root}",
                "--mount",
                f"type=bind,source={inputs},target=/input,readonly",
                "--mount",
                f"type=bind,source={destination},target=/output",
                "--mount",
                f"type=bind,source={vendor},target=/vendor"
                + (",readonly" if mode == "build" else ""),
            ]
            if mode == "build":
                arguments.extend(
                    [
                        "--network",
                        "none",
                        "--env-file",
                        str(environment_file),
                        "--env",
                        f"REPRO_SOURCE_MTIME={source_mtime}",
                    ]
                )
            run([*arguments, image, mode])

        container("fetch", inputs, "/fetch")
        builds = ("first", "second") if args.check else ("first",)
        for index, name in enumerate(builds):
            destination = output / name
            destination.mkdir()
            container(
                "build",
                destination,
                "/build-one" if index == 0 else "/different/build-two",
                source_mtime=int(epoch) + index * 3600,
            )

        if args.check:
            mismatches = []
            for name in (f"uv-{TARGET}/uv", f"uv-{TARGET}/uvx", f"uv-{TARGET}.tar.gz"):
                first = sha256(output / "first" / name)
                second = sha256(output / "second" / name)
                print(f"{name}: {first} {second}", flush=True)
                if first != second:
                    mismatches.append(name)
            if mismatches:
                raise SystemExit(f"Non-reproducible artifacts: {', '.join(mismatches)}")
            print("Both clean builds are byte-for-byte identical.", flush=True)


if __name__ == "__main__":
    main()
