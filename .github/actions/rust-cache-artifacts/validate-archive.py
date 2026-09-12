"""Check native cache tar entries before publishing them in another repository."""

import gzip
import json
import posixpath
import re
import subprocess
import sys
import tarfile


def normalize(value):
    if not value or "\\" in value or "\0" in value:
        raise ValueError("invalid archive path")
    return posixpath.normpath(value)


def validate(stream, roots):
    roots = [normalize(root) for root in roots]

    def allowed(value):
        value = normalize(value)
        return any(value == root or value.startswith(root + "/") for root in roots)

    size = 0
    with tarfile.open(fileobj=stream, mode="r|") as archive:
        for count, member in enumerate(archive):
            size += member.size
            if count >= 2_000_000 or size > 100 * 1024**3:
                raise ValueError("oversized cache archive")
            if not allowed(member.name):
                raise ValueError("archive member is outside the cache paths")
            if member.issym():
                target = member.linkname
                if not target.startswith("/") and not re.match(r"^[A-Za-z]:/", target):
                    target = posixpath.join(posixpath.dirname(member.name), target)
                if not allowed(target):
                    raise ValueError("archive symlink is not portable")
            elif member.islnk():
                if not allowed(member.linkname):
                    raise ValueError("archive hardlink is outside the cache paths")
            elif not member.isfile() and not member.isdir():
                raise ValueError("unsupported archive member")


def main():
    filename, compression, roots = sys.argv[1:]
    roots = json.loads(roots)
    if compression == "gzip":
        with gzip.open(filename, "rb") as stream:
            validate(stream, roots)
    elif compression in ("zstd", "zstd-without-long"):
        with subprocess.Popen(
            ["zstd", "--decompress", "--stdout", filename],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        ) as process:
            try:
                validate(process.stdout, roots)
                while process.stdout.read(1024 * 1024):
                    pass
                if process.wait() != 0:
                    raise ValueError("cache decompression failed")
            finally:
                process.stdout.close()
                if process.poll() is None:
                    process.terminate()
    else:
        raise ValueError("unsupported compression")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError) as error:
        sys.exit(str(error))
