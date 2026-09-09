import os
import subprocess
import unittest
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field, replace
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import override
from unittest.mock import patch

from uv_automations.artifacts import CommitRange
from uv_automations.git import Git
from uv_automations.models import (
    CommitSha,
    PullRequestDetails,
    PullRequestRef,
    PullRequestRevision,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
)
from uv_automations.workflows.rebase import (
    CloseOutcome,
    EmptyRebase,
    LoadedRebase,
    PersistedRebase,
    PreparedRebase,
    PushOutcome,
    RebaseSource,
    SkippedRebase,
    VerifiedEmptyRebase,
    VerifiedRebaseSource,
    close_empty_rebase,
    load_rebase,
    persist_rebase,
    prepare_rebase,
    push_rebase,
    verify_empty_rebase,
    verify_rebase_source,
)

UV = RepositoryIdentity(RepositoryName("astral-sh/uv"), 699532645)
UV_DEV = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)


@dataclass(frozen=True, slots=True)
class LocalGit(Git):
    remotes: Mapping[str, Path]
    commands: list[tuple[str, ...]] = field(default_factory=list, compare=False)
    credentials: list[tuple[str | None, tuple[str, ...]]] = field(
        default_factory=list, compare=False
    )

    @override
    def command(
        self,
        arguments: Sequence[str],
        *,
        check: bool = True,
        input: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        self.commands.append(tuple(arguments))
        self.credentials.append((self.token_variable, tuple(arguments)))
        local_arguments = [str(self.remotes.get(value, value)) for value in arguments]
        return Git.command(self, local_arguments, check=check, input=input)


@dataclass
class FakeGitHub:
    snapshots: list[PullRequestDetails]
    closed: list[tuple[PullRequestRef, str]] = field(default_factory=list)

    def get_pull_request(self, reference: PullRequestRef) -> PullRequestDetails:
        current = self.snapshots[0]
        if len(self.snapshots) > 1:
            self.snapshots.pop(0)
        if current.reference != reference:
            raise AssertionError("Unexpected pull request")
        return current

    def close_pull_request(self, reference: PullRequestRef, *, comment: str) -> None:
        self.closed.append((reference, comment))


def details(rebase: PreparedRebase) -> PullRequestDetails:
    source = rebase.source
    head_repository = UV_DEV if source.head_repository == UV_DEV.name else UV
    return PullRequestDetails(
        reference=source.reference,
        state=PullRequestState.OPEN,
        url=f"https://github.com/{source.repository.name}/pull/{source.number}",
        base=PullRequestRevision(source.repository, source.base_ref, rebase.base_sha),
        head=PullRequestRevision(head_repository, source.head_ref, source.head_sha),
        labels=("bot:rebase",),
    )


class RebaseTests(unittest.TestCase):
    @override
    def setUp(self) -> None:
        self.enterContext(patch.dict(os.environ, {"GH_READ_TOKEN": "test-read-token"}))
        directory = TemporaryDirectory(prefix="uv-automations-rebase-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        remote_path = self.root / "remote"
        remote_path.mkdir()
        self.remote = Git(remote_path)
        self.remote.command(("init", "--quiet", "--initial-branch=main"))
        self.remote.command(("config", "user.name", "Automation tests"))
        self.remote.command(("config", "user.email", "automation@example.invalid"))
        self.common = self.commit(
            self.remote,
            {"change.txt": "original\n", "parent.txt": "original\n"},
            "common",
        )

    def commit(
        self, repository: Git, files: Mapping[str, str], message: str
    ) -> CommitSha:
        for name, content in files.items():
            (repository.path / name).write_text(content, encoding="utf-8")
        repository.command(("add", "--all"))
        repository.command(("commit", "--quiet", "--message", message))
        return repository.resolve_commit("HEAD")

    def clone(self, name: str = "checkout") -> LocalGit:
        path = self.root / name
        Git(self.root).command(
            ("clone", "--quiet", "--no-hardlinks", str(self.remote.path), str(path))
        )
        repository = LocalGit(
            path,
            {
                "https://github.com/astral-sh/uv.git": self.remote.path,
                "https://github.com/astral-sh/uv-dev.git": self.remote.path,
            },
        )
        repository.command(("config", "user.name", "Automation tests"))
        repository.command(("config", "user.email", "automation@example.invalid"))
        return repository

    def history(self, *, base_content: str, head_content: str) -> PreparedRebase:
        self.remote.command(("checkout", "--quiet", "-b", "feature", str(self.common)))
        head = self.commit(self.remote, {"change.txt": head_content}, "feature")
        self.remote.command(("checkout", "--quiet", "main"))
        if base_content == "original\n":
            base = self.common
        else:
            base = self.commit(self.remote, {"change.txt": base_content}, "upstream")
        return PreparedRebase(
            RebaseSource(
                repository=UV_DEV,
                number=123,
                base_ref="main",
                head_repository=UV_DEV.name,
                head_ref="feature",
                head_sha=head,
            ),
            base,
        )

    def test_prepare_checks_out_the_pinned_head(self) -> None:
        rebase = self.history(base_content="original\n", head_content="feature\n")
        repository = self.clone()
        prepared = prepare_rebase(
            FakeGitHub([details(rebase)]), repository, rebase.source
        )
        self.assertEqual(prepared, rebase)
        self.assertEqual(repository.resolve_commit("HEAD"), rebase.source.head_sha)
        self.assertIn(
            ("checkout", "--detach", str(rebase.source.head_sha)), repository.commands
        )

    def test_prepare_rejects_changed_metadata(self) -> None:
        rebase = self.history(base_content="original\n", head_content="feature\n")
        repository = self.clone()
        original_head = repository.resolve_commit("HEAD")
        current = details(rebase)
        github = FakeGitHub(
            [replace(current, head=replace(current.head, sha=self.common))]
        )
        with self.assertRaisesRegex(ValueError, "stale revision"):
            prepare_rebase(github, repository, rebase.source)
        self.assertEqual(repository.resolve_commit("HEAD"), original_head)

    def test_empty_rebase_has_no_artifact(self) -> None:
        rebase = self.history(base_content="upstream\n", head_content="feature\n")
        repository = self.clone()
        bundle = self.root / "rebased.bundle"
        self.assertEqual(
            persist_rebase(repository, rebase.base_sha, bundle),
            EmptyRebase(rebase.base_sha),
        )
        self.assertFalse(bundle.exists())

    def test_nonempty_rebase_uses_the_shared_bundle_contract(self) -> None:
        rebase = self.history(base_content="upstream\n", head_content="feature\n")
        producer = self.clone("producer")
        head = self.commit(producer, {"new.txt": "rebased\n"}, "rebased")
        bundle = self.root / "rebased.bundle"
        persisted = persist_rebase(producer, rebase.base_sha, bundle)
        self.assertIsInstance(persisted, PersistedRebase)
        consumer = self.clone("consumer")
        original_head = consumer.resolve_commit("HEAD")
        loaded = load_rebase(
            consumer, "main", CommitRange(rebase.base_sha, head), bundle
        )
        self.assertEqual(loaded, LoadedRebase(head))
        self.assertEqual(consumer.resolve_commit("HEAD"), original_head)

    def test_rejects_unfinished_and_dirty_rebases(self) -> None:
        rebase = self.history(base_content="upstream\n", head_content="feature\n")
        repository = self.clone()
        rebase_path = repository.path / repository.output(
            "rev-parse", "--git-path", "rebase-merge"
        )
        rebase_path.mkdir()
        with self.assertRaisesRegex(ValueError, "still in progress"):
            persist_rebase(repository, rebase.base_sha, self.root / "unfinished.bundle")
        rebase_path.rmdir()
        (repository.path / "change.txt").write_text("dirty\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "not clean"):
            persist_rebase(repository, rebase.base_sha, self.root / "dirty.bundle")

    def test_independent_verification_does_not_check_out_pr_code(self) -> None:
        rebase = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        repository = self.clone()
        original_head = repository.resolve_commit("HEAD")
        verified = verify_empty_rebase(
            FakeGitHub([details(rebase)]), repository, rebase
        )
        self.assertEqual(verified, VerifiedEmptyRebase(rebase, UV_DEV))
        self.assertEqual(repository.resolve_commit("HEAD"), original_head)
        self.assertFalse(
            any("checkout" in arguments for arguments in repository.commands)
        )

    def test_original_changes_remain_open(self) -> None:
        rebase = self.history(base_content="original\n", head_content="feature\n")
        result = verify_empty_rebase(
            FakeGitHub([details(rebase)]), self.clone(), rebase
        )
        self.assertEqual(
            result,
            SkippedRebase(
                "The original pull request still has changes; leaving it open"
            ),
        )

    def test_original_conflicts_remain_open(self) -> None:
        rebase = self.history(base_content="upstream\n", head_content="feature\n")
        result = verify_empty_rebase(
            FakeGitHub([details(rebase)]), self.clone(), rebase
        )
        self.assertEqual(
            result,
            SkippedRebase(
                "The original pull request does not merge cleanly; leaving it open"
            ),
        )

    def test_stacked_verification_excludes_the_previous_parent(self) -> None:
        self.remote.command(("checkout", "--quiet", "-b", "feature", str(self.common)))
        previous = self.commit(self.remote, {"parent.txt": "parent\n"}, "parent")
        head = self.commit(self.remote, {"change.txt": "already merged\n"}, "child")
        self.remote.command(("checkout", "--quiet", "main"))
        base = self.commit(self.remote, {"change.txt": "already merged\n"}, "upstream")
        rebase = PreparedRebase(
            RebaseSource(
                repository=UV_DEV,
                number=123,
                base_ref="main",
                head_repository=UV_DEV.name,
                head_ref="feature",
                head_sha=head,
                previous_base=previous,
            ),
            base,
        )
        repository = self.clone()
        self.assertEqual(
            verify_empty_rebase(FakeGitHub([details(rebase)]), repository, rebase),
            VerifiedEmptyRebase(rebase, UV_DEV),
        )
        without_parent = replace(
            rebase, source=replace(rebase.source, previous_base=None)
        )
        self.assertIsInstance(
            verify_empty_rebase(
                FakeGitHub([details(without_parent)]), repository, without_parent
            ),
            SkippedRebase,
        )

    def test_verification_rejects_stale_repository_and_revision_data(self) -> None:
        rebase = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        repository = self.clone()
        original = details(rebase)
        wrong_id = RepositoryIdentity(UV_DEV.name, 1)
        invalid = (
            replace(original, state=PullRequestState.CLOSED),
            replace(original, base=replace(original.base, repository=wrong_id)),
            replace(original, base=replace(original.base, ref="other")),
            replace(original, base=replace(original.base, sha=self.common)),
            replace(original, head=replace(original.head, repository=wrong_id)),
            replace(original, head=replace(original.head, repository=None)),
            replace(original, head=replace(original.head, ref="other")),
            replace(original, head=replace(original.head, sha=self.common)),
        )
        for current in invalid:
            with self.subTest(current=current):
                self.assertIsInstance(
                    verify_empty_rebase(FakeGitHub([current]), repository, rebase),
                    SkippedRebase,
                )
        self.assertFalse(
            any("merge-tree" in arguments for arguments in repository.commands)
        )

    def test_verification_rechecks_after_merging(self) -> None:
        rebase = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        current = details(rebase)
        changed = replace(current, base=replace(current.base, sha=self.common))
        self.assertEqual(
            verify_empty_rebase(FakeGitHub([current, changed]), self.clone(), rebase),
            SkippedRebase(
                "The pull request changed during verification; leaving it open"
            ),
        )

    def test_verification_rejects_a_moved_remote_branch(self) -> None:
        rebase = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        repository = self.clone()
        self.remote.command(("update-ref", "refs/heads/feature", str(rebase.base_sha)))
        self.assertEqual(
            verify_empty_rebase(FakeGitHub([details(rebase)]), repository, rebase),
            SkippedRebase("The pull request head changed; leaving it open"),
        )

    def test_verification_rejects_an_unrelated_previous_parent(self) -> None:
        original = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        rebase = replace(
            original, source=replace(original.source, previous_base=original.base_sha)
        )
        with self.assertRaisesRegex(ValueError, "previous base"):
            verify_empty_rebase(FakeGitHub([details(rebase)]), self.clone(), rebase)

    def test_cross_repository_closure_pins_the_head_repository_id(self) -> None:
        original = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        rebase = replace(original, source=replace(original.source, repository=UV))
        current = details(rebase)
        verified = verify_empty_rebase(FakeGitHub([current]), self.clone(), rebase)
        self.assertEqual(verified, VerifiedEmptyRebase(rebase, UV_DEV))
        changed = replace(
            current,
            head=replace(current.head, repository=RepositoryIdentity(UV_DEV.name, 1)),
        )
        github = FakeGitHub([changed])
        self.assertEqual(
            close_empty_rebase(github, VerifiedEmptyRebase(rebase, UV_DEV), run_id=456),
            CloseOutcome.STALE,
        )
        self.assertEqual(github.closed, [])

    def test_close_rechecks_before_writing(self) -> None:
        rebase = self.history(
            base_content="already merged\n", head_content="already merged\n"
        )
        current = details(rebase)
        verified = VerifiedEmptyRebase(rebase, UV_DEV)
        stale = FakeGitHub(
            [replace(current, head=replace(current.head, sha=self.common))]
        )
        self.assertEqual(
            close_empty_rebase(stale, verified, run_id=456), CloseOutcome.STALE
        )
        self.assertEqual(stale.closed, [])
        github = FakeGitHub([current])
        self.assertEqual(
            close_empty_rebase(github, verified, run_id=456), CloseOutcome.CLOSED
        )
        self.assertEqual(
            github.closed,
            [
                (
                    rebase.source.reference,
                    (
                        "Closing automatically because the changes in this pull request are already "
                        f"present in the base branch at {rebase.base_sha}, leaving no changes after rebasing."
                        "\n\nRebase run: https://github.com/astral-sh/uv-dev/actions/runs/456"
                    ),
                )
            ],
        )

    def test_push_uses_an_exact_lease_and_no_token_in_the_url(self) -> None:
        rebase = self.history(base_content="upstream\n", head_content="feature\n")
        repository = self.clone()
        head = self.commit(repository, {"new.txt": "rebased\n"}, "rebased")
        github = FakeGitHub([details(rebase)])
        verified = VerifiedRebaseSource(rebase, UV_DEV)
        self.assertEqual(
            push_rebase(github, repository, verified, head), PushOutcome.PUSHED
        )
        self.assertEqual(self.remote.resolve_commit("refs/heads/feature"), head)
        pushes = [arguments for arguments in repository.commands if "push" in arguments]
        self.assertEqual(len(pushes), 1)
        self.assertEqual(
            pushes[0][-4:],
            (
                "push",
                f"--force-with-lease=refs/heads/feature:{rebase.source.head_sha}",
                "https://github.com/astral-sh/uv-dev.git",
                f"{head}:refs/heads/feature",
            ),
        )
        self.assertEqual(
            [
                token
                for token, arguments in repository.credentials
                if "ls-remote" in arguments
            ],
            ["GH_READ_TOKEN", "GH_READ_TOKEN"],
        )
        self.assertEqual(
            [
                token
                for token, arguments in repository.credentials
                if "push" in arguments
            ],
            [None],
        )
        self.assertEqual(
            push_rebase(github, repository, verified, head), PushOutcome.STALE
        )

    def test_source_preflight_pins_repository_identity(self) -> None:
        original = self.history(base_content="upstream\n", head_content="feature\n")
        rebase = replace(original, source=replace(original.source, repository=UV))
        self.assertEqual(
            verify_rebase_source(FakeGitHub([details(rebase)]), rebase),
            VerifiedRebaseSource(rebase, UV_DEV),
        )
        closed = replace(details(rebase), state=PullRequestState.CLOSED)
        self.assertIsInstance(
            verify_rebase_source(FakeGitHub([closed]), rebase), SkippedRebase
        )

    def test_push_rechecks_current_pull_request_metadata(self) -> None:
        original = self.history(base_content="upstream\n", head_content="feature\n")
        rebase = replace(original, source=replace(original.source, repository=UV))
        repository = self.clone()
        head = self.commit(repository, {"new.txt": "rebased\n"}, "rebased")
        current = details(rebase)
        verified = VerifiedRebaseSource(rebase, UV_DEV)
        for changed in (
            replace(current, state=PullRequestState.CLOSED),
            replace(current, base=replace(current.base, ref="retargeted")),
            replace(
                current,
                head=replace(
                    current.head, repository=RepositoryIdentity(UV_DEV.name, 1)
                ),
            ),
        ):
            with self.subTest(changed=changed):
                self.assertEqual(
                    push_rebase(FakeGitHub([changed]), repository, verified, head),
                    PushOutcome.STALE,
                )
        self.assertFalse(any("push" in arguments for arguments in repository.commands))

    def test_source_rejects_unmanaged_heads_and_invalid_refs(self) -> None:
        head = CommitSha("a" * 40)
        for repository, head_repository, base_ref, head_ref, previous in (
            (UV_DEV, UV.name, "main", "feature", None),
            (UV_DEV, RepositoryName("contributor/uv"), "main", "feature", None),
            (UV, UV_DEV.name, "main", "feature", head),
            (UV_DEV, UV_DEV.name, "../bad", "feature", None),
            (UV_DEV, UV_DEV.name, "main", "../bad", None),
        ):
            with (
                self.subTest(repository=repository, head_repository=head_repository),
                self.assertRaises(ValueError),
            ):
                RebaseSource(
                    repository=repository,
                    number=123,
                    base_ref=base_ref,
                    head_repository=head_repository,
                    head_ref=head_ref,
                    head_sha=head,
                    previous_base=previous,
                )
