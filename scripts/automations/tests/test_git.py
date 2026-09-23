import os
import subprocess
import unittest
from dataclasses import dataclass
from pathlib import Path
from unittest.mock import patch

from uv_automations.git import Git


@dataclass(frozen=True, slots=True)
class LocalGit(Git):
    marker: str


class GitCredentialTests(unittest.TestCase):
    def test_selected_credentials_only_change_the_child_environment(self) -> None:
        with (
            patch.dict(
                os.environ, {"GH_TOKEN": "write-token", "GH_READ_TOKEN": "read-token"}
            ),
            patch("uv_automations.git.subprocess.run") as run,
        ):
            run.return_value = subprocess.CompletedProcess([], 0, "", "")
            repository = Git(Path("repository"), token_variable="GH_READ_TOKEN")
            repository.command(("rev-parse", "HEAD"))
            self.assertEqual(run.call_args.kwargs["env"]["GH_TOKEN"], "read-token")
            self.assertEqual(os.environ["GH_TOKEN"], "write-token")
            self.assertNotIn("read-token", repr(repository))
            self.assertNotIn("read-token", run.call_args.args[0])

    def test_missing_or_empty_selected_credentials_fail_closed(self) -> None:
        for value in (None, ""):
            environment = {"GH_TOKEN": "write-token"}
            if value is not None:
                environment["GH_READ_TOKEN"] = value
            with (
                self.subTest(value=value),
                patch.dict(os.environ, environment, clear=True),
                patch("uv_automations.git.subprocess.run") as run,
                self.assertRaisesRegex(ValueError, "Missing Git token environment"),
            ):
                Git(Path("repository"), token_variable="GH_READ_TOKEN").command(
                    ("rev-parse", "HEAD")
                )
            run.assert_not_called()

    def test_with_token_preserves_subclasses_and_can_restore_ambient_auth(self) -> None:
        repository = LocalGit(Path("repository"), "marker")
        selected = repository.with_token("GH_READ_TOKEN")
        self.assertIsInstance(selected, LocalGit)
        self.assertEqual(selected.marker, "marker")
        self.assertEqual(selected.token_variable, "GH_READ_TOKEN")
        self.assertIsNone(repository.token_variable)
        self.assertEqual(selected.with_token(None), repository)
