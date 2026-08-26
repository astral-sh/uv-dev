#!/usr/bin/env -S uv run --offline --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Provide Bazel credentials for an explicitly selected BuildBuddy endpoint.

Bazel consumes stdout directly. Do not run this helper in a terminal with a real key.
"""

import json
import os
import stat
import sys
from urllib.parse import urlsplit

ALLOWED_HOSTS = {"remote.buildbuddy.io", "openai.buildbuddy.io"}
MAX_KEY_FILE_BYTES = 4096
INVALID_KEY_MESSAGE = "BuildBuddy API keys must contain only visible ASCII characters."


def read_key_file(filename: str) -> str:
    flags = getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    try:
        with open(
            filename,
            "rb",
            opener=lambda path, open_flags: os.open(path, open_flags | flags),
        ) as key_file:
            metadata = os.fstat(key_file.fileno())
            if not stat.S_ISREG(metadata.st_mode):
                raise ValueError
            if os.name == "posix" and (
                metadata.st_uid != os.getuid() or metadata.st_mode & 0o077
            ):
                raise ValueError
            contents = key_file.read(MAX_KEY_FILE_BYTES + 1)
    except (OSError, ValueError):
        raise SystemExit(
            "UV_BAZEL_KEY_FILE must name a readable private regular file owned by the current user."
        ) from None

    if len(contents) > MAX_KEY_FILE_BYTES:
        raise SystemExit("UV_BAZEL_KEY_FILE exceeds the 4096-byte limit.")
    if contents.endswith(b"\r\n"):
        contents = contents[:-2]
    elif contents.endswith(b"\n"):
        contents = contents[:-1]
    try:
        return contents.decode("ascii")
    except UnicodeDecodeError:
        raise SystemExit(INVALID_KEY_MESSAGE) from None


def main() -> None:
    if sys.argv[1:] != ["get"]:
        raise SystemExit("Usage: buildbuddy_credentials.py get")

    try:
        request = json.load(sys.stdin)
        if not isinstance(request, dict) or not isinstance(request.get("uri"), str):
            raise ValueError
        uri = urlsplit(request["uri"])
        port = uri.port
    except ValueError:
        raise SystemExit(
            "Expected a JSON object containing a valid string 'uri'."
        ) from None

    expected_host = os.environ.get("UV_BAZEL_CREDENTIAL_HOST", "remote.buildbuddy.io")
    if expected_host not in ALLOWED_HOSTS:
        raise SystemExit(
            "UV_BAZEL_CREDENTIAL_HOST must be remote.buildbuddy.io or openai.buildbuddy.io."
        )
    if (
        uri.scheme not in {"https", "grpcs"}
        or uri.hostname != expected_host
        or uri.username is not None
        or uri.password is not None
        or port not in {None, 443}
    ):
        raise SystemExit(
            "Credentials require HTTPS or grpcs on the selected BuildBuddy host and port 443."
        )

    key_filename = os.environ.get("UV_BAZEL_KEY_FILE")
    api_key = (
        read_key_file(key_filename)
        if key_filename is not None
        else os.environ.get("BUILDBUDDY_API_KEY")
    )
    if not api_key:
        raise SystemExit(
            "Set UV_BAZEL_KEY_FILE or BUILDBUDDY_API_KEY before enabling BuildBuddy credentials."
        )
    if any(not 33 <= ord(character) <= 126 for character in api_key):
        raise SystemExit(INVALID_KEY_MESSAGE)

    json.dump({"headers": {"x-buildbuddy-api-key": [api_key]}}, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
