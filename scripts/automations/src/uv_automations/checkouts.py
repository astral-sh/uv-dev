"""Inspect agent-owned checkouts through fresh, trusted Git metadata."""

import errno
import os
import re
import shutil
import stat
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


def _copy_regular_file(
    source_directory: int, name: str, flags: int, destination: Path
) -> None:
    try:
        descriptor = os.open(name, flags, dir_fd=source_directory)
    except OSError as error:
        if error.errno in {errno.ELOOP, errno.ENOENT}:
            raise ValueError(f"Missing or unsafe candidate metadata: {name}") from error
        raise
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise ValueError(f"Candidate metadata must be a regular file: {name}")
        with (
            os.fdopen(descriptor, "rb", closefd=False) as source,
            destination.open("xb") as output,
        ):
            shutil.copyfileobj(source, output)
    finally:
        os.close(descriptor)


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
        """Reject in-progress rebases and staged, tracked, or untracked changes."""
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
    """Clone an ordinary Actions checkout for post-agent inspection.

    `scratch` must be protected from the agent and outside its writable temporary
    roots as well as the checkout. Git 2.50.1+ and native no-follow directory
    descriptors are required; unsupported layouts and platforms fail closed.
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
        index_snapshot = root / "source-index"
        index_snapshot.mkdir()
        _copy_regular_file(metadata, "index", file_flags, index_snapshot / "index")
        for name in sorted(names):
            if _SHARED_INDEX.fullmatch(name) is not None:
                _copy_regular_file(metadata, name, file_flags, index_snapshot / name)

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
                str(source_path),
                str(clone_path),
            )
        )
        repository = Git(clone_path, token_variable=source.token_variable)
        head = repository.resolve_commit("HEAD")
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
