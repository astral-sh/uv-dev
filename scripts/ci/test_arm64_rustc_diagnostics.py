"""Exercise the compiler observer with real exit and signal statuses."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("arm64-rustc-diagnostics.py")
WRAPPER = SCRIPT.with_name("arm64-rustc-wrapper.sh")
DOCKERFILE = """FROM ubuntu:24.04 AS build
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv
RUN case "${TARGETPLATFORM}" in \\
  "linux/arm64") export JEMALLOC_SYS_WITH_LG_PAGE=16;; \\
  esac && \\
  cargo auditable zigbuild --bin uv --bin uvx --target $(cat rust_target.txt) --release
FROM scratch
COPY --from=build /uv /uvx /
"""


class CompilerDiagnosticsTest(unittest.TestCase):
    def setUp(self):
        temporary_root = Path.home() / "code" / "tmp"
        temporary_root.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="uv-rustc-diagnostic-test-", dir=temporary_root
        )
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = dict(os.environ) | {
            "UV_DIAGNOSTIC_REAL_RUSTC": sys.executable,
            "UV_DIAGNOSTIC_RESULTS": str(self.root),
            "UV_DIAGNOSTIC_PYTHON": sys.executable,
            "UV_DIAGNOSTIC_UV": shutil.which("uv") or "uv",
            "UV_DIAGNOSTIC_SCRIPT": str(SCRIPT),
            "CARGO_AUDITABLE_ORIG_ARGS": "[]",
        }
        self.environment.pop("CARGO_PRIMARY_PACKAGE", None)

    def observe(self, source, prefix=None):
        command = prefix or [sys.executable, str(SCRIPT), "rustc", "--"]
        return subprocess.run(
            [*command, "-c", source],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def record(self):
        paths = list((self.root / "rustc").glob("*.json"))
        self.assertEqual(len(paths), 1)
        return json.loads(paths[0].read_text())

    def test_preserves_stdout_and_exit_status(self):
        result = self.observe("print('compiler output'); raise SystemExit(42)")
        self.assertEqual(result.returncode, 42)
        self.assertEqual(result.stdout, "compiler output\n")
        record = self.record()
        self.assertEqual(record["returncode"], 42)
        self.assertNotIn("signal", record)
        self.assertGreater(record["max_rss_bytes"], 0)

    def test_snapshot_includes_the_requested_parent_cgroup(self):
        cgroup = self.root / "parent-cgroup"
        cgroup.mkdir()
        (cgroup / "memory.max").write_text("34359738368\n")
        (cgroup / "memory.events").write_text("oom 2\noom_kill 1\n")
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "snapshot"],
            env=self.environment | {"UV_DIAGNOSTIC_CGROUP": str(cgroup)},
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        values = json.loads(result.stdout)["cgroups"][str(cgroup)]
        self.assertEqual(values["memory.max"], "34359738368")
        self.assertEqual(values["memory.events"], "oom 2\noom_kill 1")

    def test_reports_sigkill(self):
        result = self.observe("import os, signal; os.kill(os.getpid(), signal.SIGKILL)")
        self.assertEqual(result.returncode, 137)
        record = self.record()
        self.assertEqual(record["returncode"], -9)
        self.assertEqual(record["signal"], 9)
        self.assertEqual(record["signal_name"], "SIGKILL")
        self.assertIn("RUSTC_DIAGNOSTIC", result.stderr)

    def test_build_failure_keeps_memory_evidence(self):
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "build",
                "--label",
                "test",
                "--",
                sys.executable,
                "-c",
                "raise SystemExit(3)",
            ],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 3)
        self.assertEqual(
            json.loads((self.root / "test-result.json").read_text())["returncode"], 3
        )
        self.assertTrue((self.root / "test-memory.jsonl").read_text())
        self.assertIn("BUILD_START_DIAGNOSTIC", result.stdout)
        self.assertIn("MEMORY_DIAGNOSTIC", result.stdout)
        self.assertIn("COMPILER_REPORT", result.stdout)

    def test_reports_uv_compiler_start_before_exit(self):
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "rustc",
                "--",
                "-c",
                "raise SystemExit(42)",
                "--crate-name",
                "uv",
                "--crate-type",
                "bin",
            ],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 42)
        lines = result.stderr.splitlines()
        self.assertTrue(lines[0].startswith("RUSTC_START_DIAGNOSTIC "))
        self.assertTrue(lines[1].startswith("RUSTC_DIAGNOSTIC "))
        started = json.loads(lines[0].split(" ", 1)[1])
        completed = json.loads(lines[1].split(" ", 1)[1])
        self.assertEqual(started["pid"], completed["pid"])
        self.assertLessEqual(started["before"]["time"], completed["after"]["time"])

    def test_docker_attempts_have_distinct_build_inputs(self):
        rewritten = []
        digest = "a" * 64
        for build_id in ("run-1", "run-2"):
            source = self.root / build_id
            source.mkdir()
            (source / "Dockerfile").write_text(DOCKERFILE)
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "dockerfile",
                    str(source),
                    str(SCRIPT.parent),
                    "--uv-digest",
                    digest,
                    "--build-id",
                    build_id,
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (source / ".ci-2124/build-id").read_text(), build_id + "\n"
            )
            contents = (source / "Dockerfile").read_text()
            self.assertIn("COPY .ci-2124 /root/.ci-2124\n", contents)
            self.assertIn(f"ghcr.io/astral-sh/uv@sha256:{digest}", contents)
            self.assertEqual(contents.count("cargo auditable zigbuild"), 1)
            rewritten.append(contents)
        self.assertEqual(rewritten[0], rewritten[1])

    def test_prepares_docker_image_with_a_separate_build_command(self):
        source = self.root / "prepared"
        source.mkdir()
        (source / "Dockerfile").write_text(DOCKERFILE)
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "dockerfile",
                str(source),
                str(SCRIPT.parent),
                "--uv-digest",
                "a" * 64,
                "--prepare-only",
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        contents = (source / "Dockerfile").read_text()
        self.assertIn("FROM ubuntu:24.04 AS build", contents)
        self.assertIn("COPY .ci-2124 /root/.ci-2124", contents)
        self.assertNotIn("FROM scratch", contents)
        self.assertNotIn("cargo auditable zigbuild", contents)
        launcher = source / ".ci-2124/build.sh"
        self.assertTrue(os.access(launcher, os.X_OK))
        self.assertIn("cargo auditable zigbuild", launcher.read_text())
        syntax = subprocess.run(["sh", "-n", launcher], check=False)
        self.assertEqual(syntax.returncode, 0)

    def test_preserves_inherited_jobserver_descriptors(self):
        read_fd, write_fd = os.pipe()
        self.addCleanup(os.close, read_fd)
        self.addCleanup(os.close, write_fd)
        os.write(write_fd, b"token")
        self.environment["CARGO_MAKEFLAGS"] = (
            f"-j --jobserver-fds={read_fd},{write_fd} "
            f"--jobserver-auth={read_fd},{write_fd}"
        )
        command = [str(WRAPPER)]
        if binary := os.environ.get("AUDITABLE_TEST_BINARY"):
            command.insert(0, binary)
        result = subprocess.run(
            [
                *command,
                "-c",
                f"import os; assert os.read({read_fd}, 5) == b'token'",
            ],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
            pass_fds=(read_fd, write_fd),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.record()["returncode"], 0)

    @unittest.skipUnless(
        os.environ.get("AUDITABLE_TEST_BINARY"),
        "set AUDITABLE_TEST_BINARY to test cargo-auditable",
    )
    def test_cargo_auditable_reports_original_signal(self):
        result = self.observe(
            "import os, signal; os.kill(os.getpid(), signal.SIGTERM)",
            [os.environ["AUDITABLE_TEST_BINARY"], str(WRAPPER)],
        )
        self.assertEqual(result.returncode, 143)
        self.assertEqual(self.record()["signal_name"], "SIGTERM")
        self.assertNotIn("deadly signal", result.stderr)


if __name__ == "__main__":
    unittest.main()
