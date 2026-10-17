"""Create a deterministic chain of remote requirements files."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--count", type=int, default=6)
    args = parser.parse_args()
    if args.count < 1:
        parser.error("--count must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    source = json.loads(Path(__file__).with_name("fixtures.json").read_text())
    manifest = [
        item
        for item in source
        if item["filename"] == "iniconfig-2.1.0-py3-none-any.whl"
    ]
    if len(manifest) != 1:
        raise ValueError("Expected the pinned iniconfig wheel")
    for index in range(args.count):
        body = (
            f"-r part{index + 1}.txt\n"
            if index + 1 < args.count
            else "iniconfig==2.1.0\n"
        ).encode()
        filename = f"part{index}.txt"
        (args.directory / filename).write_bytes(body)
        manifest.append(
            {
                "kind": "raw",
                "filename": filename,
                "url": "data:application/octet-stream;base64,"
                + base64.b64encode(body).decode(),
                "sha256": hashlib.sha256(body).hexdigest(),
                "size": len(body),
                "paths": [f"/requirements/{filename}"],
            }
        )
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")
    print(
        json.dumps({"requirements_files": args.count, "manifest": str(args.manifest)})
    )


if __name__ == "__main__":
    main()
