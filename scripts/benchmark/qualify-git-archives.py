"""Qualify concurrent Git-archive builds, cache recovery, and version interoperability."""

from __future__ import annotations

import argparse
import gzip
import importlib.util
import io
import json
import os
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "uv_source_preparation_qualification",
    Path(__file__).with_name("qualify-source-preparation.py"),
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Could not load the source-preparation qualification")
source = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = source
SPEC.loader.exec_module(source)

NAME = "git-archive-fixture"
MODULE = NAME.replace("-", "_")
VERSION = "0.1.0"
ARCHIVE = f"archives/{MODULE}-{VERSION}.tar.gz"
SCENARIOS = (
    "same-ref",
    "distinct-ref",
    "different-settings",
    "metadata-wheel",
    "interrupted",
    "missing-source",
    "mixed-reuse",
    "mixed-concurrent",
)

# The shared fixture supplies valid PEP 517 metadata, wheels, and RECORD hashes.
# Only the observation and deterministic blocking hooks differ here.
BACKEND = r"""
import json
import os
import time
from pathlib import Path

import _fixture_backend as original

SETTINGS = {}

def event(phase, kind):
    value = json.dumps({
        "phase": phase,
        "kind": kind,
        "name": original.NAME,
        "pid": os.getpid(),
        "time_ns": time.monotonic_ns(),
        "process": os.environ["SOURCE_GIT_PROCESS"],
        "source": str(original.ROOT),
        "revision": int((original.ROOT / "revision.txt").read_text()),
        "settings": SETTINGS,
    }) + "\n"
    descriptor = os.open(os.environ["SOURCE_FD_LOG"], os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(descriptor, value.encode())
    finally:
        os.close(descriptor)
    if phase == "start" and kind == os.environ.get("SOURCE_GIT_GATE_KIND"):
        gate = Path(os.environ["SOURCE_GIT_GATE"])
        deadline = time.monotonic() + 120
        while not gate.exists():
            if time.monotonic() >= deadline:
                raise RuntimeError("Git archive fixture gate timed out")
            time.sleep(0.005)

original.event = event

def get_requires_for_build_wheel(config_settings=None):
    return []

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    global SETTINGS
    SETTINGS = config_settings or {}
    event("start", "metadata")
    try:
        return original.prepare_metadata_for_build_wheel(metadata_directory, config_settings)
    finally:
        event("end", "metadata")

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    global SETTINGS
    SETTINGS = config_settings or {}
    return original.build_wheel(wheel_directory, config_settings, metadata_directory)
"""


def environment() -> dict[str, str]:
    clean = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(
            (
                "UV_",
                "SOURCE_FD_",
                "SOURCE_GIT_",
                "GIT_CONFIG_",
                "GIT_AUTHOR_",
                "GIT_COMMITTER_",
            )
        )
        and key
        not in {
            "VIRTUAL_ENV",
            "CONDA_PREFIX",
            "PYTHONPATH",
            "PYTHONHOME",
            "RUST_LOG",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_COMMON_DIR",
        }
    }
    clean.update(
        UV_PYTHON_DOWNLOADS="never",
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_TERMINAL_PROMPT="0",
    )
    return clean


def git(directory: Path, *arguments: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(directory), *arguments],
        env=environment(),
        text=True,
        stderr=subprocess.PIPE,
    ).strip()


def write_archive(path: Path, package: Path) -> None:
    with (
        path.open("wb") as output,
        gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressed,
        tarfile.open(
            fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT
        ) as archive,
    ):
        for member in sorted(package.iterdir()):
            data = member.read_bytes()
            info = tarfile.TarInfo(f"{MODULE}-{VERSION}/{member.name}")
            info.size = len(data)
            info.mode = 0o644
            archive.addfile(info, io.BytesIO(data))


def prepare_fixture(directory: Path) -> dict:
    package = directory / "package"
    source.write_package(package, NAME, [], dynamic_version=True)
    (package / "backend.py").rename(package / "_fixture_backend.py")
    (package / "backend.py").write_text(BACKEND)
    repository = directory / "repository"
    repository.mkdir()
    git(
        repository,
        "-c",
        "init.templateDir=",
        "init",
        "--quiet",
        "--initial-branch=main",
    )
    archive = repository / ARCHIVE
    archive.parent.mkdir()
    revisions = []
    for value in (1001, 1002):
        (package / f"{MODULE}.py").write_text(f"VALUE = {value}\n")
        (package / "revision.txt").write_text(f"{value}\n")
        write_archive(archive, package)
        git(repository, "add", "--", ARCHIVE)
        subprocess.run(
            [
                "git",
                "-C",
                str(repository),
                "-c",
                "user.name=zaniebot",
                "-c",
                "user.email=242828183+zaniebot@users.noreply.github.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                f"Git archive fixture {value}",
            ],
            env={
                **environment(),
                "GIT_AUTHOR_DATE": "2020-01-01T00:00:00Z",
                "GIT_COMMITTER_DATE": "2020-01-01T00:00:00Z",
            },
            check=True,
            capture_output=True,
            text=True,
        )
        commit = git(repository, "rev-parse", "HEAD")
        revisions.append(
            {
                "commit": commit,
                "tree": git(repository, "rev-parse", "HEAD^{tree}"),
                "archive_sha256": source.digest(archive),
                "archive_size": archive.stat().st_size,
                "value": value,
                "url": f"git+{repository.as_uri()}@{commit}#path={ARCHIVE}",
            }
        )
    git(repository, "fsck", "--no-reflogs", "--connectivity-only")
    return {"repository": str(repository), "revisions": revisions}


def binary_record(path: Path) -> dict:
    return {
        "path": str(path),
        "sha256": source.digest(path),
        "version": subprocess.check_output([path, "--version"], text=True).strip(),
    }


@dataclass
class Child:
    label: str
    command: list[str]
    process: subprocess.Popen
    stdout_path: Path
    stderr_path: Path
    target: Path | None
    revision: dict
    settings: dict[str, str]
    started: float
    finished: float | None = None
    timed_out: bool = False
    interrupted: bool = False

    def stderr(self) -> str:
        return self.stderr_path.read_text(errors="replace")

    def stop(self) -> None:
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait()
        if self.finished is None:
            self.finished = time.perf_counter()

    def finish(self, timeout: float) -> dict:
        if self.finished is None:
            try:
                self.process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                self.timed_out = True
                self.stop()
            self.finished = time.perf_counter()
        return {
            "label": self.label,
            "command": self.command,
            "elapsed_seconds": self.finished - self.started,
            "exit_code": self.process.returncode,
            "timed_out": self.timed_out,
            "interrupted": self.interrupted,
            "stdout": self.stdout_path.read_text(errors="replace"),
            "stderr": self.stderr(),
        }


class Qualification:
    def __init__(self, args: argparse.Namespace, directory: Path, fixture: dict):
        self.args = args
        self.directory = directory
        self.directory.mkdir()
        self.cache = directory / "cache"
        self.events_path = directory / "events.jsonl"
        self.release = directory / "release"
        self.fixture = fixture
        self.children: list[Child] = []
        self.details: dict = {}
        self.environment = environment()
        temporary = directory / "tmp"
        temporary.mkdir()
        self.environment.update(
            TMPDIR=str(temporary),
            SOURCE_FD_LOG=str(self.events_path),
            SOURCE_FD_BUILD_DELAY=str(args.build_delay),
            UV_CONCURRENT_BUILDS=str(args.builds),
            RUST_LOG="uv_fs=info",
        )

    def events(
        self, *, process: str | None = None, kind: str | None = None
    ) -> list[dict]:
        return [
            event
            for event in source.read_events(self.events_path)
            if (process is None or event["process"] == process)
            and (kind is None or event["kind"] == kind)
        ]

    def start(
        self,
        binary: Path,
        label: str,
        revision: dict,
        *,
        settings: dict[str, str] | None = None,
        metadata: bool = False,
        offline: bool = False,
        gated: bool = False,
    ) -> Child:
        settings = settings or {}
        directory = self.directory / label
        directory.mkdir()
        requirement = directory / "requirements.in"
        requirement.write_text(f"{NAME} @ {revision['url']}\n")
        command = [
            "sh",
            "-c",
            'ulimit -S -n "$1" && ulimit -H -n "$1" && shift && exec "$@"',
            "sh",
            str(self.args.fd_limit),
            str(binary),
            "--no-config",
            "--no-progress",
            "--cache-dir",
            str(self.cache),
        ]
        if offline:
            command.append("--offline")
        command.extend(
            [
                "pip",
                "compile" if metadata else "install",
                "--no-index",
                "--no-deps",
                "--python",
                str(self.args.python),
            ]
        )
        for key, value in settings.items():
            command.extend(["--config-setting", f"{key}={value}"])
        target = None if metadata else directory / "installed"
        if metadata:
            command.extend(["--generate-hashes", "--no-header", str(requirement)])
        else:
            command.extend(["--target", str(target), "-r", str(requirement)])
        child_environment = {**self.environment, "SOURCE_GIT_PROCESS": label}
        if gated:
            child_environment.update(
                SOURCE_GIT_GATE=str(self.release),
                SOURCE_GIT_GATE_KIND="wheel",
            )
        stdout_path = directory / "stdout"
        stderr_path = directory / "stderr"
        started = time.perf_counter()
        with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
            process = subprocess.Popen(
                command,
                cwd=directory,
                env=child_environment,
                stdout=stdout,
                stderr=stderr,
                start_new_session=True,
            )
        child = Child(
            label,
            command,
            process,
            stdout_path,
            stderr_path,
            target,
            revision,
            settings,
            started,
        )
        self.children.append(child)
        return child

    def wait_started(self, child: Child, kind: str = "wheel") -> dict:
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            for event in self.events(process=child.label, kind=kind):
                if event["phase"] == "start":
                    return event
            if child.process.poll() is not None:
                break
            time.sleep(0.005)
        raise AssertionError(f"{child.label} did not start {kind}: {child.stderr()}")

    def wait_state(self, child: Child, lock: Path, *, kind: str = "wheel") -> str:
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            if any(
                "Waiting to acquire exclusive lock" in line and str(lock) in line
                for line in child.stderr().splitlines()
            ):
                return "source-lock"
            if any(
                event["phase"] == "start"
                for event in self.events(process=child.label, kind=kind)
            ):
                return "backend-started"
            if child.process.poll() is not None:
                return "completed"
            time.sleep(0.005)
        raise AssertionError(
            f"{child.label} neither reached the source lock nor started {kind}: "
            f"{child.stderr()}"
        )

    def source_path(self, event: dict) -> Path:
        path = Path(event["source"])
        if (
            path.name != "src"
            or not path.is_relative_to(self.cache)
            or not any(part.startswith("sdists-") for part in path.parts)
        ):
            raise AssertionError(f"Unexpected extracted Git archive source: {path}")
        return path

    def finish(self, child: Child, *, success: bool = True) -> dict:
        record = child.finish(self.args.timeout)
        if record["timed_out"] or (success and record["exit_code"] != 0):
            raise AssertionError(
                f"{child.label} did not complete successfully: {record}"
            )
        return record

    def verify(self, child: Child) -> dict:
        if child.target is None:
            raise AssertionError("Cannot inspect an uninstalled metadata-only request")
        result = subprocess.run(
            [
                self.args.python,
                "-I",
                "-c",
                (
                    "import importlib, importlib.metadata as metadata, json, pathlib, sys\n"
                    "target, name, version, value, settings = sys.argv[1:]\n"
                    "sys.path.insert(0, target)\n"
                    "module = importlib.import_module(name.replace('-', '_'))\n"
                    "assert module.VALUE == int(value), module.VALUE\n"
                    "info = pathlib.Path(target) / (name.replace('-', '_') + '-' + version + '.dist-info')\n"
                    "distribution = metadata.Distribution.at(info)\n"
                    "assert distribution.metadata['Name'] == name\n"
                    "assert distribution.version == version\n"
                    "build = json.loads((info / 'uv_build.json').read_text()) if (info / 'uv_build.json').exists() else {}\n"
                    "assert build.get('config_settings', {}) == json.loads(settings), build\n"
                    "origin = json.loads((info / 'direct_url.json').read_text())\n"
                    "print(json.dumps({'value': module.VALUE, 'build': build, 'direct_url': origin}))"
                ),
                str(child.target),
                NAME,
                VERSION,
                str(child.revision["value"]),
                json.dumps(child.settings),
            ],
            env=self.environment,
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(result.stdout)

    def warm(self, binary: Path, revision: dict, *, label: str = "warm") -> dict:
        before = len(self.events())
        child = self.start(binary, label, revision, offline=True)
        self.finish(child)
        installed = self.verify(child)
        if events := self.events()[before:]:
            raise AssertionError(f"A warm offline installation ran a backend: {events}")
        return installed

    def close(self) -> dict:
        for child in self.children:
            if child.process.poll() is None:
                child.stop()
        events = self.events()
        wheels = [event for event in events if event["kind"] == "wheel"]
        return {
            **self.details,
            "commands": [child.finish(self.args.timeout) for child in self.children],
            "backend_events": events,
            "wheel_builds": source.summarize_events(wheels),
            "metadata_calls": source.summarize_events(
                [event for event in events if event["kind"] == "metadata"]
            ),
        }


def same_ref(qualification: Qualification, binary: Path, policy: str) -> None:
    revision = qualification.fixture["revisions"][0]
    leader = qualification.start(binary, "leader", revision, gated=True)
    event = qualification.wait_started(leader)
    lock = qualification.source_path(event).parent / ".lock"
    followers = [
        qualification.start(binary, f"follower-{number}", revision, gated=True)
        for number in range(qualification.args.processes - 1)
    ]
    states = [qualification.wait_state(child, lock) for child in followers]
    qualification.details["before_release"] = {
        "follower_states": states,
        "wheel_builds": source.summarize_events(qualification.events(kind="wheel")),
    }
    qualification.release.touch()
    for child in [leader, *followers]:
        qualification.finish(child)
        qualification.verify(child)
    summary = source.summarize_events(qualification.events(kind="wheel"))
    qualification.details["outcome"] = (
        "serialized" if summary["started_builds"] == 1 else "duplicate-builds"
    )
    qualification.warm(binary, revision)
    if policy != "record" and (
        summary["started_builds"] != 1
        or summary["peak_backend_builds"] != 1
        or states != ["source-lock"] * len(followers)
    ):
        raise AssertionError(
            f"Same-reference builds were not coalesced: {states}, {summary}"
        )


def distinct_ref(qualification: Qualification, binary: Path, _policy: str) -> None:
    first, second = qualification.fixture["revisions"]
    leader = qualification.start(binary, "leader", first, gated=True)
    qualification.wait_started(leader)
    follower = qualification.start(binary, "different-ref", second)
    qualification.wait_started(follower)
    qualification.finish(follower)
    qualification.details["follower_completed_before_release"] = True
    qualification.verify(follower)
    qualification.release.touch()
    qualification.finish(leader)
    qualification.verify(leader)
    summary = source.summarize_events(qualification.events(kind="wheel"))
    if summary["started_builds"] != 2 or summary["peak_backend_builds"] != 2:
        raise AssertionError(
            f"Different Git references did not build independently: {summary}"
        )
    qualification.details["outcome"] = "independent"


def different_settings(qualification: Qualification, binary: Path, policy: str) -> None:
    revision = qualification.fixture["revisions"][0]
    leader = qualification.start(
        binary, "leader", revision, settings={"flavor": "one"}, gated=True
    )
    event = qualification.wait_started(leader)
    lock = qualification.source_path(event).parent / ".lock"
    follower = qualification.start(
        binary, "different-settings", revision, settings={"flavor": "two"}, gated=True
    )
    state = qualification.wait_state(follower, lock)
    qualification.details["before_release"] = {"follower_state": state}
    qualification.release.touch()
    for child in (leader, follower):
        qualification.finish(child)
        qualification.verify(child)
    summary = source.summarize_events(qualification.events(kind="wheel"))
    if summary["started_builds"] != 2:
        raise AssertionError(
            f"Distinct build settings reused an incompatible wheel: {summary}"
        )
    qualification.details["outcome"] = (
        "serialized" if summary["peak_backend_builds"] == 1 else "concurrent"
    )
    if policy != "record" and (
        state != "source-lock" or summary["peak_backend_builds"] != 1
    ):
        raise AssertionError(
            f"Shared Git sources were built concurrently: {state}, {summary}"
        )


def metadata_wheel(qualification: Qualification, binary: Path, policy: str) -> None:
    revision = qualification.fixture["revisions"][0]
    leader = qualification.start(binary, "leader", revision, gated=True)
    event = qualification.wait_started(leader)
    lock = qualification.source_path(event).parent / ".lock"
    follower = qualification.start(binary, "metadata", revision, metadata=True)
    state = qualification.wait_state(follower, lock, kind="metadata")
    qualification.details["before_release"] = {"metadata_state": state}
    qualification.release.touch()
    qualification.finish(leader)
    qualification.verify(leader)
    result = qualification.finish(follower)
    if revision["commit"] not in result["stdout"]:
        raise AssertionError(
            "The metadata result did not retain the exact Git revision"
        )
    qualification.details["outcome"] = (
        "waited-for-wheel" if state == "source-lock" else "read-while-building"
    )
    if policy != "record" and state != "source-lock":
        raise AssertionError(
            f"Metadata did not coordinate with the wheel builder: {state}"
        )


def interrupted(qualification: Qualification, binary: Path, policy: str) -> None:
    revision = qualification.fixture["revisions"][0]
    leader = qualification.start(binary, "leader", revision, gated=True)
    event = qualification.wait_started(leader)
    lock = qualification.source_path(event).parent / ".lock"
    follower = qualification.start(binary, "follower", revision)
    state = qualification.wait_state(follower, lock)
    qualification.details["before_interrupt"] = {"follower_state": state}
    leader.interrupted = True
    leader.stop()
    result = qualification.finish(leader, success=False)
    if result["exit_code"] == 0:
        raise AssertionError("The active wheel builder was not interrupted")
    qualification.finish(follower)
    qualification.verify(follower)
    qualification.warm(binary, revision)
    qualification.details["outcome"] = "recovered"
    if policy != "record" and state != "source-lock":
        raise AssertionError(
            f"The follower was not waiting for the interrupted leader: {state}"
        )


def missing_source(qualification: Qualification, binary: Path, policy: str) -> None:
    revision = qualification.fixture["revisions"][0]
    seed = qualification.start(binary, "metadata-seed", revision, metadata=True)
    seed_result = qualification.finish(seed)
    expected_hash = f"--hash=sha256:{revision['archive_sha256']}"
    if expected_hash not in seed_result["stdout"]:
        raise AssertionError("The seed did not compute the actual Git archive hash")
    metadata = [
        event
        for event in qualification.events(process=seed.label, kind="metadata")
        if event["phase"] == "start"
    ]
    if len(metadata) != 1 or qualification.events(kind="wheel"):
        raise AssertionError(
            f"The seed did not prepare only metadata: {qualification.events()}"
        )
    path = qualification.source_path(metadata[0])
    hashes = path.parent / "hashes.msgpack"
    before = source.digest(hashes)
    path.rename(qualification.directory / "displaced-source")
    if not hashes.is_file():
        raise AssertionError("Displacing the source also removed its revision hashes")
    child = qualification.start(binary, "install", revision, offline=True)
    result = qualification.finish(child, success=False)
    recovered = result["exit_code"] == 0
    if recovered:
        qualification.verify(child)
        if not path.is_dir():
            raise AssertionError(
                "Source recovery did not restore the extracted archive"
            )
        qualification.details["hashes_after_recovery_sha256"] = source.digest(hashes)
        check = qualification.start(
            binary, "hash-check", revision, metadata=True, offline=True
        )
        check_result = qualification.finish(check)
        if (
            expected_hash not in check_result["stdout"]
            or source.digest(hashes) != before
        ):
            raise AssertionError(
                "The recovered archive did not retain its expected hash"
            )
        qualification.warm(binary, revision)
    elif str(path) not in result["stderr"] or not any(
        reason in result["stderr"]
        for reason in (
            "No such file or directory",
            "does not appear to be a Python project",
        )
    ):
        raise AssertionError(f"The missing-source failure was not identified: {result}")
    qualification.details.update(
        outcome="recovered" if recovered else "missing-source",
        retained_hashes_sha256=before,
        source_restored=path.is_dir(),
    )
    if policy == "recovery" and not recovered:
        raise AssertionError(
            f"The retained Git revision did not recover its source: {result}"
        )


def mixed_reuse(qualification: Qualification, base: Path, candidate: Path) -> None:
    revision = qualification.fixture["revisions"][0]
    seed = qualification.start(base, "seed", revision)
    qualification.finish(seed)
    qualification.verify(seed)
    qualification.details["reader"] = qualification.warm(
        candidate, revision, label="reader"
    )
    qualification.details["outcome"] = "reused"


def mixed_concurrent(qualification: Qualification, base: Path, candidate: Path) -> None:
    revision = qualification.fixture["revisions"][0]
    leader = qualification.start(base, "leader", revision, gated=True)
    event = qualification.wait_started(leader)
    lock = qualification.source_path(event).parent / ".lock"
    follower = qualification.start(candidate, "follower", revision, gated=True)
    state = qualification.wait_state(follower, lock)
    qualification.details["before_release"] = {"follower_state": state}
    qualification.release.touch()
    for child in (leader, follower):
        qualification.finish(child)
        qualification.verify(child)
    summary = source.summarize_events(qualification.events(kind="wheel"))
    if summary["started_builds"] not in {1, 2}:
        raise AssertionError(f"Unexpected mixed-version build count: {summary}")
    qualification.details["outcome"] = (
        "serialized" if summary["started_builds"] == 1 else "duplicate-builds"
    )
    qualification.warm(candidate, revision)


def run_case(
    args: argparse.Namespace,
    directory: Path,
    fixture: dict,
    name: str,
    callback,
    *participants,
) -> dict:
    qualification = Qualification(args, directory / name, fixture)
    success = False
    error = None
    try:
        callback(qualification, *participants)
        success = True
    except (
        AssertionError,
        OSError,
        RuntimeError,
        subprocess.SubprocessError,
        ValueError,
    ) as exception:
        error = {"type": type(exception).__name__, "message": str(exception)}
    finally:
        details = qualification.close()
    result = {"name": name, "success": success, **details}
    if error is not None:
        result["error"] = error
    return result


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument(
        "--work-directory", type=Path, default=root / ".cache/git-qualification"
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--scenario", choices=("all", *SCENARIOS), default="all")
    parser.add_argument("--processes", type=int, default=4)
    parser.add_argument("--builds", type=int, default=4)
    parser.add_argument("--fd-limit", type=int, default=128)
    parser.add_argument("--build-delay", type=float, default=0.05)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument(
        "--base-policy", choices=("record", "serialized", "recovery"), default="record"
    )
    parser.add_argument(
        "--candidate-policy",
        choices=("record", "serialized", "recovery"),
        default="serialized",
    )
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("this process-group interruption harness requires a POSIX system")
    if args.processes < 2 or args.builds < 1 or args.fd_limit < 32:
        parser.error(
            "at least two processes, one build, and 32 descriptors are required"
        )
    if args.timeout <= 0 or args.build_delay < 0:
        parser.error("timeout must be positive and build delay must be nonnegative")
    args.base = args.base.resolve(strict=True)
    args.candidate = args.candidate.resolve(strict=True)
    args.python = args.python.resolve(strict=True)
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    cases = []
    report = {
        "success": False,
        "platform": sys.platform,
        "harness_sha256": source.digest(Path(__file__)),
        "source_harness_sha256": source.digest(Path(SPEC.origin)),
        "base": binary_record(args.base),
        "candidate": binary_record(args.candidate),
        "python": str(args.python),
        "python_version": subprocess.check_output(
            [args.python, "-I", "--version"], text=True
        ).strip(),
        "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
        "scenario": args.scenario,
        "processes": args.processes,
        "concurrent_builds": args.builds,
        "fd_limit": args.fd_limit,
        "base_policy": args.base_policy,
        "candidate_policy": args.candidate_policy,
        "cases": cases,
    }
    try:
        with tempfile.TemporaryDirectory(
            dir=args.work_directory, prefix="git-archives-"
        ) as temporary:
            directory = Path(temporary)
            report["fixture"] = fixture = prepare_fixture(directory)
            individual = {
                "same-ref": same_ref,
                "distinct-ref": distinct_ref,
                "different-settings": different_settings,
                "metadata-wheel": metadata_wheel,
                "interrupted": interrupted,
                "missing-source": missing_source,
            }
            for scenario, callback in individual.items():
                if args.scenario not in {"all", scenario}:
                    continue
                for label, binary, policy in (
                    ("base", args.base, args.base_policy),
                    ("candidate", args.candidate, args.candidate_policy),
                ):
                    cases.append(
                        run_case(
                            args,
                            directory,
                            fixture,
                            f"{scenario}-{label}",
                            callback,
                            binary,
                            policy,
                        )
                    )
            for scenario, callback in (
                ("mixed-reuse", mixed_reuse),
                ("mixed-concurrent", mixed_concurrent),
            ):
                if args.scenario not in {"all", scenario}:
                    continue
                for label, first, second in (
                    ("base-to-candidate", args.base, args.candidate),
                    ("candidate-to-base", args.candidate, args.base),
                ):
                    cases.append(
                        run_case(
                            args,
                            directory,
                            fixture,
                            f"{scenario}-{label}",
                            callback,
                            first,
                            second,
                        )
                    )
        report["success"] = all(case["success"] for case in cases)
    except Exception as exception:
        report["error"] = {"type": type(exception).__name__, "message": str(exception)}
        raise
    finally:
        output = json.dumps(report, indent=2) + "\n"
        if args.output is None:
            print(output, end="")
        else:
            args.output.write_text(output)
    if not report["success"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
