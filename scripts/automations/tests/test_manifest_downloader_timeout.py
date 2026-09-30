import base64
import subprocess
import threading
import time
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import Mock, patch

from test_action_manifests import archive_contents, downloader, manifest_artifact

from uv_automations import github_actions
from uv_automations.github_actions import ActionsGitHub


def holding_downloader(path: Path, content: bytes, *, escaped: bool) -> Path:
    encoded = base64.b64encode(content).decode()
    return downloader(
        path,
        "import base64, os, pathlib, sys, time\n"
        "directory = pathlib.Path(__file__).parent\n"
        "if os.fork() == 0:\n"
        + ("    os.setsid()\n" if escaped else "")
        + "    (directory / 'ready').write_text(str(os.getpid()))\n"
        "    deadline = time.monotonic() + 10\n"
        "    while not (directory / 'release').exists() and time.monotonic() < deadline:\n"
        "        time.sleep(0.01)\n"
        "    (directory / 'released').touch()\n"
        "    os._exit(0)\n"
        f"sys.stdout.buffer.write(base64.b64decode({encoded!r}))\n"
        "sys.stdout.buffer.flush()\n"
        "os._exit(0)",
    )


class ManifestDownloaderTimeoutTests(unittest.TestCase):
    def held_pipe(self, *, escaped: bool) -> None:
        content = archive_contents()
        with TemporaryDirectory() as directory_name:
            directory = Path(directory_name)
            executable = holding_downloader(
                directory / ("held-escaped" if escaped else "held-group"),
                content,
                escaped=escaped,
            )
            completed = threading.Event()
            outcome: list[object] = []

            def read() -> None:
                try:
                    outcome.append(
                        ActionsGitHub(executable=str(executable)).read_json_manifest(
                            manifest_artifact(content)
                        )
                    )
                except (
                    OSError,
                    RuntimeError,
                    ValueError,
                    subprocess.SubprocessError,
                ) as error:
                    outcome.append(error)
                finally:
                    completed.set()

            with patch("uv_automations.github_actions.MANIFEST_TIMEOUT_SECONDS", 1):
                reader = threading.Thread(target=read)
                reader.start()
                try:
                    ready_deadline = time.monotonic() + 3
                    while (
                        not (directory / "ready").exists()
                        and time.monotonic() < ready_deadline
                    ):
                        time.sleep(0.01)
                    self.assertTrue((directory / "ready").exists())
                    finished_at_deadline = completed.wait(timeout=2)
                finally:
                    (directory / "release").touch()
                    reader.join(timeout=10)
                    self.assertFalse(reader.is_alive())
                    if escaped:
                        release_deadline = time.monotonic() + 3
                        while (
                            not (directory / "released").exists()
                            and time.monotonic() < release_deadline
                        ):
                            time.sleep(0.01)
                        self.assertTrue((directory / "released").exists())
            self.assertTrue(
                finished_at_deadline,
                "manifest download remained blocked after its deadline",
            )
            self.assertEqual(len(outcome), 1)
            self.assertIsInstance(outcome[0], subprocess.TimeoutExpired)

    def test_exited_downloader_cannot_leave_the_manifest_read_blocked(self) -> None:
        self.held_pipe(escaped=False)

    def test_escaped_child_cannot_extend_the_manifest_deadline(self) -> None:
        self.held_pipe(escaped=True)

    def test_unverified_group_is_never_signaled(self) -> None:
        process = Mock(spec=subprocess.Popen, pid=12345)
        process.poll.return_value = None
        with (
            patch("uv_automations.github_actions.os.getpgrp", return_value=54321),
            patch("uv_automations.github_actions.os.killpg") as kill_group,
        ):
            github_actions._stop_manifest_downloader(process, None)
            github_actions._stop_manifest_downloader(process, 54321)
            github_actions._stop_manifest_downloader(process, 12346)
        kill_group.assert_not_called()
        self.assertEqual(process.kill.call_count, 3)
        self.assertEqual(process.wait.call_count, 3)
        process.wait.assert_called_with(timeout=1)

    def test_zombie_group_permission_error_keeps_the_timeout(self) -> None:
        process = Mock(spec=subprocess.Popen, pid=12345)
        process.poll.return_value = 0
        with (
            patch("uv_automations.github_actions.os.getpgrp", return_value=54321),
            patch(
                "uv_automations.github_actions.os.killpg",
                side_effect=PermissionError("zombie-only group"),
            ) as kill_group,
            self.assertRaises(subprocess.TimeoutExpired),
        ):
            try:
                raise subprocess.TimeoutExpired(["downloader"], 60)
            finally:
                github_actions._stop_manifest_downloader(process, 12345)
        kill_group.assert_called_once()
        process.kill.assert_not_called()
        process.wait.assert_called_once_with(timeout=1)

    def test_unsettled_downloader_cleanup_is_bounded_and_reported(self) -> None:
        process = Mock(spec=subprocess.Popen, pid=12345)
        process.poll.return_value = None
        process.kill.side_effect = PermissionError("owned child")
        process.wait.side_effect = subprocess.TimeoutExpired(["downloader"], 1)
        with (
            patch("uv_automations.github_actions.os.getpgrp", return_value=54321),
            patch(
                "uv_automations.github_actions.os.killpg",
                side_effect=PermissionError("owned group"),
            ),
            patch("uv_automations.github_actions.logger.warning") as warning,
        ):
            github_actions._stop_manifest_downloader(process, 12345)
        process.kill.assert_called_once()
        process.wait.assert_called_once_with(timeout=1)
        self.assertEqual(warning.call_count, 3)
