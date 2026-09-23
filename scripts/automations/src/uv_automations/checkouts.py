"""Inspect agent-owned checkouts through fresh, trusted Git metadata."""

import errno
import os
import re
import shutil
import stat
import subprocess
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from pathlib import Path
from tempfile import TemporaryDirectory

from uv_automations.git import Git
from uv_automations.models import CommitSha

# This includes `--attr-source`, the untrusted-clone fixes, and the later
# bundle-URI clone fix. Do not fall back to inspecting the source repository.
# https://github.com/git/git/security/advisories/GHSA-vm9j-46j9-qvq4
# https://github.com/git/git/security/advisories/GHSA-m98c-vgpc-9655
_MINIMUM_GIT_VERSION = (2, 50, 1)
_OBJECT_ID = re.compile(r"[0-9a-f]{40}")
_SHARED_INDEX = re.compile(r"sharedindex\.[0-9a-f]{40}")
_LOOSE_DIRECTORY = re.compile(r"[0-9a-f]{2}")
_LOOSE_OBJECT = re.compile(r"[0-9a-f]{38}")
_PACK_FILE = re.compile(r"(pack-[0-9a-f]{40})\.(pack|idx)")
_PROMISOR_FILE = re.compile(r"pack-[0-9a-f]{40}\.promisor")
_MAX_SNAPSHOT_FILES = 100_000
_MAX_SNAPSHOT_BYTES = 2 * 1024**3
_MAX_REFERENCE_BYTES = 16 * 1024**2
_COPY_CHUNK_BYTES = 1024**2


@dataclass(slots=True)
class _SnapshotBudget:
    file_count: int = 0
    byte_count: int = 0

    def reserve(self, size: int) -> None:
        if (
            size < 0
            or self.file_count >= _MAX_SNAPSHOT_FILES
            or size > _MAX_SNAPSHOT_BYTES - self.byte_count
        ):
            raise ValueError("Candidate metadata exceeds the snapshot limits")
        self.file_count += 1
        self.byte_count += size


@dataclass(frozen=True, slots=True, order=True)
class _Entry:
    path: str
    mode: str
    object_id: str
    stage: int = 0


def _entry(path: str, mode: str, object_id: str, stage: int) -> _Entry:
    if mode == "160000":
        raise ValueError("Candidate inspection does not support Git submodules")
    if (
        mode not in {"100644", "100755", "120000"}
        or _OBJECT_ID.fullmatch(object_id) is None
        or stage not in {0, 1, 2, 3}
        or any(part in {"", ".", ".."} for part in path.split("/"))
    ):
        raise ValueError("Git returned an unsupported candidate index entry")
    return _Entry(path, mode, object_id, stage)


def _records(value: str) -> tuple[str, ...]:
    if not value:
        return ()
    if not value.endswith("\0"):
        raise ValueError("Git returned an incomplete candidate index")
    return tuple(value[:-1].split("\0"))


def _index_entries(repository: Git) -> tuple[_Entry, ...]:
    entries: list[_Entry] = []
    for record in _records(
        repository.output("ls-files", "--stage", "--full-name", "-z")
    ):
        metadata, separator, path = record.partition("\t")
        fields = metadata.split()
        if not separator or len(fields) != 3 or fields[2] not in {"0", "1", "2", "3"}:
            raise ValueError("Git returned an invalid candidate index")
        entries.append(_entry(path, fields[0], fields[1], int(fields[2])))
    if len({(entry.path, entry.stage) for entry in entries}) != len(entries):
        raise ValueError("Git returned duplicate candidate index entries")
    return tuple(sorted(entries))


def _tree_entries(repository: Git, revision: CommitSha) -> tuple[_Entry, ...]:
    entries: list[_Entry] = []
    for record in _records(
        repository.output("ls-tree", "-r", "--full-tree", "-z", str(revision))
    ):
        metadata, separator, path = record.partition("\t")
        fields = metadata.split()
        if not separator or len(fields) != 3 or fields[1] not in {"blob", "commit"}:
            raise ValueError("Git returned an invalid candidate tree")
        entries.append(_entry(path, fields[0], fields[2], 0))
    if len({entry.path for entry in entries}) != len(entries):
        raise ValueError("Git returned duplicate candidate tree entries")
    return tuple(sorted(entries))


def _require_git_version(repository: Git) -> None:
    version = repository.output("--version")
    match = re.fullmatch(r"git version (\d+)\.(\d+)\.(\d+)(?:[. -][^\r\n]*)?", version)
    if match is None or tuple(map(int, match.groups())) < _MINIMUM_GIT_VERSION:
        raise ValueError(
            f"Candidate inspection requires Git 2.50.1 or newer; found {version!r}"
        )


def _open_flags() -> tuple[int, int]:
    if (
        os.open not in os.supports_dir_fd
        or os.listdir not in os.supports_fd
        or not getattr(os, "O_DIRECTORY", 0)
        or not getattr(os, "O_NOFOLLOW", 0)
        or not getattr(os, "O_NONBLOCK", 0)
    ):
        raise OSError(
            errno.ENOTSUP,
            "Secure candidate inspection requires no-follow directory descriptors",
        )
    return (
        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
        os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
    )


def _open_directory(
    name: str, flags: int, descriptors: ExitStack, *, parent: int | None = None
) -> int:
    try:
        descriptor = os.open(name, flags, dir_fd=parent)
    except OSError as error:
        if error.errno in {errno.ELOOP, errno.ENOTDIR}:
            raise ValueError(
                "Candidate inspection requires ordinary checkout directories"
            ) from error
        raise
    descriptors.callback(os.close, descriptor)
    return descriptor


def _open_checkout(path: Path, flags: int, descriptors: ExitStack) -> int:
    descriptor = _open_directory(path.anchor, flags, descriptors)
    for part in path.parts[1:]:
        descriptor = _open_directory(part, flags, descriptors, parent=descriptor)
    return descriptor


def _file_state(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


@contextmanager
def _open_regular_file(
    source_directory: int, name: str, flags: int
) -> Iterator[tuple[int, os.stat_result]]:
    try:
        descriptor = os.open(name, flags, dir_fd=source_directory)
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise ValueError(f"Missing or unsafe candidate metadata: {name}") from error
        raise
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError(f"Candidate metadata must be a regular file: {name}")
        if metadata.st_nlink != 1:
            raise ValueError(f"Candidate metadata must not be hardlinked: {name}")
        yield descriptor, metadata
        if _file_state(metadata) != _file_state(os.fstat(descriptor)):
            raise ValueError(f"Candidate metadata changed while reading: {name}")
    finally:
        os.close(descriptor)


def _copy_regular_file(
    source_directory: int,
    name: str,
    flags: int,
    destination: Path,
    budget: _SnapshotBudget,
) -> None:
    try:
        with _open_regular_file(source_directory, name, flags) as (
            descriptor,
            metadata,
        ):
            budget.reserve(metadata.st_size)
            with (
                os.fdopen(descriptor, "rb", closefd=False) as source,
                destination.open("xb") as output,
            ):
                remaining = metadata.st_size
                while remaining:
                    contents = source.read(min(remaining, _COPY_CHUNK_BYTES))
                    if not contents:
                        raise ValueError(
                            f"Candidate metadata changed while reading: {name}"
                        )
                    output.write(contents)
                    remaining -= len(contents)
                if source.read(1):
                    raise ValueError(
                        f"Candidate metadata changed while reading: {name}"
                    )
    except FileNotFoundError as error:
        raise ValueError(f"Missing or unsafe candidate metadata: {name}") from error


def _read_metadata(
    source_directory: int,
    name: str,
    flags: int,
    budget: _SnapshotBudget,
    *,
    limit: int = _MAX_REFERENCE_BYTES,
) -> bytes:
    with _open_regular_file(source_directory, name, flags) as (descriptor, metadata):
        if metadata.st_size > limit:
            raise ValueError(f"Candidate reference metadata is too large: {name}")
        budget.reserve(metadata.st_size)
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            contents = source.read(metadata.st_size + 1)
        if len(contents) != metadata.st_size:
            raise ValueError(f"Candidate metadata changed while reading: {name}")
        return contents


def _read_loose_reference(
    metadata: int,
    reference: str,
    directory_flags: int,
    file_flags: int,
    budget: _SnapshotBudget,
) -> CommitSha | None:
    try:
        with ExitStack() as descriptors:
            directory = metadata
            *parents, name = reference.split("/")
            for parent in parents:
                directory = _open_directory(
                    parent, directory_flags, descriptors, parent=directory
                )
            contents = _read_metadata(directory, name, file_flags, budget, limit=4096)
    except FileNotFoundError:
        return None
    return CommitSha(contents.decode("ascii").removesuffix("\n"))


def _read_head(
    bootstrap: Git,
    metadata: int,
    directory_flags: int,
    file_flags: int,
    budget: _SnapshotBudget,
) -> CommitSha:
    contents = _read_metadata(metadata, "HEAD", file_flags, budget, limit=4096)
    head = contents.decode("utf-8").removesuffix("\n")
    if _OBJECT_ID.fullmatch(head) is not None:
        return CommitSha(head)
    if not head.startswith("ref: refs/heads/"):
        raise ValueError("Candidate HEAD must name a SHA-1 commit or ordinary branch")
    reference = head.removeprefix("ref: ")
    if bootstrap.command(("check-ref-format", reference), check=False).returncode:
        raise ValueError("Candidate HEAD names an invalid branch")
    if loose := _read_loose_reference(
        metadata, reference, directory_flags, file_flags, budget
    ):
        return loose
    try:
        packed = _read_metadata(metadata, "packed-refs", file_flags, budget)
    except FileNotFoundError as error:
        raise ValueError("Candidate HEAD does not resolve to a commit") from error
    matches = []
    for line in packed.split(b"\n"):
        object_id, separator, name = line.partition(b" ")
        if separator and name == reference.encode("utf-8"):
            matches.append(CommitSha(object_id.decode("ascii")))
    if len(matches) != 1:
        raise ValueError("Candidate HEAD must have exactly one packed reference")
    return matches[0]


def _copy_objects(
    metadata: int,
    destination: Path,
    directory_flags: int,
    file_flags: int,
    budget: _SnapshotBudget,
) -> None:
    with ExitStack() as descriptors:
        objects = _open_directory(
            "objects", directory_flags, descriptors, parent=metadata
        )
        names = set(os.listdir(objects))
        if "info" in names:
            info = _open_directory("info", directory_flags, descriptors, parent=objects)
            if set(os.listdir(info)) & {"alternates", "http-alternates"}:
                raise ValueError(
                    "Candidate inspection does not support alternate object stores"
                )
        for name in sorted(names):
            if _LOOSE_DIRECTORY.fullmatch(name) is None:
                continue
            directory = _open_directory(
                name, directory_flags, descriptors, parent=objects
            )
            target = destination / "objects" / name
            target.mkdir()
            for object_name in sorted(os.listdir(directory)):
                if _LOOSE_OBJECT.fullmatch(object_name) is not None:
                    _copy_regular_file(
                        directory, object_name, file_flags, target / object_name, budget
                    )
        if "pack" not in names:
            return
        packs = _open_directory("pack", directory_flags, descriptors, parent=objects)
        pack_names = set(os.listdir(packs))
        if any(_PROMISOR_FILE.fullmatch(name) for name in pack_names):
            raise ValueError("Candidate inspection does not support promisor packs")
        selected: dict[str, set[str]] = {}
        for name in pack_names:
            if match := _PACK_FILE.fullmatch(name):
                selected.setdefault(match[1], set()).add(match[2])
        if any(extensions != {"pack", "idx"} for extensions in selected.values()):
            raise ValueError("Candidate inspection requires complete pack/index pairs")
        for basename in sorted(selected):
            for extension in ("pack", "idx"):
                name = f"{basename}.{extension}"
                _copy_regular_file(
                    packs,
                    name,
                    file_flags,
                    destination / "objects" / "pack" / name,
                    budget,
                )


def _snapshot_repository(
    bootstrap: Git,
    metadata: int,
    destination: Path,
    base: CommitSha,
    directory_flags: int,
    file_flags: int,
    budget: _SnapshotBudget,
) -> tuple[Git, CommitSha]:
    head = _read_head(bootstrap, metadata, directory_flags, file_flags, budget)
    bootstrap.command(
        (
            "init",
            "--bare",
            "--quiet",
            "--object-format=sha1",
            "--ref-format=files",
            "--initial-branch=candidate",
            "--template=",
            str(destination),
        )
    )
    _copy_objects(metadata, destination, directory_flags, file_flags, budget)
    repository = Git(destination, token_variable=bootstrap.token_variable)
    try:
        if repository.resolve_commit(str(head)) != head:
            raise ValueError("Candidate HEAD did not resolve to its captured commit")
        repository.command(("update-ref", "refs/heads/candidate", str(head)))
        repository.command(
            (
                "fsck",
                "--strict",
                "--full",
                "--no-reflogs",
                "--no-dangling",
                "--no-progress",
                str(head),
                str(base),
            )
        )
    except subprocess.CalledProcessError as error:
        raise ValueError(
            "Candidate object snapshot is incomplete or corrupt"
        ) from error
    return repository, head


@dataclass(frozen=True, slots=True)
class InspectedCandidate:
    repository: Git
    head: CommitSha
    _base: CommitSha
    _worktree: Path
    _original_index: tuple[_Entry, ...]
    _head_tree: tuple[_Entry, ...]
    _rebase_in_progress: bool

    def require_clean(self) -> None:
        """Check the live worktree for in-progress or uncommitted changes.

        This is a consistency check, not an atomic working-tree snapshot. Only
        the independently copied Git objects are used for commit transport.
        """
        if self._rebase_in_progress:
            raise ValueError("The rebase is still in progress")
        if self._original_index != self._head_tree:
            raise ValueError("The candidate index is not clean")

        # Never let source index flags or cached-tree extensions determine
        # whether the working tree is clean. Start with a genuinely new index.
        (self.repository.path / "index").unlink(missing_ok=True)
        self.repository.command(("read-tree", "--no-sparse-checkout", str(self.head)))
        if self.repository.output(
            f"--git-dir={self.repository.path}",
            f"--work-tree={self._worktree}",
            f"--attr-source={self._base}",
            "-c",
            "core.bare=false",
            "-c",
            f"core.attributesFile={os.devnull}",
            "-c",
            f"core.excludesFile={os.devnull}",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.sparseCheckout=false",
            "-c",
            "core.splitIndex=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.ignoreStat=false",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=all",
        ):
            raise ValueError("The candidate worktree is not clean")


@contextmanager
def inspect_candidate(
    source: Git, *, base: CommitSha, scratch: Path
) -> Iterator[InspectedCandidate]:
    """Snapshot an ordinary Actions checkout for post-agent inspection.

    `scratch` must be protected from the agent and outside its writable temporary
    roots as well as the checkout. Git 2.50.1+ and native no-follow directory
    descriptors are required; unsupported layouts and platforms fail closed.
    At most 100,000 source files and 2 GiB are copied. Transport reads only that
    protected object snapshot; `require_clean` checks the live working tree.
    """
    directory_flags, file_flags = _open_flags()
    source_path = Path(os.path.abspath(source.path))
    scratch = scratch.resolve(strict=True)
    if not scratch.is_dir() or scratch.is_relative_to(source_path):
        raise ValueError("Candidate inspection requires scratch outside the checkout")

    with (
        TemporaryDirectory(prefix=".inspect-candidate-", dir=scratch) as temporary,
        ExitStack() as descriptors,
    ):
        root = Path(temporary)
        bootstrap = Git(root, token_variable=source.token_variable)
        _require_git_version(bootstrap)

        checkout = _open_checkout(source_path, directory_flags, descriptors)
        metadata = _open_directory(
            ".git", directory_flags, descriptors, parent=checkout
        )
        names = set(os.listdir(metadata))
        if "commondir" in names:
            raise ValueError("Candidate inspection does not support linked worktrees")
        if "shallow" in names:
            raise ValueError(
                "Candidate inspection does not support shallow repositories"
            )
        if "reftable" in names:
            raise ValueError(
                "Candidate inspection does not support reftable repositories"
            )
        budget = _SnapshotBudget()
        index_snapshot = root / "source-index"
        index_snapshot.mkdir()
        _copy_regular_file(
            metadata, "index", file_flags, index_snapshot / "index", budget
        )
        for name in sorted(names):
            if _SHARED_INDEX.fullmatch(name) is not None:
                _copy_regular_file(
                    metadata, name, file_flags, index_snapshot / name, budget
                )

        snapshot, captured_head = _snapshot_repository(
            bootstrap,
            metadata,
            root / "source.git",
            base,
            directory_flags,
            file_flags,
            budget,
        )

        clone_path = root / "repository.git"
        bootstrap.command(
            (
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.file.allow=always",
                "-c",
                "fetch.fsckObjects=true",
                "clone",
                "--bare",
                "--no-local",
                "--no-checkout",
                "--no-tags",
                "--no-recurse-submodules",
                "--single-branch",
                "--reject-shallow",
                "--template=",
                "--",
                str(snapshot.path),
                str(clone_path),
            )
        )
        repository = Git(clone_path, token_variable=source.token_variable)
        head = repository.resolve_commit("HEAD")
        if head != captured_head:
            raise ValueError("The candidate snapshot changed its captured HEAD")
        if repository.resolve_commit(str(base)) != base:
            raise ValueError("The candidate does not contain its trusted base")
        repository.command(("config", "attr.tree", str(base)))
        repository.command(("config", "core.attributesFile", os.devnull))
        repository.command(("config", "core.excludesFile", os.devnull))
        head_tree = _tree_entries(repository, head)
        _tree_entries(repository, base)
        for path in index_snapshot.iterdir():
            shutil.copyfile(path, clone_path / path.name)
        original_index = _index_entries(repository)
        (clone_path / "index").unlink()

        yield InspectedCandidate(
            repository,
            head,
            base,
            source_path,
            original_index,
            head_tree,
            bool(names & {"rebase-merge", "rebase-apply"}),
        )
