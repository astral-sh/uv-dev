"""Create direct-archive tools and unresolved-name controls for tool commands."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import io
import json
import tarfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "scheduling_fixtures", Path(__file__).with_name("make_scheduling_fixtures.py")
)
assert spec is not None and spec.loader is not None
scheduling = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scheduling)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    name = "uv-bench-filename-tool"
    wheel = scheduling.wheel(args.directory, name, 1, None, console_script=True)
    source = args.directory / "source-project"
    source.mkdir(exist_ok=True)
    files = {
        "pyproject.toml": (
            f'[project]\nname = "{name}"\nversion = "1.0"\n'
            '[build-system]\nrequires = []\nbuild-backend = "backend"\n'
            'backend-path = ["."]\n'
        ).encode(),
        "PKG-INFO": (f"Metadata-Version: 2.3\nName: {name}\nVersion: 1.0\n").encode(),
        "backend.py": (
            "import shutil\nfrom pathlib import Path\n\n"
            "def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):\n"
            f"    wheel = Path(__file__).with_name({wheel['filename']!r})\n"
            "    shutil.copyfile(wheel, Path(wheel_directory) / wheel.name)\n"
            "    return wheel.name\n"
        ).encode(),
        wheel["filename"]: (args.directory / wheel["filename"]).read_bytes(),
    }
    for path, contents in files.items():
        (source / path).write_bytes(contents)
    tar = io.BytesIO()
    with tarfile.open(fileobj=tar, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for path, contents in sorted(files.items()):
            member = tarfile.TarInfo("uv_bench_filename_tool-1.0/" + path)
            member.mode = 0o644
            member.size = len(contents)
            archive.addfile(member, io.BytesIO(contents))
    compressed = io.BytesIO()
    with gzip.GzipFile(fileobj=compressed, mode="wb", filename="", mtime=0) as stream:
        stream.write(tar.getvalue())
    payload = compressed.getvalue()
    archives = []
    for filename in ("uv_bench_filename_tool-1.0.tar.gz", "source.tar.gz"):
        path = args.directory / filename
        path.write_bytes(payload)
        archives.append(
            {
                "filename": filename,
                "url": path.resolve().as_uri(),
                "sha256": hashlib.sha256(payload).hexdigest(),
                "size": len(payload),
                "name": name,
                "version": "1.0",
            }
        )
    profiles = {
        "fast": {"latency_ms": 0, "bytes_per_second": 0},
        "slow": {"latency_ms": 300, "bytes_per_second": 1250000},
        "connection": {
            "latency_ms": 20,
            "bytes_per_second": 1250000,
            "connection_latency_ms": 300,
        },
        "slow-flat": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_latency_ms": {"/flat/extra": 1500},
        },
        "flaky": {
            "latency_ms": 150,
            "bytes_per_second": 1250000,
            "path_failures": {"/flat/extra": {"status": 503, "count": 1}},
        },
    }
    for profile in profiles.values():
        profile.update(pep658=True, cache_control="no-store")
    targets = {
        "local": "{fixtures}/" + wheel["filename"],
        "remote": "{base}/files/" + wheel["filename"],
        "sdist": "{fixtures}/" + archives[0]["filename"],
        "unknown": "{fixtures}/" + archives[1]["filename"],
        "source": "{fixtures}/source-project",
    }
    for selection, target in targets.items():
        setup = [
            [
                "tool",
                "install",
                "--no-preview",
                "--no-index",
                "--find-links",
                "{base}/flat/extra",
                "--python",
                "{python}",
                target,
            ]
        ]
        (args.directory / f"tool-filename-{selection}-setup.json").write_text(
            json.dumps(setup, indent=2) + "\n"
        )
    for filename, value in (
        ("tool-filename-fixtures.json", [wheel, *archives]),
        ("tool-filename-profiles.json", profiles),
        ("tool-filename-targets.json", targets),
    ):
        (args.directory / filename).write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps({"wheel": wheel["filename"], "sha256": wheel["sha256"]}))


if __name__ == "__main__":
    main()
