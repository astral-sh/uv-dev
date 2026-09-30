"""Prepared Git fixtures support filtered fetches and missing-object recovery."""

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "prepare_git",
    Path(__file__).resolve().parents[1] / "benchmark/prepare-git.py",
)
assert SPEC is not None and SPEC.loader is not None
git_fixtures = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(git_fixtures)


class GitFixtures(unittest.TestCase):
    def test_filtered_fetch_retains_history_and_recovers_objects(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            upstream = root / "upstream"
            upstream.mkdir()
            environment = os.environ.copy()
            environment.update(
                GIT_CONFIG_NOSYSTEM="1",
                GIT_CONFIG_GLOBAL=os.devnull,
                GIT_AUTHOR_NAME="Test",
                GIT_AUTHOR_EMAIL="test@example.com",
                GIT_COMMITTER_NAME="Test",
                GIT_COMMITTER_EMAIL="test@example.com",
                GIT_TERMINAL_PROMPT="0",
            )

            def git(directory: Path, *args: str, no_lazy_fetch: bool = False) -> str:
                return subprocess.check_output(
                    ["git", "-C", str(directory), *args],
                    text=True,
                    env=environment
                    | ({"GIT_NO_LAZY_FETCH": "1"} if no_lazy_fetch else {}),
                    stderr=subprocess.PIPE,
                ).strip()

            git(upstream, "-c", "init.templateDir=", "init", "--quiet")
            for value in ("first\n", "second\n"):
                (upstream / "data.txt").write_text(value)
                git(upstream, "add", "data.txt")
                git(upstream, "-c", "commit.gpgsign=false", "commit", "-qm", "Fixture")
            commit = git(upstream, "rev-parse", "HEAD")
            tree = git(upstream, "rev-parse", "HEAD^{tree}")
            git_fixtures.configure_upload_pack(upstream)

            client = root / "client"
            client.mkdir()
            git(client, "-c", "init.templateDir=", "init", "--bare", "--quiet")
            git(
                client,
                "fetch",
                "--filter=tree:0",
                upstream.as_uri(),
                f"{commit}:refs/heads/captured",
            )
            self.assertEqual(
                git(client, "rev-list", "--count", commit, no_lazy_fetch=True), "2"
            )
            missing = subprocess.check_output(
                [
                    "git",
                    "-C",
                    str(client),
                    "cat-file",
                    "--batch-check=%(objectname) %(objecttype)",
                ],
                input=tree + "\n",
                text=True,
                env=environment | {"GIT_NO_LAZY_FETCH": "1"},
            ).strip()
            self.assertEqual(missing, f"{tree} missing")
            self.assertEqual(git(client, "show", f"{commit}:data.txt"), "second")


if __name__ == "__main__":
    unittest.main()
