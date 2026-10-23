"""Create pinned remote requirements graphs for include scheduling benchmarks."""

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
    parser.add_argument("--count", type=int, default=12)
    parser.add_argument("--chain-length", type=int, default=6)
    args = parser.parse_args()
    if args.count < 1 or args.chain_length < 1:
        parser.error("graph sizes must be positive")
    args.directory.mkdir(parents=True, exist_ok=True)
    source = json.loads(Path(__file__).with_name("fixtures.json").read_text())
    manifest = [
        item
        for item in source
        if item["filename"] == "iniconfig-2.1.0-py3-none-any.whl"
    ]
    if len(manifest) != 1:
        raise ValueError("Expected the pinned iniconfig wheel")

    def add(filename: str, content: str) -> None:
        body = content.encode()
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

    children = [f"prefetch-child-{index:02}.txt" for index in range(args.count)]
    add("prefetch-root.txt", "".join(f"-r {child}\n" for child in children))
    for child in children:
        add(child, "iniconfig==2.1.0\n")
    add(
        "prefetch-duplicates.txt",
        f"-r {children[0]}\n-c {children[0]}\n-r {children[0]}\n",
    )
    for index in range(args.chain_length):
        add(
            f"prefetch-chain-{index}.txt",
            f"-r prefetch-chain-{index + 1}.txt\n"
            if index + 1 < args.chain_length
            else "iniconfig==2.1.0\n",
        )
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")
    print(
        json.dumps(
            {
                "fanout": args.count,
                "chain_length": args.chain_length,
                "manifest": str(args.manifest),
            }
        )
    )


if __name__ == "__main__":
    main()
