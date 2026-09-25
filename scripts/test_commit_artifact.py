"""Exercise the commit-artifact actions with real Git repositories."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ACTIONS = Path(__file__).resolve().parents[1] / ".github" / "actions"


class CommitArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="commit-artifact-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.source = self.directory / "source"
        self.consumer = self.directory / "consumer"
        self.environment = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "Test",
            "GIT_AUTHOR_EMAIL": "test@example.com",
            "GIT_COMMITTER_NAME": "Test",
            "GIT_COMMITTER_EMAIL": "test@example.com",
            "RUNNER_TEMP": str(self.directory),
        }
        self.source.mkdir()
        self.git(self.source, "init", "--quiet")
        self.base = self.commit("base")
        self.git(
            self.directory,
            "clone",
            "--quiet",
            "--no-local",
            str(self.source),
            str(self.consumer),
        )
        self.head = self.commit("head")
        self.sequence = 0

    def git(self, directory, *arguments):
        return subprocess.check_output(
            ["git", "-C", str(directory), *arguments],
            env=self.environment,
            text=True,
            stderr=subprocess.PIPE,
        ).strip()

    def commit(self, content):
        (self.source / "file").write_text(content + "\n")
        self.git(self.source, "add", "file")
        self.git(self.source, "commit", "--quiet", "-m", content)
        return self.git(self.source, "rev-parse", "HEAD")

    def invoke(self, action, directory, *, success=True, **inputs):
        self.sequence += 1
        output = self.directory / f"output-{self.sequence}"
        result = subprocess.run(
            [
                "bash",
                str(
                    ACTIONS
                    / action
                    / ("persist.sh" if action == "persist-commit" else "load.sh")
                ),
            ],
            cwd=directory,
            env={**self.environment, "GITHUB_OUTPUT": str(output), **inputs},
            text=True,
            capture_output=True,
            check=False,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(
                output.exists(), "Rejected commits must not produce outputs"
            )
        return (
            dict(line.split("=", 1) for line in output.read_text().splitlines())
            if output.exists()
            else {}
        )

    def persist(self, *, base=None, head="HEAD", success=True):
        return self.invoke(
            "persist-commit",
            self.source,
            success=success,
            BASE_SHA=base or self.base,
            HEAD_SHA=head,
        )

    def load(self, bundle, *, base=None, head=None, success=True):
        return self.invoke(
            "load-commit",
            self.consumer,
            success=success,
            BASE_SHA=base or self.base,
            HEAD_SHA=head or self.head,
            BUNDLE=str(bundle),
        )

    def bundle(self, head, *, base=None, references=None):
        references = references or [f"refs/uv-automations/commits/{head}"]
        bundle = self.directory / "custom.bundle"
        for reference in references:
            self.git(self.source, "update-ref", reference, head)
        try:
            revisions = ([f"^{base}"] if base else []) + references
            self.git(self.source, "bundle", "create", str(bundle), *revisions)
        finally:
            for reference in references:
                self.git(self.source, "update-ref", "-d", reference)
        return bundle

    def test_round_trip_keeps_the_trusted_checkout(self):
        self.head = self.commit("second commit")
        self.git(self.source, "checkout", "--quiet", "--detach", self.base)
        original_refs = self.git(self.source, "show-ref")
        persisted = self.persist(head=self.head)
        self.assertEqual(persisted["head-sha"], self.head)
        self.assertEqual(self.git(self.source, "show-ref"), original_refs)
        self.assertEqual(self.git(self.source, "rev-parse", "HEAD"), self.base)

        fetch_head = self.consumer / ".git" / "FETCH_HEAD"
        fetch_head.write_text("preserve this\n")
        original_refs = self.git(self.consumer, "show-ref")
        marker = self.directory / "hook-ran"
        hook = self.consumer / ".git" / "hooks" / "reference-transaction"
        hook.write_text(f'#!/bin/sh\ntouch "{marker}"\n')
        hook.chmod(0o755)

        self.assertEqual(self.load(persisted["path"]), {"head-sha": self.head})
        self.assertEqual(
            self.git(self.consumer, "show", f"{self.head}:file"), "second commit"
        )
        self.assertEqual(self.git(self.consumer, "rev-parse", "HEAD"), self.base)
        self.assertEqual(self.git(self.consumer, "show-ref"), original_refs)
        self.assertEqual(fetch_head.read_text(), "preserve this\n")
        self.assertEqual((self.consumer / "file").read_text(), "base\n")
        self.assertFalse(marker.exists())

    def test_default_head(self):
        persisted = self.persist()
        self.assertEqual(self.load(persisted["path"]), {"head-sha": self.head})

    def test_shallow_producer(self):
        shallow = self.directory / "shallow"
        self.git(
            self.directory,
            "clone",
            "--quiet",
            "--depth=1",
            self.source.as_uri(),
            str(shallow),
        )
        self.git(self.consumer, "fetch", "--quiet", str(self.source), self.head)
        self.source = shallow
        self.base = self.head
        self.head = self.commit("shallow producer")
        persisted = self.persist()
        self.assertEqual(self.load(persisted["path"]), {"head-sha": self.head})

    def test_reject_invalid_revisions(self):
        for base, head in [
            ("main", self.head),
            (self.base, "--all"),
            (self.head, self.base),
            (self.base, self.base),
        ]:
            with self.subTest(base=base, head=head):
                self.persist(base=base, head=head, success=False)
        self.git(self.source, "tag", "-a", "tag", "-m", "tag")
        tag = self.git(self.source, "rev-parse", "tag")
        self.persist(head=tag, success=False)

    def test_existing_transport_ref_is_not_overwritten(self):
        reference = f"refs/uv-automations/commits/{self.head}"
        self.git(self.source, "update-ref", reference, self.base)
        self.persist(success=False)
        self.assertEqual(self.git(self.source, "rev-parse", reference), self.base)

    def test_reject_wrong_sha_and_refs(self):
        persisted = self.persist()
        self.load(persisted["path"], head=self.base, success=False)
        self.load(persisted["path"], head="HEAD", success=False)
        for references in [
            ["refs/heads/unexpected"],
            [f"refs/uv-automations/commits/{self.head}", "refs/heads/extra"],
        ]:
            with self.subTest(references=references):
                self.load(
                    self.bundle(self.head, base=self.base, references=references),
                    success=False,
                )

    def test_reject_non_commit_object(self):
        self.git(self.source, "tag", "-a", "tag", "-m", "tag")
        tag = self.git(self.source, "rev-parse", "tag")
        self.load(self.bundle(tag, base=self.base), head=tag, success=False)

    def test_reject_unrelated_history(self):
        tree = self.git(self.source, "rev-parse", f"{self.head}^{{tree}}")
        unrelated = self.git(self.source, "commit-tree", tree, "-m", "unrelated")
        self.load(self.bundle(unrelated), head=unrelated, success=False)

    def test_replace_refs_cannot_fake_ancestry(self):
        tree = self.git(self.source, "rev-parse", f"{self.head}^{{tree}}")
        unrelated = self.git(self.source, "commit-tree", tree, "-m", "unrelated")
        bundle = self.bundle(unrelated)
        self.git(self.consumer, "fetch", "--quiet", str(self.source), self.head)
        self.git(self.consumer, "update-ref", f"refs/replace/{unrelated}", self.head)
        self.load(bundle, head=unrelated, success=False)

    def test_reject_missing_prerequisite(self):
        parent = self.head
        self.head = self.commit("requires missing parent")
        persisted = self.persist(base=parent)
        self.load(persisted["path"], success=False)

    def test_reject_corrupt_or_symlinked_bundle(self):
        persisted = self.persist()
        bundle = Path(persisted["path"])
        symlink = self.directory / "symlink.bundle"
        symlink.symlink_to(bundle)
        self.load(symlink, success=False)
        corrupt = self.directory / "corrupt.bundle"
        shutil.copyfile(bundle, corrupt)
        with corrupt.open("r+b") as file:
            file.truncate(corrupt.stat().st_size - 20)
        self.load(corrupt, success=False)


if __name__ == "__main__":
    unittest.main()
