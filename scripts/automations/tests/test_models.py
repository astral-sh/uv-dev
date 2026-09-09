import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from uv_automations.actions import write_json_output
from uv_automations.models import (
    CommitSha,
    ManagedRepository,
    PullRequestRef,
    RepositoryName,
)


class ModelTests(unittest.TestCase):
    def test_invalid_repository(self) -> None:
        for name in ["uv", "/uv", "a/b/c", "a/..", "a/.", "a/b\n"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                RepositoryName(name)

    def test_invalid_commit(self) -> None:
        for sha in ["a" * 39, "a" * 41, "A" * 40, "main", "-x"]:
            with self.subTest(sha=sha), self.assertRaises(ValueError):
                CommitSha(sha)

    def test_invalid_pull_request_number(self) -> None:
        for number in [0, -1, True]:
            with self.subTest(number=number), self.assertRaises(ValueError):
                PullRequestRef(RepositoryName("astral-sh/uv"), number)

    def test_managed_repositories_match_dispatch_manifest(self) -> None:
        repository = Path(__file__).resolve().parents[3]
        dispatch = json.loads(
            (repository / ".github/automations-dispatch.json").read_text()
        )
        self.assertEqual(
            {item.value for item in ManagedRepository},
            {item["name"] for item in dispatch["repositories"].values()},
        )

    def test_actions_output_is_one_line(self) -> None:
        with TemporaryDirectory() as directory:
            path = Path(directory) / "github-output"
            write_json_output(path, "value", ["first\nsecond", "third\rfourth"])
            self.assertEqual(
                path.read_text(), 'value=["first\\nsecond","third\\rfourth"]\n'
            )

    def test_invalid_actions_output_name(self) -> None:
        with TemporaryDirectory() as directory:
            path = Path(directory) / "github-output"
            with self.assertRaises(ValueError):
                write_json_output(path, "value\nother", [])
            self.assertFalse(path.exists())
