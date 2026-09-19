import subprocess
import unittest
from unittest.mock import patch

from benchmark.resolver import create_lockfile


class CreateLockfileTest(unittest.TestCase):
    def test_creates_lockfile(self):
        command = ["uv", "lock"]
        with (
            patch("benchmark.resolver.subprocess.check_call") as check_call,
            patch("benchmark.resolver.os.path.exists", return_value=True) as exists,
        ):
            create_lockfile(command, cwd="project", lockfile="project/uv.lock")

        check_call.assert_called_once_with(
            command,
            cwd="project",
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        exists.assert_called_once_with("project/uv.lock")

    def test_missing_lockfile(self):
        with (
            patch("benchmark.resolver.subprocess.check_call"),
            patch("benchmark.resolver.os.path.exists", return_value=False),
            self.assertRaises(AssertionError) as caught,
        ):
            create_lockfile(["uv", "lock"], cwd="project", lockfile="project/uv.lock")

        self.assertEqual(
            str(caught.exception), "Lockfile doesn't exist at: project/uv.lock"
        )

    def test_subprocess_failure(self):
        failure = subprocess.CalledProcessError(2, ["uv", "lock"])
        with (
            patch("benchmark.resolver.subprocess.check_call", side_effect=failure),
            patch("benchmark.resolver.os.path.exists") as exists,
            self.assertRaises(subprocess.CalledProcessError) as caught,
        ):
            create_lockfile(["uv", "lock"], cwd="project", lockfile="project/uv.lock")

        self.assertIs(caught.exception, failure)
        exists.assert_not_called()


if __name__ == "__main__":
    unittest.main()
