"""Prepare and capture a bounded, ordinary-I/O local wheelhouse census."""

from __future__ import annotations

import argparse
import ast
import base64
import csv
import email.parser
import email.policy
import hashlib
import io
import json
import os
import platform
import posixpath
import re
import stat
import statistics
import subprocess
import sys
import time
import zipfile
from collections import Counter, defaultdict
from pathlib import Path, PurePosixPath

BASE_COMMIT = "cdc71a4fd8cd5aa76304c5af6e13473488283f62"
BASE_TREE = "a6dca5e1aba65d19d0c0154321484db4b28a7449"
SAVED_COMMIT = "7f42434960c790fb70736558fddb4613c9fbb833"
SAVED_TREE = "a8fcfbe60aa504bb62c9ba4a7a7e56bc8d9b397c"
SAVED_UV_SHA256 = "1e5dee79bdb92a4b4cb60bb99119455ab5ced5585b82a0ddcdb2288a7e2023df"
COUNTS = (1, 1_000, 10_000)
EXPECTED_STDOUT = b"uv-census-target==1.0.0\n"
TRACE_CALLS = (
    "openat",
    "read",
    "close",
    "getdents64",
    "statx",
    "newfstatat",
    "readlink",
    "readlinkat",
)
PATHNAME_TRACE_CALLS = (*TRACE_CALLS, "execve")
HEX256 = re.compile(r"[0-9a-f]{64}\Z")
SAFE_ID = re.compile(r"[a-zA-Z0-9_-]+\Z")
TRACE_PREFIX = re.compile(r"^(?:(?:\[pid\s+(\d+)\]|(\d+))\s+)?(\d+\.\d+)\s+(.*)$")
TRACE_BODY = re.compile(r"^(\w+)\((.*)\)\s+=\s+(.+?)\s+<([0-9.]+)>$")
TRACE_RESUMED = re.compile(r"^<\.\.\. (\w+) resumed>(.*)$")
TRACE_EXITED = re.compile(r"^\+\+\+ exited with (\d+) \+\+\+$")
TRACE_KILLED = re.compile(r"^\+\+\+ killed by (SIG\w+)(?: \(core dumped\))? \+\+\+$")
TRACE_PATH = re.compile(r'"(?:\\.|[^"\\])*"|<[^>]*>')


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def checked_file(path: Path, expected: str) -> Path:
    require(bool(HEX256.fullmatch(expected)), f"invalid SHA-256: {expected!r}")
    require(path.is_absolute(), f"path must be absolute: {path}")
    resolved = path.resolve(strict=True)
    require(resolved.is_file(), f"not a regular file: {resolved}")
    require(sha256_file(resolved) == expected, f"SHA-256 mismatch: {path}")
    return resolved


def canonical_bytes(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def write_new(path: Path, data: bytes) -> None:
    with path.open("xb") as stream:
        stream.write(data)


def write_json(path: Path, value: object) -> str:
    data = json.dumps(value, indent=2, sort_keys=True).encode() + b"\n"
    write_new(path, data)
    return sha256_bytes(data)


def new_directory(path: Path) -> Path:
    require(path.is_absolute(), f"new directory must be absolute: {path}")
    parent = path.parent.resolve(strict=True)
    require(path.name not in ("", ".", ".."), f"invalid directory: {path}")
    path = parent / path.name
    path.mkdir()
    return path


def relative_file(stage: Path, relative: str) -> Path:
    parts = PurePosixPath(relative).parts
    require(
        bool(parts)
        and not PurePosixPath(relative).is_absolute()
        and all(part not in ("", ".", "..") for part in parts)
        and PurePosixPath(relative).as_posix() == relative
        and "\\" not in relative
        and "\0" not in relative,
        f"invalid staged relative path: {relative!r}",
    )
    path = stage.joinpath(*parts)
    require(
        path.resolve(strict=True).is_relative_to(stage), f"path escapes stage: {path}"
    )
    return path


def single_header(headers: email.message.Message, name: str) -> str:
    values = headers.get_all(name, [])
    require(len(values) == 1, f"expected one {name} header")
    return str(values[0])


def validate_generated_wheel(path: Path, expected: dict) -> dict:
    """Independently validate the exact four-member plain Packse wheel format."""
    require(stat.S_ISREG(path.lstat().st_mode), f"non-regular wheel: {path}")
    require(path.name == expected["filename"], f"filename mismatch: {path}")
    require(path.stat().st_size == expected["size"], f"size mismatch: {path}")
    require(sha256_file(path) == expected["sha256"], f"wheel hash mismatch: {path}")
    normalized = expected["name"].replace("-", "_")
    version = expected["version"]
    require(version == "1.0.0", f"unexpected generated version: {version}")
    require(expected["tags"] == ["py3-none-any"], f"unexpected generated tags: {path}")
    dist_info = f"{normalized}-{version}.dist-info"
    names = (
        f"{normalized}/__init__.py",
        f"{dist_info}/METADATA",
        f"{dist_info}/WHEEL",
        f"{dist_info}/RECORD",
    )
    with zipfile.ZipFile(path) as archive:
        members = archive.infolist()
        require(
            tuple(member.filename for member in members) == names,
            f"member mismatch: {path}",
        )
        contents = {}
        for member in members:
            kind = stat.S_IFMT(member.external_attr >> 16)
            require(
                not member.is_dir()
                and kind in (0, stat.S_IFREG)
                and member.compress_type == zipfile.ZIP_STORED
                and member.file_size <= 64 * 1024,
                f"unsupported generated member: {member.filename}",
            )
            data = archive.read(member)  # zipfile verifies the stored CRC.
            require(
                len(data) == member.file_size,
                f"member size mismatch: {member.filename}",
            )
            contents[member.filename] = data
    require(
        contents[names[0]] == f'__version__ = "{version}"\n'.encode(),
        f"module bytes differ: {path}",
    )
    parser = email.parser.BytesParser(policy=email.policy.compat32)
    metadata = parser.parsebytes(contents[names[1]])
    require(
        single_header(metadata, "Metadata-Version") == "2.3",
        "unexpected metadata version",
    )
    require(
        single_header(metadata, "Name") == expected["name"],
        f"metadata name differs: {path}",
    )
    require(
        single_header(metadata, "Version") == version,
        f"metadata version differs: {path}",
    )
    require(
        not metadata.get_all("Requires-Dist"),
        f"generated wheel has dependencies: {path}",
    )
    require(
        not metadata.get_all("Provides-Extra"), f"generated wheel has extras: {path}"
    )
    require(
        not metadata.get_all("Requires-Python"),
        f"generated wheel restricts Python: {path}",
    )
    wheel = parser.parsebytes(contents[names[2]])
    require(single_header(wheel, "Wheel-Version") == "1.0", "unexpected wheel version")
    require(
        single_header(wheel, "Generator") == "uv-test", "unexpected wheel generator"
    )
    require(single_header(wheel, "Root-Is-Purelib") == "true", "wheel is not purelib")
    tags = [str(tag) for tag in wheel.get_all("Tag", [])]
    require(tags == ["py3-none-any"], f"metadata tags differ: {path}")
    record_rows = []
    for name in names[:-1]:
        data = contents[name]
        digest = (
            base64.urlsafe_b64encode(hashlib.sha256(data).digest())
            .rstrip(b"=")
            .decode()
        )
        record_rows.append([name, f"sha256={digest}", str(len(data))])
    record_rows.append([names[-1], "", ""])
    record = contents[names[-1]].decode("utf-8")
    require(
        list(csv.reader(io.StringIO(record, newline=""), strict=True)) == record_rows
        and record == "".join(",".join(row) + "\n" for row in record_rows),
        f"RECORD mismatch: {path}",
    )
    return {
        "filename": path.name,
        "name": single_header(metadata, "Name"),
        "version": single_header(metadata, "Version"),
        "tags": tags,
        "sha256": expected["sha256"],
        "size": expected["size"],
        "metadata_sha256": sha256_bytes(contents[names[1]]),
        "record_sha256": sha256_bytes(contents[names[-1]]),
    }


def verify_identities(generator: Path, wheels: list[dict]) -> list[dict]:
    requests = [
        {key: wheel[key] for key in ("filename", "name", "version", "tags")}
        for wheel in wheels
    ]
    result = subprocess.run(
        [str(generator), "verify-identities"],
        input=canonical_bytes(requests),
        capture_output=True,
        check=False,
    )
    require(
        result.returncode == 0,
        f"uv identity validation failed: {result.stderr.decode(errors='replace')}",
    )
    response = json.loads(result.stdout)
    require(
        response["schema"] == 1
        and response["base_commit"] == BASE_COMMIT
        and response["base_tree"] == BASE_TREE,
        "identity helper has the wrong source identity",
    )
    identities = response["identities"]
    require(len(identities) == len(wheels), "identity result count mismatch")
    for wheel, identity in zip(wheels, identities, strict=True):
        require(
            wheel["filename"] == identity["filename"], "identity result order changed"
        )
        wheel.update(identity)
    return wheels


def audit_generated_catalog(
    directory: Path, expected: list[dict], generator: Path
) -> list[dict]:
    entries = sorted(directory.iterdir(), key=lambda path: path.name)
    expected_by_name = {wheel["filename"]: wheel for wheel in expected}
    require(len(expected_by_name) == len(expected), "duplicate expected wheel filename")
    require(
        [path.name for path in entries] == sorted(expected_by_name),
        f"catalog filenames differ: {directory}",
    )
    wheels = [
        validate_generated_wheel(path, expected_by_name[path.name]) for path in entries
    ]
    wheels = verify_identities(generator, wheels)
    for wheel in wheels:
        expected_wheel = expected_by_name[wheel["filename"]]
        require(
            all(
                wheel[key] == expected_wheel[key] for key in ("name", "version", "tags")
            ),
            f"embedded identity differs: {wheel['filename']}",
        )
    return wheels


def prepare(args: argparse.Namespace) -> None:
    generator = checked_file(args.generator, args.generator_sha256)
    stage = new_directory(args.stage)
    fixtures = stage / "fixtures"
    subprocess.run([str(generator), "generate", str(fixtures)], check=True)
    generation_path = fixtures / "generation.json"
    generation = json.loads(generation_path.read_bytes())
    require(
        generation["schema"] == 1
        and generation["kind"] == "synthetic-packse"
        and generation["base_commit"] == BASE_COMMIT
        and generation["base_tree"] == BASE_TREE
        and generation["generator"] == "uv_test::packse::generate_wheel"
        and generation["counts"] == list(COUNTS)
        and generation["requirements"] == "requirements.in"
        and len(generation["wheels"]) == COUNTS[-1],
        "generator manifest has the wrong identity or shape",
    )
    require(
        [wheel["name"] for wheel in generation["wheels"]]
        == ["uv-census-target"]
        + [f"uv-census-filler-{index:05}" for index in range(1, COUNTS[-1])],
        "generated package identities differ",
    )
    requirements = fixtures / "requirements.in"
    require(
        requirements.read_bytes() == EXPECTED_STDOUT, "unexpected generated requirement"
    )
    catalogs = []
    for count in COUNTS:
        relative = f"fixtures/wheelhouse-{count}"
        wheels = audit_generated_catalog(
            stage / relative, generation["wheels"][:count], generator
        )
        catalogs.append(
            {
                "id": f"synthetic-{count}",
                "kind": "synthetic-packse",
                "path": relative,
                "count": count,
                "entries_sha256": sha256_bytes(canonical_bytes(wheels)),
                "entries": wheels,
            }
        )
    manifest = {
        "schema": 1,
        "kind": "synthetic-packse",
        "base_commit": BASE_COMMIT,
        "base_tree": BASE_TREE,
        "generator": {"path": str(generator), "sha256": args.generator_sha256},
        "generation_sha256": sha256_file(generation_path),
        "requirements": "fixtures/requirements.in",
        "requirements_sha256": sha256_file(requirements),
        "expected_stdout_sha256": sha256_bytes(EXPECTED_STDOUT),
        "catalogs": catalogs,
    }
    manifest_path = stage / "catalogs.json"
    digest = write_json(manifest_path, manifest)
    print(
        json.dumps({"manifest": str(manifest_path), "sha256": digest}, sort_keys=True)
    )


def verify_manifest(path: Path, expected_sha256: str) -> tuple[Path, dict]:
    manifest_path = checked_file(path, expected_sha256)
    stage = manifest_path.parent
    manifest = json.loads(manifest_path.read_bytes())
    require(
        manifest["schema"] == 1
        and manifest["kind"] == "synthetic-packse"
        and manifest["base_commit"] == BASE_COMMIT
        and manifest["base_tree"] == BASE_TREE,
        "unsupported catalog manifest",
    )
    require(
        sha256_file(relative_file(stage, "fixtures/generation.json"))
        == manifest["generation_sha256"],
        "generation manifest changed",
    )
    requirements = relative_file(stage, manifest["requirements"])
    require(
        sha256_file(requirements) == manifest["requirements_sha256"]
        and requirements.read_bytes() == EXPECTED_STDOUT
        and manifest["expected_stdout_sha256"] == sha256_bytes(EXPECTED_STDOUT),
        "requirements or expected result changed",
    )
    require(
        [catalog["count"] for catalog in manifest["catalogs"]] == list(COUNTS),
        "wrong catalog counts",
    )
    for catalog in manifest["catalogs"]:
        require(
            catalog["kind"] == "synthetic-packse"
            and catalog["id"] == f"synthetic-{catalog['count']}"
            and len(catalog["entries"]) == catalog["count"]
            and sha256_bytes(canonical_bytes(catalog["entries"]))
            == catalog["entries_sha256"],
            "invalid catalog identity",
        )
        directory = relative_file(stage, catalog["path"])
        require(
            stat.S_ISDIR(directory.lstat().st_mode),
            f"non-directory catalog: {directory}",
        )
        entries = catalog["entries"]
        require(
            [path.name for path in sorted(directory.iterdir())]
            == [entry["filename"] for entry in entries],
            f"catalog filenames changed: {directory}",
        )
        for entry in entries:
            wheel = directory / entry["filename"]
            require(stat.S_ISREG(wheel.lstat().st_mode), f"non-regular wheel: {wheel}")
            require(
                wheel.stat().st_size == entry["size"], f"wheel size changed: {wheel}"
            )
            require(
                sha256_file(wheel) == entry["sha256"], f"wheel bytes changed: {wheel}"
            )
    return stage, manifest


def trace_records(
    text: str,
) -> tuple[list[tuple[str, str, str, str, float]], list[str]]:
    pending = {}
    records = []
    unparsed = []
    for line in text.splitlines():
        match = TRACE_PREFIX.match(line)
        if match is None:
            if line.strip():
                unparsed.append(line)
            continue
        process = match[1] or match[2] or "main"
        body = match[4]
        if body.startswith("--- "):
            continue
        exited = TRACE_EXITED.fullmatch(body)
        if exited is not None:
            records.append((process, "process_exit", body, exited[1], 0.0))
            continue
        killed = TRACE_KILLED.fullmatch(body)
        if killed is not None:
            records.append((process, "process_killed", body, killed[1], 0.0))
            continue
        if body.endswith("<unfinished ...>"):
            if process in pending:
                unparsed.append(pending[process])
            pending[process] = body.removesuffix("<unfinished ...>")
            continue
        resumed = TRACE_RESUMED.match(body)
        if resumed is not None:
            prefix = pending.pop(process, None)
            if prefix is None or not prefix.startswith(resumed[1] + "("):
                unparsed.append(line)
                continue
            body = prefix + resumed[2]
        parsed = TRACE_BODY.match(body)
        if parsed is None or parsed[1] not in PATHNAME_TRACE_CALLS:
            unparsed.append(line)
            continue
        records.append((process, parsed[1], parsed[2], parsed[3], float(parsed[4])))
    unparsed.extend(pending.values())
    return records, unparsed


def first_arguments(arguments: str, count: int) -> list[str]:
    """Split leading syscall arguments, respecting strings, structs, and FD annotations."""
    fields = []
    start = 0
    quoted = False
    escaped = False
    angle_depth = 0
    nested_depth = 0
    for index, character in enumerate(arguments):
        if quoted:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                quoted = False
        elif character == '"':
            quoted = True
        elif character == "<":
            angle_depth += 1
        elif character == ">" and angle_depth:
            angle_depth -= 1
        elif character in "([{" and angle_depth == 0:
            nested_depth += 1
        elif character in ")]}" and nested_depth and angle_depth == 0:
            nested_depth -= 1
        elif character == "," and angle_depth == 0 and nested_depth == 0:
            fields.append(arguments[start:index].strip())
            if len(fields) == count:
                return fields
            start = index + 1
    fields.append(arguments[start:].strip())
    return fields[:count]


def path_tokens(arguments: str) -> set[str]:
    paths = set()
    for match in TRACE_PATH.finditer(arguments):
        token = match[0]
        if token.startswith('"'):
            try:
                value = ast.literal_eval(token)
            except (SyntaxError, ValueError):
                continue
        else:
            value = token[1:-1].removesuffix(" (deleted)")
        if isinstance(value, str) and value.startswith("/"):
            paths.add(value)
    return paths


def operation_paths(syscall: str, arguments: str, cwd: Path | None) -> set[str]:
    """Read pathname arguments, never a read/readlink result buffer."""
    if syscall in ("read", "close", "getdents64"):
        fields = first_arguments(arguments, 1)
        return path_tokens(fields[0])
    if syscall == "readlink":
        fields = first_arguments(arguments, 1)
        directory = str(cwd) if cwd else None
        pathname = fields[0]
    else:
        fields = first_arguments(arguments, 2)
        if len(fields) != 2:
            return set()
        directory = next(iter(path_tokens(fields[0])), None)
        if directory is None and fields[0] == "AT_FDCWD" and cwd:
            directory = str(cwd)
        pathname = fields[1]
    paths = set()
    if pathname.startswith('"'):
        try:
            value = ast.literal_eval(pathname)
        except (SyntaxError, ValueError):
            return paths
        if isinstance(value, str):
            if value.startswith("/"):
                paths.add(posixpath.normpath(value))
            elif directory:
                paths.add(posixpath.normpath(posixpath.join(directory, value)))
    return paths


def attributable(path: str, root: str) -> bool:
    return path == root or path.startswith(root + "/")


def metadata_arguments(
    syscall: str, arguments: str
) -> tuple[str, str, str, str] | None:
    """Return the dirfd, pathname, flags, and returned metadata of a stat call."""
    if syscall not in ("statx", "newfstatat"):
        return None
    fields = first_arguments(arguments, 6)
    if syscall == "statx" and len(fields) == 5:
        return fields[0], fields[1], fields[2], fields[4]
    if syscall == "newfstatat" and len(fields) == 4:
        return fields[0], fields[1], fields[3], fields[2]
    return None


def flag_set(flags: str) -> set[str]:
    return {flag.strip() for flag in flags.split("|")}


def metadata_value(attributes: str, name: str) -> str | None:
    match = re.search(r"(?:\{|,)\s*" + re.escape(name) + r"=([^,}]+)", attributes)
    return match[1].strip() if match else None


def descriptor_number(value: str) -> int | None:
    match = re.fullmatch(r"(\d+)(?:<[^<>]*>)?", value)
    return int(match[1]) if match else None


def attribute_trace(text: str, wheelhouse: Path, cwd: Path | None = None) -> dict:
    root = str(wheelhouse)
    records, unparsed = trace_records(text)
    total = Counter()
    errors = Counter()
    attributed = Counter()
    attributed_errors = Counter()
    seconds = defaultdict(float)
    wheel_opens = Counter()
    wheel_reads = defaultdict(lambda: {"calls": 0, "bytes": 0})
    nofollow = 0
    for _, syscall, arguments, result, duration in records:
        if syscall not in TRACE_CALLS:
            continue
        total[syscall] += 1
        failed = result.startswith("-1 ")
        if failed:
            errors[syscall] += 1
        paths = {
            path
            for path in operation_paths(syscall, arguments, cwd)
            if attributable(path, root)
        }
        if not paths:
            continue
        attributed[syscall] += 1
        seconds[syscall] += duration
        if failed:
            attributed_errors[syscall] += 1
        metadata = metadata_arguments(syscall, arguments)
        if metadata is not None and "AT_SYMLINK_NOFOLLOW" in flag_set(metadata[2]):
            nofollow += 1
        wheels = {
            str(PurePosixPath(path).relative_to(root))
            for path in paths
            if path.endswith(".whl")
        }
        if syscall == "openat":
            for wheel in wheels:
                wheel_opens[wheel] += 1
        elif syscall == "read":
            returned = re.match(r"^\d+", result)
            for wheel in wheels:
                wheel_reads[wheel]["calls"] += 1
                if returned:
                    wheel_reads[wheel]["bytes"] += int(returned[0])
    return {
        "trace_filter": list(TRACE_CALLS),
        "process_calls": dict(sorted(total.items())),
        "process_errors": dict(sorted(errors.items())),
        "wheelhouse_calls": dict(sorted(attributed.items())),
        "wheelhouse_errors": dict(sorted(attributed_errors.items())),
        "wheelhouse_syscall_seconds": dict(sorted(seconds.items())),
        "wheelhouse_nofollow_metadata_calls": nofollow,
        "wheel_opens": dict(sorted(wheel_opens.items())),
        "wheel_reads": dict(sorted(wheel_reads.items())),
        "unparsed_lines": unparsed,
        "complete": not unparsed,
    }


def catalog_trace_coverage(
    text: str, wheelhouse: Path, entries: list[dict], cwd: Path | None = None
) -> dict:
    """Check the frozen synthetic catalog independently of trace-parser completeness."""
    root = str(wheelhouse)
    require(
        wheelhouse.is_absolute() and posixpath.normpath(root) == root,
        "catalog trace directory must be an absolute normalized path",
    )
    expected = {}
    for entry in entries:
        name = entry["filename"]
        require(
            isinstance(name, str)
            and PurePosixPath(name).name == name
            and name not in ("", ".", "..")
            and "\\" not in name
            and "\0" not in name
            and name.endswith(".whl")
            and name not in expected
            and type(entry["size"]) is int
            and entry["size"] > 0,
            "invalid frozen catalog entry",
        )
        expected[name] = entry["size"]
    require(bool(expected), "empty frozen catalog")
    expected_paths = {root + "/" + name: name for name in expected}
    allowed = {root, *expected_paths}
    covered = Counter()
    enumeration_data = 0
    enumeration_eof = 0
    enumeration_pending = set()
    enumeration_complete = []
    unexpected = set()
    contradictions = []

    def contradict(reason: str, syscall: str, path: str, result: str) -> None:
        contradictions.append(
            {"reason": reason, "syscall": syscall, "path": path, "result": result}
        )

    records, _ = trace_records(text)
    for process, syscall, arguments, result, _ in records:
        if syscall not in TRACE_CALLS:
            continue
        first = first_arguments(arguments, 1)[0]
        descriptor = descriptor_number(first)
        identity = (process, descriptor)
        requested = {
            posixpath.normpath(path)
            for path in operation_paths(syscall, arguments, cwd)
        }
        returned = (
            {posixpath.normpath(path) for path in path_tokens(result)}
            if syscall == "openat"
            else set()
        )
        if syscall == "close" and result == "0" and identity in enumeration_pending:
            enumeration_pending.remove(identity)
            contradict("directory closed before EOF", syscall, root, result)
        if syscall == "openat":
            opened = (process, descriptor_number(result))
            if opened in enumeration_pending:
                enumeration_pending.remove(opened)
                contradict(
                    "directory descriptor reused before EOF", syscall, root, result
                )
        if (
            syscall == "getdents64"
            and identity in enumeration_pending
            and requested != {root}
        ):
            enumeration_pending.remove(identity)
            contradict("directory descriptor changed before EOF", syscall, root, result)
        observed = (
            requested
            | returned
            | {posixpath.normpath(path) for path in path_tokens(first)}
        )
        observed = {path for path in observed if attributable(path, root)}
        if not observed:
            continue
        for path in sorted(observed - allowed):
            unexpected.add(path)
            contradict("unexpected catalog path", syscall, path, result)
        if (
            syscall == "openat"
            and requested & allowed
            and returned
            and requested != returned
        ):
            for path in sorted(requested & allowed):
                contradict("opened catalog path differs", syscall, path, result)
        for field in (first, result if syscall == "openat" else ""):
            if " (deleted)>" in field:
                for path in sorted(path_tokens(field)):
                    if attributable(path, root):
                        contradict("deleted catalog descriptor", syscall, path, result)
        for path in sorted(requested & allowed):
            if syscall == "getdents64":
                if (
                    path != root
                    or descriptor is None
                    or re.fullmatch(r"\d+", result) is None
                ):
                    contradict("invalid directory enumeration", syscall, path, result)
                elif int(result) > 0:
                    enumeration_data += 1
                    enumeration_pending.add(identity)
                else:
                    enumeration_eof += 1
                    if identity in enumeration_pending:
                        enumeration_pending.remove(identity)
                        enumeration_complete.append(
                            {"process": process, "fd": descriptor}
                        )
                continue
            if syscall not in ("statx", "newfstatat"):
                continue
            metadata = metadata_arguments(syscall, arguments)
            if metadata is None:
                contradict("invalid metadata arguments", syscall, path, result)
                continue
            if syscall == "statx" and result.startswith("-1 ENOSYS "):
                # The ordinary implementation can fall back to newfstatat.
                continue
            if result != "0":
                contradict("failed metadata lookup", syscall, path, result)
                continue
            _, pathname, flags, attributes = metadata
            prefix = "stx_" if syscall == "statx" else "st_"
            mode = metadata_value(attributes, prefix + "mode")
            size = metadata_value(attributes, prefix + "size")
            mask = flag_set(metadata_value(attributes, "stx_mask") or "")
            has_type = syscall == "newfstatat" or bool(
                mask & {"STATX_TYPE", "STATX_BASIC_STATS", "STATX_ALL"}
            )
            has_size = syscall == "newfstatat" or bool(
                mask & {"STATX_SIZE", "STATX_BASIC_STATS", "STATX_ALL"}
            )
            file_type = "S_IFDIR" if path == root else "S_IFREG"
            if has_type and mode is not None and mode.split("|", 1)[0] != file_type:
                contradict("metadata file type differs", syscall, path, result)
                continue
            if path == root:
                continue
            name = expected_paths[path]
            if has_size and size is not None and size != str(expected[name]):
                contradict("metadata file size differs", syscall, path, result)
                continue
            flags = flag_set(flags)
            if (
                has_type
                and mode is not None
                and has_size
                and size is not None
                and "AT_SYMLINK_NOFOLLOW" in flags
                and "AT_EMPTY_PATH" not in flags
                and pathname != '""'
            ):
                covered[name] += 1
    missing = sorted(expected.keys() - covered.keys())
    return {
        "directory": root,
        "expected_entries": len(expected),
        "enumeration_data_calls": enumeration_data,
        "enumeration_eof_calls": enumeration_eof,
        "enumeration_complete": enumeration_complete,
        "enumeration_pending": [
            {"process": process, "fd": descriptor}
            for process, descriptor in sorted(enumeration_pending)
        ],
        "successful_nofollow_metadata": dict(sorted(covered.items())),
        "missing_entries": missing,
        "unexpected_paths": sorted(unexpected),
        "contradictions": contradictions,
        "accepted": bool(enumeration_complete)
        and not enumeration_pending
        and not missing
        and not unexpected
        and not contradictions,
    }


def tracee_termination(text: str, uv: Path) -> dict:
    """Bind the direct tracee's successful exec to its own terminal status."""
    records, _ = trace_records(text)
    expected = str(uv)
    require(uv.is_absolute(), "tracee executable must be an absolute checked path")
    matches = []
    for index, (process, syscall, arguments, result, _) in enumerate(records):
        if syscall != "execve" or result != "0":
            continue
        try:
            path = ast.literal_eval(first_arguments(arguments, 1)[0])
        except (SyntaxError, ValueError):
            continue
        if path == expected:
            matches.append({"process": process, "path": path, "record_index": index})
    exec_record = matches[0] if len(matches) == 1 else None
    contradictions = []
    terminal = None
    if (
        exec_record is None
        or exec_record["record_index"] != 0
        or not exec_record["process"].isdigit()
        or int(exec_record["process"]) <= 0
    ):
        contradictions.append(
            "the trace must begin with one successful execve of the checked uv"
        )
    else:
        process = exec_record["process"]
        for index, (record_process, syscall, arguments, result, _) in enumerate(
            records[1:], 1
        ):
            if record_process != process:
                continue
            if terminal is not None:
                contradictions.append("a tracee record follows its terminal status")
            elif syscall == "execve" and result == "0":
                contradictions.append("the checked uv tracee executed another image")
            elif syscall in ("process_exit", "process_killed"):
                terminal = {
                    "process": process,
                    "record_index": index,
                    "record": arguments,
                    "exit_code": int(result) if syscall == "process_exit" else None,
                    "signal": result if syscall == "process_killed" else None,
                }
        if terminal is None:
            contradictions.append("the checked uv tracee has no terminal status")
        elif terminal["exit_code"] != 0:
            contradictions.append("the checked uv tracee did not exit zero")
    return {
        "expected_executable": expected,
        "trace_filter": ["execve"],
        "matching_execve_count": len(matches),
        "execve": exec_record,
        "terminal": terminal,
        "contradictions": contradictions,
        "accepted": not contradictions,
    }


def require_capture_trace(attribution: dict, trace_path: Path) -> None:
    require(attribution["complete"], f"incomplete pathname attribution: {trace_path}")
    coverage = attribution["catalog_coverage"]
    require(
        coverage["accepted"],
        f"incomplete catalog coverage: {trace_path}; "
        f"missing={len(coverage['missing_entries'])}, "
        f"unexpected={len(coverage['unexpected_paths'])}, "
        f"contradictions={len(coverage['contradictions'])}, "
        f"enumeration={len(coverage['enumeration_complete'])} complete/"
        f"{len(coverage['enumeration_pending'])} pending",
    )
    require(
        attribution["tracee_termination"]["accepted"],
        f"incomplete tracee termination: {trace_path}; "
        + "; ".join(attribution["tracee_termination"]["contradictions"]),
    )


def clean_environment(temporary: Path) -> dict[str, str]:
    return {
        "PATH": "/usr/bin:/bin",
        "LANG": "C.UTF-8",
        "TMPDIR": str(temporary),
        "UV_PYTHON_DOWNLOADS": "never",
        "UV_NO_PROGRESS": "1",
    }


def uv_command(
    uv: Path, python: Path, cache: Path, wheelhouse: Path, requirements: Path
) -> list[str]:
    return [
        str(uv),
        "--no-config",
        "--offline",
        "--cache-dir",
        str(cache),
        "pip",
        "compile",
        "--python",
        str(python),
        "--no-index",
        "--only-binary",
        ":all:",
        "--find-links",
        str(wheelhouse),
        "--no-header",
        "--no-annotate",
        str(requirements),
    ]


def pathname_trace_command(strace: Path, trace: Path, command: list[str]) -> list[str]:
    return [
        str(strace),
        "-f",
        "-q",
        "-ttt",
        "-T",
        "-yy",
        "-s",
        "4096",
        "-e",
        f"trace={','.join(PATHNAME_TRACE_CALLS)}",
        "-o",
        str(trace),
        "--",
        *command,
    ]


def capture_process(
    command: list[str],
    environment: dict[str, str],
    directory: Path,
    label: str,
) -> dict:
    started = time.perf_counter_ns()
    result = subprocess.run(
        command,
        env=environment,
        cwd=directory,
        capture_output=True,
        check=False,
    )
    elapsed_ns = time.perf_counter_ns() - started
    stdout = directory / "results" / f"{label}.stdout"
    stderr = directory / "results" / f"{label}.stderr"
    write_new(stdout, result.stdout)
    write_new(stderr, result.stderr)
    record = {
        "label": label,
        "argv": command,
        "environment": environment,
        "exit_code": result.returncode,
        "elapsed_ns": elapsed_ns,
        "stdout_sha256": sha256_bytes(result.stdout),
        "stderr_sha256": sha256_bytes(result.stderr),
    }
    write_json(directory / "results" / f"{label}.json", record)
    require(result.returncode == 0, f"{label} failed; retained {stderr}")
    require(
        result.stdout == EXPECTED_STDOUT, f"{label} output differs; retained {stdout}"
    )
    return record


def capture(args: argparse.Namespace) -> None:
    require(
        sys.platform.startswith("linux"), "the pathname census requires native Linux"
    )
    require(bool(SAFE_ID.fullmatch(args.run_id)), "invalid run ID")
    require(
        (args.uv_source, args.uv_tree)
        in ((BASE_COMMIT, BASE_TREE), (SAVED_COMMIT, SAVED_TREE)),
        "the ordinary census must use a pinned reference source",
    )
    if args.uv_source == SAVED_COMMIT:
        require(args.uv_sha256 == SAVED_UV_SHA256, "wrong saved reference binary")
    uv = checked_file(args.uv, args.uv_sha256)
    python_target = checked_file(args.python, args.python_sha256)
    # The supplied interpreter pathname can carry virtual-environment semantics.
    python = args.python
    strace = checked_file(args.strace, args.strace_sha256)
    stage, manifest = verify_manifest(args.manifest, args.manifest_sha256)
    runs = stage / "runs"
    if runs.exists() or runs.is_symlink():
        require(stat.S_ISDIR(runs.lstat().st_mode), "runs must be an owned directory")
    else:
        runs.mkdir()
    require(runs.resolve(strict=True).is_relative_to(stage), "runs escapes the stage")
    run = new_directory(runs / args.run_id)
    metadata = {
        "schema": 1,
        "kind": "ordinary-census-not-a-speedup-comparison",
        "base_commit": BASE_COMMIT,
        "base_tree": BASE_TREE,
        "declared_uv_source": args.uv_source,
        "declared_uv_tree": args.uv_tree,
        "uv": {"path": str(uv), "sha256": args.uv_sha256},
        "python": {
            "path": str(python),
            "resolved_path": str(python_target),
            "sha256": args.python_sha256,
        },
        "strace": {"path": str(strace), "sha256": args.strace_sha256},
        "catalog_manifest_sha256": args.manifest_sha256,
        "script_sha256": sha256_file(Path(__file__).resolve()),
        "platform": platform.platform(),
        "uname": list(os.uname()),
        "catalogs": [],
    }
    write_json(run / "identity.json", metadata)
    for catalog in manifest["catalogs"]:
        directory = new_directory(run / catalog["id"])
        for name in ("cache", "tmp", "results", "traces"):
            (directory / name).mkdir()
        wheelhouse = relative_file(stage, catalog["path"]).resolve(strict=True)
        requirements = relative_file(stage, manifest["requirements"])
        command = uv_command(uv, python, directory / "cache", wheelhouse, requirements)
        environment = clean_environment(directory / "tmp")
        first = capture_process(command, environment, directory, "first-uv-cache-use")
        warm = [
            capture_process(command, environment, directory, f"warm-{index}")
            for index in range(1, 4)
        ]
        summary_path = directory / "traces" / "summary.txt"
        summary_command = [
            str(strace),
            "-f",
            "-qq",
            "-c",
            "-e",
            f"trace={','.join(TRACE_CALLS)}",
            "-o",
            str(summary_path),
            "--",
            *command,
        ]
        capture_process(summary_command, environment, directory, "strace-summary")
        trace_path = directory / "traces" / "pathnames.txt"
        trace_command = pathname_trace_command(strace, trace_path, command)
        capture_process(trace_command, environment, directory, "strace-pathnames")
        trace_text = trace_path.read_text()
        attribution = attribute_trace(trace_text, wheelhouse, directory)
        attribution["catalog_coverage"] = catalog_trace_coverage(
            trace_text, wheelhouse, catalog["entries"], directory
        )
        attribution["tracee_termination"] = tracee_termination(trace_text, uv)
        write_json(directory / "traces" / "attribution.json", attribution)
        require_capture_trace(attribution, trace_path)
        metadata["catalogs"].append(
            {
                "id": catalog["id"],
                "entry_count": catalog["count"],
                "valid_distribution_filenames": catalog["count"],
                "requested_name_candidates": 1,
                "entries_sha256": catalog["entries_sha256"],
                "first_uv_cache_use_ns": first["elapsed_ns"],
                "warm_elapsed_ns": [record["elapsed_ns"] for record in warm],
                "ordinary_warm_median_ns": statistics.median(
                    record["elapsed_ns"] for record in warm
                ),
                "summary_sha256": sha256_file(summary_path),
                "pathnames_sha256": sha256_file(trace_path),
                "attribution": attribution,
            }
        )
    verify_manifest(args.manifest, args.manifest_sha256)
    report_path = run / "report.json"
    digest = write_json(report_path, metadata)
    print(json.dumps({"report": str(report_path), "sha256": digest}, sort_keys=True))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subcommands.add_parser(
        "prepare", help="generate and validate all synthetic catalogs"
    )
    prepare_parser.add_argument("--generator", type=Path, required=True)
    prepare_parser.add_argument("--generator-sha256", required=True)
    prepare_parser.add_argument("--stage", type=Path, required=True)
    verify_parser = subcommands.add_parser(
        "verify", help="verify an immutable prepared manifest"
    )
    capture_parser = subcommands.add_parser(
        "capture", help="capture the ordinary native Linux census"
    )
    for command in (verify_parser, capture_parser):
        command.add_argument("--manifest", type=Path, required=True)
        command.add_argument("--manifest-sha256", required=True)
    for name in ("uv", "python", "strace"):
        capture_parser.add_argument(f"--{name}", type=Path, required=True)
        capture_parser.add_argument(f"--{name}-sha256", required=True)
    capture_parser.add_argument("--uv-source", required=True)
    capture_parser.add_argument("--uv-tree", required=True)
    capture_parser.add_argument("--run-id", required=True)
    attribute_parser = subcommands.add_parser(
        "attribute", help="recompute one pathname attribution"
    )
    attribute_parser.add_argument("--trace", type=Path, required=True)
    attribute_parser.add_argument("--wheelhouse", type=Path, required=True)
    attribute_parser.add_argument("--cwd", type=Path)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args)
    elif args.command == "verify":
        _, manifest = verify_manifest(args.manifest, args.manifest_sha256)
        print(
            json.dumps(
                {"counts": [catalog["count"] for catalog in manifest["catalogs"]]}
            )
        )
    elif args.command == "capture":
        capture(args)
    else:
        print(
            json.dumps(
                attribute_trace(args.trace.read_text(), args.wheelhouse, args.cwd),
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    main()
