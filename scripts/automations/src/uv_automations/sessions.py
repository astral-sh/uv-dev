"""Bounded snapshots of one verified root Codex session and its child sessions."""

import os
import re
import stat
from collections.abc import Iterator
from compression import zstd
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path, PurePath
from uuid import UUID

from uv_automations.json import as_object, as_string, loads, require_keys

MAX_SESSION_FILES = 1_000
MAX_SESSION_FILE_BYTES = 64 * 1024 * 1024
MAX_SESSION_BYTES = 256 * 1024 * 1024
MAX_METADATA_BYTES = 64 * 1024
MAX_SESSION_ENTRIES = 4_000
MAX_SESSION_DEPTH = 16


@dataclass(frozen=True, slots=True)
class SessionFile:
    path: PurePath
    content: bytes

    def __post_init__(self) -> None:
        if (
            self.path.is_absolute()
            or any(part in {".", ".."} for part in self.path.parts)
            or re.fullmatch(r"rollout-[A-Za-z0-9_.-]+\.jsonl", self.path.name) is None
            or any(
                re.fullmatch(r"[A-Za-z0-9_.-]+", part) is None
                for part in self.path.parts
            )
            or len(self.content) > MAX_SESSION_FILE_BYTES
        ):
            raise ValueError("Invalid retained Codex rollout")


@dataclass(frozen=True, slots=True)
class CodexSessionSnapshot:
    identifier: UUID
    files: tuple[SessionFile, ...]

    def __post_init__(self) -> None:
        if (
            not self.files
            or len(self.files) > MAX_SESSION_FILES
            or len({file.path for file in self.files}) != len(self.files)
            or sum(len(file.content) for file in self.files) > MAX_SESSION_BYTES
        ):
            raise ValueError("Invalid bounded Codex session snapshot")


def _same_entry(expected: os.stat_result, actual: os.stat_result) -> bool:
    return (expected.st_dev, expected.st_ino, stat.S_IFMT(expected.st_mode)) == (
        actual.st_dev,
        actual.st_ino,
        stat.S_IFMT(actual.st_mode),
    )


def _directory_flags() -> int:
    try:
        return os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    except AttributeError as error:
        raise OSError(
            "This platform cannot securely traverse Codex sessions"
        ) from error


@contextmanager
def _source_directory(source: Path, trusted_root: Path) -> Iterator[int]:
    if (
        not source.is_absolute()
        or not trusted_root.is_absolute()
        or ".." in source.parts
        or ".." in trusted_root.parts
    ):
        raise ValueError("Session paths must be absolute and unambiguous")
    relative = source.relative_to(trusted_root)
    flags = _directory_flags()
    descriptor = os.open(trusted_root, flags)
    try:
        for part in relative.parts:
            child = os.open(part, flags, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        yield descriptor
    finally:
        os.close(descriptor)


def _session_id(value: object) -> UUID:
    text = as_string(value)
    identifier = UUID(text)
    if str(identifier) != text:
        raise ValueError("Invalid Codex session identifier")
    return identifier


def _parent_session(payload: dict[str, object]) -> UUID:
    source = as_object(payload["source"])
    require_keys(source, {"subagent"})
    parent = payload.get("parent_thread_id")
    subagent = source["subagent"]
    if isinstance(subagent, dict):
        data = as_object(subagent)
        if "thread_spawn" in data:
            require_keys(data, {"thread_spawn"})
            declared = as_object(data["thread_spawn"])["parent_thread_id"]
            if parent is not None and parent != declared:
                raise ValueError("A Codex subagent has inconsistent parent metadata")
            parent = declared
        else:
            require_keys(data, {"other"})
            as_string(data["other"])
    elif subagent not in {"review", "compact", "memory_consolidation"}:
        raise ValueError("Unexpected Codex subagent source")
    if parent is None:
        raise ValueError("A retained Codex subagent must identify its parent")
    return _session_id(parent)


def _check_lineage(root: UUID, parents: dict[UUID, UUID]) -> None:
    for identifier in parents:
        seen = {identifier}
        current = identifier
        while current != root:
            if current not in parents or parents[current] in seen:
                raise ValueError(
                    "A retained Codex subagent is not descended from the root"
                )
            current = parents[current]
            seen.add(current)


def _read_rollout(directory: int, name: str, expected: os.stat_result) -> bytes:
    descriptor = os.open(
        name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
    )
    with os.fdopen(descriptor, "rb") as source:
        actual = os.fstat(source.fileno())
        if not stat.S_ISREG(actual.st_mode) or not _same_entry(expected, actual):
            raise ValueError("A Codex rollout must remain the same regular file")
        if expected.st_nlink != 1 or actual.st_nlink != 1:
            raise ValueError("A Codex rollout must not be hardlinked")
        if name.endswith(".zst"):
            with zstd.open(source, "rb") as decompressed:
                content = decompressed.read(MAX_SESSION_FILE_BYTES + 1)
        else:
            content = source.read(MAX_SESSION_FILE_BYTES + 1)
    if len(content) > MAX_SESSION_FILE_BYTES:
        raise ValueError("A Codex rollout exceeds the size limit")
    return content


@dataclass(slots=True)
class _SessionWalk:
    entries: int = 0

    def files(
        self, directory: int, relative: PurePath, depth: int = 0
    ) -> Iterator[tuple[PurePath, bytes]]:
        if depth > MAX_SESSION_DEPTH:
            raise ValueError("The Codex session tree is too deep")
        entries: list[tuple[str, os.stat_result]] = []
        with os.scandir(directory) as iterator:
            for entry in iterator:
                self.entries += 1
                if self.entries > MAX_SESSION_ENTRIES:
                    raise ValueError("The Codex session tree contains too many entries")
                entries.append((entry.name, entry.stat(follow_symlinks=False)))
        for name, expected in sorted(entries):
            if stat.S_ISDIR(expected.st_mode):
                if re.fullmatch(r"[A-Za-z0-9_.-]+", name) is None:
                    raise ValueError("Invalid Codex session directory name")
                child = os.open(name, _directory_flags(), dir_fd=directory)
                try:
                    if not _same_entry(expected, os.fstat(child)):
                        raise ValueError(
                            "A Codex session directory changed during traversal"
                        )
                    yield from self.files(child, relative / name, depth + 1)
                finally:
                    os.close(child)
            elif stat.S_ISREG(expected.st_mode):
                if (
                    re.fullmatch(r"rollout-[A-Za-z0-9_.-]+\.jsonl(?:\.zst)?", name)
                    is None
                ):
                    raise ValueError("Unexpected file in Codex session artifact")
                yield relative / name, _read_rollout(directory, name, expected)
            else:
                raise ValueError(
                    "A Codex session artifact may contain only regular files and directories"
                )


def snapshot_sessions(
    source: Path, workspace: Path, *, trusted_root: Path
) -> CodexSessionSnapshot:
    """Read beneath a root outside every agent-writable ancestor of `source`.

    The caller supplies a trusted runner-owned directory, not a resolved path
    through an agent-owned Codex home. Every descendant is opened relative to a
    held directory descriptor, without following symlinks.
    """
    files: list[SessionFile] = []
    paths: set[PurePath] = set()
    sessions: set[UUID] = set()
    parents: dict[UUID, UUID] = {}
    root_identifier: UUID | None = None
    size = 0
    with _source_directory(source, trusted_root) as directory:
        for relative, content in _SessionWalk().files(directory, PurePath()):
            size += len(content)
            if size > MAX_SESSION_BYTES or len(files) >= MAX_SESSION_FILES:
                raise ValueError("The Codex session artifact exceeds the size limit")
            metadata_line = content.split(b"\n", 1)[0]
            if len(metadata_line) > MAX_METADATA_BYTES:
                raise ValueError("The Codex session metadata is too large")
            metadata = as_object(loads(metadata_line.decode("utf-8")))
            if metadata["type"] != "session_meta":
                raise ValueError("A Codex rollout must begin with session metadata")
            payload = as_object(metadata["payload"])
            identifier = _session_id(payload["id"])
            if identifier in sessions:
                raise ValueError("Duplicate Codex session identifier")
            sessions.add(identifier)
            if payload.get("source") == "exec":
                if (
                    root_identifier is not None
                    or payload.get("originator") != "codex_github_action"
                    or payload.get("cwd") != str(workspace)
                    or payload.get("parent_thread_id") is not None
                ):
                    raise ValueError(
                        "The root Codex session does not match the workflow"
                    )
                root_identifier = identifier
            else:
                parents[identifier] = _parent_session(payload)
            if relative.name.endswith(".zst"):
                relative = relative.with_suffix("")
            if relative in paths:
                raise ValueError("Duplicate compressed and uncompressed Codex rollout")
            paths.add(relative)
            files.append(SessionFile(relative, content))
    if root_identifier is None:
        raise ValueError("The artifact must contain exactly one root Codex session")
    _check_lineage(root_identifier, parents)
    return CodexSessionSnapshot(root_identifier, tuple(files))


def write_sessions(snapshot: CodexSessionSnapshot, destination: Path) -> None:
    """Create a fresh sessions directory; never merge with an existing Codex home."""
    destination.mkdir(parents=True, exist_ok=False)
    for file in snapshot.files:
        path = destination / file.path
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as output:
            output.write(file.content)
