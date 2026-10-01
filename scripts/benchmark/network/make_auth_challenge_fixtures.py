"""Create a pinned package and synthetic credentials for authentication replay."""

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
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    source = json.loads(Path(__file__).with_name("fixtures.json").read_text())
    manifest = [
        item
        for item in source
        if item["filename"] == "iniconfig-2.1.0-py3-none-any.whl"
    ]
    if len(manifest) != 1:
        raise ValueError("Expected the pinned iniconfig wheel")
    for filename, body, paths in (
        (
            "auth.netrc",
            b"machine 127.0.0.1 login user password password\n",
            [],
        ),
        (
            "auth-requirements.txt",
            b"iniconfig==2.1.0\n",
            ["/requirements/auth-requirements.txt"],
        ),
    ):
        (args.directory / filename).write_bytes(body)
        manifest.append(
            {
                "kind": "raw",
                "filename": filename,
                "url": "data:application/octet-stream;base64,"
                + base64.b64encode(body).decode(),
                "sha256": hashlib.sha256(body).hexdigest(),
                "size": len(body),
                "paths": paths,
            }
        )
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
