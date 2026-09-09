"""GitHub Actions' file-based interfaces."""

import json
import re
from pathlib import Path


def write_json_output(path: Path, name: str, value: object) -> None:
    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name) is None:
        raise ValueError(f"Invalid GitHub Actions output name: {name!r}")
    encoded = json.dumps(value, separators=(",", ":"), allow_nan=False)
    with path.open("a", encoding="utf-8", newline="\n") as output:
        output.write(f"{name}={encoded}\n")
