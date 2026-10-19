#!/usr/bin/env python3
"""Generate smaller embedded Python copies without changing executable code.

The `.py.inc` files are checked in so Cargo builds do not require Python. Edit the
original `.py` files and run this script to regenerate them. Newlines, docstrings,
encoding declarations, and attribution comments are retained.
"""

from __future__ import annotations

import argparse
import ast
import io
import marshal
import re
import tokenize
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = (
    "crates/uv-python-interpreter/python/get_interpreter_info.py",
    "crates/uv-python-interpreter/python/packaging/_elffile.py",
    "crates/uv-python-interpreter/python/packaging/_manylinux.py",
    "crates/uv-python-interpreter/python/packaging/_musllinux.py",
    "crates/uv-installer/src/pip_compileall.py",
    "crates/uv-virtualenv/src/_virtualenv.py",
)
NOTICE = re.compile(r"copyright|licen[cs]e|spdx|based[ -]on|coding[:=]", re.IGNORECASE)


def strip_comments(source: str) -> str:
    lines = source.splitlines(keepends=True)
    comments = [
        token
        for token in tokenize.generate_tokens(io.StringIO(source).readline)
        if token.type == tokenize.COMMENT
    ]
    retained = {token.start[0] for token in comments if token.start[0] <= 2}
    for token in comments:
        if not NOTICE.search(token.string):
            continue
        row = token.start[0]
        retained.add(row)
        # Keep the whole comment block around a notice, including continuation
        # lines that do not themselves contain a copyright or license keyword.
        for direction in (-1, 1):
            adjacent = row + direction
            while 1 <= adjacent <= len(lines):
                line = lines[adjacent - 1].strip()
                if line and not line.startswith("#"):
                    break
                retained.add(adjacent)
                adjacent += direction

    for token in comments:
        row, column = token.start
        if row not in retained:
            lines[row - 1] = (
                lines[row - 1][:column].rstrip(" \t") + lines[row - 1][token.end[1] :]
            )
    return "".join(lines)


def verify_code(source: str, generated: str, filename: str) -> None:
    original_ast = ast.dump(ast.parse(source), include_attributes=True)
    generated_ast = ast.dump(ast.parse(generated), include_attributes=True)
    if original_ast != generated_ast:
        raise ValueError(f"AST or source positions changed: {filename}")
    # Code serialization includes nested code objects, constants, line tables,
    # exception tables, and flags. Use the same filename for both compilations.
    original_code = marshal.dumps(compile(source, filename, "exec"))
    generated_code = marshal.dumps(compile(generated, filename, "exec"))
    if original_code != generated_code:
        raise ValueError(f"Compiled code changed: {filename}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Check without writing.")
    args = parser.parse_args()
    stale = []
    saved = 0
    for filename in SOURCES:
        source_path = ROOT / filename
        output_path = source_path.with_suffix(".py.inc")
        source = source_path.read_text(encoding="utf-8")
        generated = strip_comments(source)
        verify_code(source, generated, filename)
        saved += len(source.encode("utf-8")) - len(generated.encode("utf-8"))
        if args.check:
            if (
                not output_path.exists()
                or output_path.read_text(encoding="utf-8") != generated
            ):
                stale.append(str(output_path.relative_to(ROOT)))
        else:
            output_path.write_text(generated, encoding="utf-8")
    if stale:
        raise SystemExit(
            "Run `python scripts/generate-embedded-python.py` to update:\n"
            + "\n".join(stale)
        )
    print(f"Verified {len(SOURCES)} embedded Python files; {saved:,} bytes omitted.")


if __name__ == "__main__":
    main()
