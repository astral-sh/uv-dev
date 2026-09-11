"""Download immutable inputs for the CodSpeed benchmarks, verifying their hashes."""

from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
import urllib.request
from pathlib import Path


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-fixtures"
    )
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)

    fixtures = json.loads(Path(__file__).with_name("fixtures.json").read_text())
    for fixture in fixtures:
        filename = fixture["filename"]
        if Path(filename).name != filename:
            raise ValueError(f"Fixture filename is not a basename: {filename}")
        destination = args.directory / filename
        expected = fixture["sha256"]
        if destination.is_file() and digest(destination) == expected:
            continue

        with tempfile.TemporaryDirectory(dir=args.directory) as temporary:
            temporary_path = Path(temporary) / filename
            with (
                urllib.request.urlopen(fixture["url"], timeout=120) as response,
                temporary_path.open("wb") as output,
            ):
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
            if digest(temporary_path) != expected:
                raise ValueError(f"SHA-256 mismatch for {filename}")
            temporary_path.replace(destination)
        print(filename)


if __name__ == "__main__":
    main()
