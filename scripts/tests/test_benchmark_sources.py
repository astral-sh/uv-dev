"""Complete source fixtures retain files omitted from release archives."""

import importlib.util
import io
import os
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "prepare_sources",
    Path(__file__).resolve().parents[1] / "benchmark/prepare-sources.py",
)
assert SPEC is not None and SPEC.loader is not None
sources = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sources)


class CompleteSourceFixtures(unittest.TestCase):
    def test_preserves_export_ignored_files_and_source_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository = root / "upstream"
            repository.mkdir()
            environment = os.environ.copy()
            environment.update(
                GIT_CONFIG_NOSYSTEM="1",
                GIT_CONFIG_GLOBAL=os.devnull,
                GIT_AUTHOR_NAME="Test",
                GIT_AUTHOR_EMAIL="test@example.com",
                GIT_COMMITTER_NAME="Test",
                GIT_COMMITTER_EMAIL="test@example.com",
            )
            command = ["git", "-C", str(repository)]
            subprocess.run(
                [*command, "-c", "init.templateDir=", "init", "--quiet"],
                check=True,
                env=environment,
            )
            files = {
                ".gitattributes": b"dev export-ignore\nversion.txt export-subst\n",
                "pyproject.toml": b"[tool.uv.workspace]\nmembers = ['dev/member']\n",
                "dev/.gitattributes": b"pyproject.toml export-ignore\n",
                "dev/member/pyproject.toml": b"[project]\nname = 'member'\nversion = '1'\n",
                "version.txt": b"$Format:%H$\n",
            }
            for name, content in files.items():
                path = repository / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            subprocess.run([*command, "add", "."], check=True, env=environment)
            subprocess.run(
                [
                    *command,
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "Fixture",
                ],
                check=True,
                env=environment,
            )
            commit = subprocess.check_output(
                [*command, "rev-parse", "HEAD"], text=True, env=environment
            ).strip()
            snapshot = sources.complete_git_snapshot(
                {"repository": repository.as_uri(), "commit": commit}, root, environment
            )
            with tarfile.open(fileobj=io.BytesIO(snapshot)) as archive:
                actual = {}
                for member in archive:
                    if member.isfile():
                        stream = archive.extractfile(member)
                        assert stream is not None
                        actual[member.name] = stream.read()
            self.assertEqual(actual, files)


if __name__ == "__main__":
    unittest.main()
