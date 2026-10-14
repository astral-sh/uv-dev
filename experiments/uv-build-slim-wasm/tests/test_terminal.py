"""Compare native terminal diagnostics with the generated public backend."""

import os
import select
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


@unittest.skipIf(os.name == "nt", "This probe uses POSIX pseudoterminals")
class TerminalParity(unittest.TestCase):
    def test_debug_colors_follow_stderr_terminal(self):
        import pty

        scratch = Path(
            os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
        )
        reference = os.environ["UV_SLIM_REFERENCE"]
        package = Path(__file__).resolve().parents[1] / "src"
        with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
            root = Path(temporary)
            (root / "pyproject.toml").write_text(
                '[project]\nname="demo"\nversion="1.0"\n[build-system]\nrequires=["uv_build>=0.13,<0.14"]\nbuild-backend="uv_build"\n'
            )
            output = root / "out"
            output.mkdir()
            environment = dict(
                os.environ,
                TERM="xterm-256color",
                CLICOLOR="1",
                RUST_LOG="debug",
                UV_PREVIEW="1",
                PYTHONPATH=str(package),
                PYTHONDONTWRITEBYTECODE="1",
            )
            for name in ["NO_COLOR", "CLICOLOR_FORCE", "CI"]:
                environment.pop(name, None)

            def capture(command):
                master, slave = pty.openpty()
                try:
                    process = subprocess.Popen(
                        command,
                        cwd=root,
                        env=environment,
                        stdout=subprocess.PIPE,
                        stderr=slave,
                    )
                    try:
                        stdout, _ = process.communicate(timeout=60)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.communicate()
                        raise
                    stderr = bytearray()
                    # Darwin discards unread PTY output when the last slave
                    # closes. Keep this descriptor until the short log is read.
                    while select.select([master], [], [], 0)[0]:
                        data = os.read(master, 65536)
                        if not data:
                            break
                        stderr.extend(data)
                    return process.returncode, stdout, bytes(stderr)
                finally:
                    os.close(slave)
                    os.close(master)

            expected = capture(
                [reference, "prepare-metadata-for-build-wheel", str(output)]
            )
            shutil.rmtree(output / "demo-1.0.dist-info")
            actual = capture(
                [
                    sys.executable,
                    "-c",
                    "import sys, uv_build; print(uv_build.prepare_metadata_for_build_wheel(sys.argv[1]))",
                    str(output),
                ]
            )
            self.assertIn(b"\x1b[", expected[2])
            self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
