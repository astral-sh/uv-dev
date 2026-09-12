"""GitHub Actions' file-based interfaces."""

import json
import re
from pathlib import Path


def write_output(path: Path, name: str, value: str) -> None:
    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name) is None:
        raise ValueError(f"Invalid GitHub Actions output name: {name!r}")
    if "\n" in value or "\r" in value:
        raise ValueError("GitHub Actions output values must be single-line")
    with path.open("a", encoding="utf-8", newline="\n") as output:
        output.write(f"{name}={value}\n")


def write_json_output(path: Path, name: str, value: object) -> None:
    write_output(path, name, json.dumps(value, separators=(",", ":"), allow_nan=False))


def append_summary(path: Path, content: str) -> None:
    with path.open("a", encoding="utf-8", newline="\n") as summary:
        summary.write(content)
        if not content.endswith("\n"):
            summary.write("\n")
