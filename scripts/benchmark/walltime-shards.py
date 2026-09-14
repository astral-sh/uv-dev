"""Partition all built walltime suites into bounded, independently uploaded runs."""

from __future__ import annotations

import argparse
import errno
import hashlib
import json
import math
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid
from collections.abc import Callable, Mapping
from enum import Enum
from pathlib import Path
from types import MappingProxyType
from typing import Any, BinaryIO

MAX_SHARDS = 8
PLAN_VERSION = 2
RUN_VERSION = 1
TERMINATION_GRACE_SECONDS = 5


def stream_digest(stream: BinaryIO) -> str:
    hasher = hashlib.sha256()
    while chunk := stream.read(1024 * 1024):
        hasher.update(chunk)
    return hasher.hexdigest()


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return stream_digest(stream)


def output(
    root: Path, *command: str, environment: Mapping[str, str] | None = None
) -> str:
    return subprocess.check_output(
        command, cwd=root, env=environment, text=True
    ).strip()


def cargo_codspeed_version(
    root: Path,
    *,
    cargo: str = "cargo",
    environment: Mapping[str, str] | None = None,
) -> str:
    result = subprocess.run(
        [cargo, "codspeed", "--version"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    # cargo-codspeed 5.0.1 sends Clap's display-version result through its error
    # handler. Other nonzero exits are not version responses.
    if (
        result.returncode == 1
        and result.stdout == ""
        and result.stderr.rstrip("\r\n") == "cargo-codspeed 5.0.1"
    ):
        return "cargo-codspeed 5.0.1"
    result.check_returncode()
    version = result.stdout.strip()
    if (
        result.stderr
        or re.fullmatch(
            r"cargo-codspeed [0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?",
            version,
        )
        is None
    ):
        raise ValueError("Unexpected cargo-codspeed version response")
    return version


def source_identity(
    root: Path, *, environment: Mapping[str, str] | None = None
) -> dict:
    """Identify tracked source without treating generated fixture files as source edits."""
    commit = output(root, "git", "rev-parse", "HEAD", environment=environment)
    difference = subprocess.check_output(
        ["git", "diff", "--binary", "--no-ext-diff", "--no-textconv", commit, "--"],
        cwd=root,
        env=environment,
    )
    identity = {
        "commit": commit,
        "tree": output(
            root, "git", "rev-parse", f"{commit}^{{tree}}", environment=environment
        ),
        "tracked_working_tree_dirty": bool(difference),
        "tracked_diff_sha256": hashlib.sha256(difference).hexdigest(),
        "cargo_lock_sha256": digest(root / "Cargo.lock"),
        "rust_toolchain_sha256": digest(root / "rust-toolchain.toml"),
    }
    if output(root, "git", "rev-parse", "HEAD", environment=environment) != commit:
        raise ValueError("The source checkout changed while its identity was recorded")
    return identity


def producer_metadata(
    root: Path, *, cargo: str = "cargo", environment: Mapping[str, str] | None = None
) -> dict:
    environment = os.environ if environment is None else environment
    return {
        "rustc": output(root, "rustc", "-Vv", environment=environment),
        "cargo_codspeed": cargo_codspeed_version(
            root, cargo=cargo, environment=environment
        ),
        "working_tree_status": output(
            root,
            "git",
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            environment=environment,
        ).splitlines(),
        "github": {
            key: environment[key]
            for key in (
                "GITHUB_REPOSITORY",
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_JOB",
                "GITHUB_WORKFLOW_REF",
                "GITHUB_WORKFLOW_SHA",
            )
            if key in environment
        },
    }


def invocation_executable(
    executable: str, environment: Mapping[str, str]
) -> Path | None:
    """Return the absolute invocation path without changing a multicall basename."""
    if discovered := shutil.which(executable, path=environment.get("PATH", os.defpath)):
        path = Path(discovered)
        return path if path.is_absolute() else Path.cwd() / path
    return None


def executable_identity(executable: str, environment: Mapping[str, str]) -> dict | None:
    if path := invocation_executable(executable, environment):
        resolved = path.resolve(strict=True)
        return {
            "invocation_executable": str(path),
            "resolved_executable": str(resolved),
            "sha256": digest(resolved),
        }
    return None


def matches_executable_identity(
    executable: str, identity: dict, environment: Mapping[str, str]
) -> bool:
    observed = executable_identity(executable, environment)
    return observed is not None and all(
        observed[key] == identity[key]
        for key in ("invocation_executable", "resolved_executable", "sha256")
    )


def tool_identity(
    root: Path,
    executable: str,
    *arguments: str,
    environment: Mapping[str, str] | None = None,
    invoked_tool: tuple[str, dict] | None = None,
    version_probe: Callable[[str], str] | None = None,
) -> dict | None:
    """Record invocation and target identity separately from the reported version."""
    environment = os.environ if environment is None else environment
    if identity := executable_identity(executable, environment):
        invoked_name, invoked_identity = invoked_tool or (executable, identity)
        if not matches_executable_identity(invoked_name, invoked_identity, environment):
            raise ValueError(
                "The walltime tool changed while its identity was recorded"
            )
        invocation = invoked_identity["invocation_executable"]
        command = (invocation, *arguments)
        version = (
            version_probe(invocation)
            if version_probe is not None
            else subprocess.check_output(
                command, cwd=root, env=environment, text=True, timeout=15
            ).strip()
        )
        if not matches_executable_identity(executable, identity, environment) or (
            invoked_tool is not None
            and not matches_executable_identity(
                invoked_name, invoked_identity, environment
            )
        ):
            raise ValueError(
                "The walltime tool changed while its identity was recorded"
            )
        return {
            **identity,
            "version_command": list(command),
            "version": version,
        }
    return None


def consumer_metadata(root: Path, environment: Mapping[str, str]) -> dict:
    cargo = tool_identity(root, "cargo", "--version", environment=environment)
    if cargo is None:
        raise ValueError("Could not identify the cargo executable")
    cargo_codspeed = tool_identity(
        root,
        "cargo-codspeed",
        "codspeed",
        "--version",
        environment=environment,
        invoked_tool=("cargo", cargo),
        version_probe=lambda invocation: cargo_codspeed_version(
            root, cargo=invocation, environment=environment
        ),
    )
    if cargo_codspeed is None:
        raise ValueError("Could not identify the cargo and cargo-codspeed executables")
    codspeed = tool_identity(root, "codspeed", "--version", environment=environment)
    inside_runner = environment.get("CODSPEED_ENV") == "runner"
    if inside_runner and codspeed is None:
        raise ValueError("Could not identify the enclosing CodSpeed runner")
    return {
        "operating_system": platform.platform(),
        "architecture": platform.machine(),
        "cargo": cargo,
        "cargo_codspeed": cargo_codspeed,
        # A local invocation need not be inside the CodSpeed runner. In CI, this
        # is populated after the enclosing action has installed the runner.
        "codspeed": codspeed,
        "inside_codspeed_runner": inside_runner,
        "codspeed_runner_mode": environment.get("CODSPEED_RUNNER_MODE"),
        "github": {
            key: environment[key]
            for key in (
                "GITHUB_REPOSITORY",
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_JOB",
                "GITHUB_WORKFLOW_REF",
                "GITHUB_WORKFLOW_SHA",
            )
            if key in environment
        },
    }


def partition(names: list[str]) -> list[dict]:
    if not names or len(names) != len(set(names)):
        raise ValueError("Expected a non-empty set of unique benchmark suites")
    if any(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]*", name) is None for name in names):
        raise ValueError("Unexpected benchmark suite name")
    names = sorted(names)
    count = min(MAX_SHARDS, len(names))
    return [
        {"index": index + 1, "total": count, "benches": names[index::count]}
        for index in range(count)
    ]


def artifact_paths(directory: Path) -> dict[str, Path]:
    return {path.name: path for path in sorted(directory.iterdir()) if path.is_file()}


def built_artifacts(
    root: Path, *, cargo: str = "cargo", environment: Mapping[str, str] | None = None
) -> dict[str, Path]:
    metadata = json.loads(
        subprocess.check_output(
            [
                cargo,
                "metadata",
                "--locked",
                "--no-deps",
                "--format-version",
                "1",
            ],
            cwd=root,
            env=environment,
            text=True,
        )
    )
    package = next(item for item in metadata["packages"] if item["name"] == "uv-bench")
    declared = {
        target["name"] for target in package["targets"] if "bench" in target["kind"]
    }
    directory = Path(metadata["target_directory"]) / "codspeed/walltime/uv-bench"
    artifacts = artifact_paths(directory)
    if unknown := set(artifacts) - declared:
        raise ValueError(f"Unexpected walltime build artifacts: {sorted(unknown)}")
    return artifacts


def artifact_identity(path: Path) -> dict:
    with path.open("rb") as stream:
        return {
            "sha256": stream_digest(stream),
            "size": os.fstat(stream.fileno()).st_size,
        }


def prepare_plan(source: dict, producer: dict, artifacts: dict[str, Path]) -> dict:
    return {
        "version": PLAN_VERSION,
        "source": source,
        "producer": producer,
        "artifacts": {
            name: artifact_identity(path) for name, path in sorted(artifacts.items())
        },
        "shards": partition(list(artifacts)),
    }


def verify_plan(
    plan: dict,
    source: dict,
    artifacts: dict[str, Path],
    shard: int,
    *,
    bench: str | None = None,
) -> dict:
    expected = partition(list(artifacts))
    if plan.get("version") != PLAN_VERSION or plan.get("shards") != expected:
        raise ValueError("The walltime shard plan does not match the built suites")
    if plan.get("source") != source:
        raise ValueError("The walltime shard plan belongs to different tracked source")
    if set(plan.get("artifacts", {})) != set(artifacts):
        raise ValueError("The walltime shard plan has a different artifact inventory")
    if not 1 <= shard <= len(expected):
        raise ValueError("Shard index is outside the prepared plan")
    selected = expected[shard - 1]
    if bench is not None and bench not in selected["benches"]:
        raise ValueError("The benchmark does not belong to the selected shard")
    # Other shards verify their own binaries; hashing the whole build in every job
    # would add unrelated filesystem work before each measurement.
    for name in selected["benches"] if bench is None else [bench]:
        if plan["artifacts"][name] != artifact_identity(artifacts[name]):
            raise ValueError(
                f"Walltime benchmark artifact does not match its plan: {name}"
            )
    return selected


def timestamp() -> str:
    seconds, nanoseconds = divmod(time.time_ns(), 1_000_000_000)
    return (
        time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(seconds))
        + f".{nanoseconds // 1_000_000:03d}Z"
    )


def write_run_state(path: Path, state: dict) -> None:
    """Replace the last complete record without exposing a partially written JSON file."""
    partial = path.with_name(path.name + ".partial")
    with partial.open("w", encoding="utf-8") as stream:
        json.dump(state, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    partial.replace(path)


def suite_command(name: str, *, cargo: str) -> list[str]:
    if not Path(cargo).is_absolute():
        raise ValueError("Expected the observed absolute cargo executable")
    return [
        cargo,
        "codspeed",
        "run",
        "-m",
        "walltime",
        "-p",
        "uv-bench",
        "--bench",
        name,
    ]


def suite_commands(plan: dict, selected: dict, *, cargo: str) -> list[dict]:
    return [
        {
            "name": name,
            "command": suite_command(name, cargo=cargo),
            "artifact": plan["artifacts"][name],
        }
        for name in selected["benches"]
    ]


def verify_suite_launch(
    root: Path,
    plan: dict,
    selected: dict,
    consumer: dict,
    environment: Mapping[str, str],
    artifact_directory: Path,
    suite: dict,
) -> None:
    """Recheck the observed source and executable inputs immediately before a suite starts."""
    for name, key in (
        ("cargo", "cargo"),
        ("cargo-codspeed", "cargo_codspeed"),
        ("codspeed", "codspeed"),
    ):
        if (identity := consumer[key]) and not matches_executable_identity(
            name, identity, environment
        ):
            raise ValueError(f"The observed walltime tool changed: {name}")
    verify_plan(
        plan,
        source_identity(root, environment=environment),
        artifact_paths(artifact_directory),
        selected["index"],
        bench=suite["name"],
    )


class RunInterruptedError(Exception):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"Walltime run interrupted by signal {signum}")


class RunSignals:
    """Keep the first cancellation authoritative without interrupting bounded cleanup."""

    def __init__(self):
        self.signum: int | None = None
        self.interrupt_wait = False
        self.previous = {}

    def handle(self, signum: int, _frame) -> None:
        if self.signum is not None:
            return
        self.signum = signum
        if self.interrupt_wait:
            raise RunInterruptedError(signum)

    def __enter__(self):
        self.previous = {
            signum: signal.signal(signum, self.handle)
            for signum in (signal.SIGINT, signal.SIGTERM)
        }
        return self

    def __exit__(self, *_exc) -> None:
        for signum, handler in self.previous.items():
            signal.signal(signum, handler)

    def check(self) -> None:
        if self.signum is not None:
            raise RunInterruptedError(self.signum)

    def wait(self, process: subprocess.Popen, timeout: float | None) -> int:
        self.interrupt_wait = True
        try:
            self.check()
            return process.wait(timeout=timeout)
        finally:
            self.interrupt_wait = False


class ShardDeadlineError(Exception):
    pass


class GroupState(Enum):
    PRESENT = "present"
    ABSENT = "absent"
    UNKNOWN = "unknown"


def record_cleanup_event(events: list[dict[str, Any]], **event: Any) -> None:
    observed_at = timestamp()
    if events and events[-1]["event"] == event:
        events[-1]["count"] += 1
        events[-1]["last_observed_at"] = observed_at
    else:
        events.append(
            {
                "event": event,
                "count": 1,
                "first_observed_at": observed_at,
                "last_observed_at": observed_at,
            }
        )


def probe_process_group(
    process_group: int, events: list[dict[str, Any]], stage: str
) -> GroupState:
    try:
        os.killpg(process_group, 0)
    except OSError as error:
        result = (
            "absent"
            if error.errno == errno.ESRCH
            else "denied"
            if error.errno == errno.EPERM
            else "error"
        )
        record_cleanup_event(
            events,
            stage=stage,
            syscall="killpg",
            process_group=process_group,
            signal=0,
            result=result,
            errno=error.errno,
        )
        if error.errno == errno.ESRCH:
            return GroupState.ABSENT
        if error.errno == errno.EPERM:
            return GroupState.UNKNOWN
        raise
    record_cleanup_event(
        events,
        stage=stage,
        syscall="killpg",
        process_group=process_group,
        signal=0,
        result="present",
    )
    return GroupState.PRESENT


def owned_process_group(
    process: subprocess.Popen, process_group: int | None
) -> int | None:
    if os.name != "posix":
        return None
    process_group = process.pid if process_group is None else process_group
    if process_group != process.pid or process_group <= 0:
        raise ValueError("Expected the walltime child's recorded process group")
    if process_group == os.getpgrp():
        raise ValueError("Refusing to signal the walltime driver's process group")
    return process_group


def wait_for_termination(
    process: subprocess.Popen,
    grace_seconds: float,
    *,
    process_group: int | None = None,
    events: list[dict[str, Any]] | None = None,
    stage: str = "wait",
) -> tuple[int | None, bool]:
    process_group = owned_process_group(process, process_group)
    events = [] if events is None else events
    expires = time.monotonic() + grace_seconds
    while True:
        returncode = process.poll()
        if returncode is not None:
            if process_group is None:
                record_cleanup_event(
                    events,
                    stage=stage,
                    syscall="Popen.poll",
                    pid=process.pid,
                    result="reaped",
                    returncode=returncode,
                )
                return returncode, True
            if probe_process_group(process_group, events, stage) is GroupState.ABSENT:
                return returncode, True
        remaining = expires - time.monotonic()
        if remaining <= 0:
            return returncode, False
        time.sleep(min(remaining, 0.05))


def send_termination(
    process: subprocess.Popen,
    process_group: int | None,
    events: list[dict[str, Any]],
    *,
    force: bool,
) -> None:
    stage = "kill" if force else "term"
    event: dict[str, Any]
    if process_group is not None:
        signum = signal.SIGKILL if force else signal.SIGTERM
        event = {
            "stage": stage,
            "syscall": "killpg",
            "process_group": process_group,
            "signal": int(signum),
        }
    else:
        event = {
            "stage": stage,
            "syscall": "Popen.kill" if force else "Popen.terminate",
            "pid": process.pid,
        }
    try:
        if process_group is not None:
            os.killpg(process_group, signal.SIGKILL if force else signal.SIGTERM)
        elif force:
            process.kill()
        else:
            process.terminate()
    except ProcessLookupError as error:
        record_cleanup_event(events, **event, result="absent", errno=error.errno)
    except PermissionError as error:
        record_cleanup_event(events, **event, result="denied", errno=error.errno)
    except OSError as error:
        record_cleanup_event(events, **event, result="error", errno=error.errno)
        raise
    else:
        record_cleanup_event(events, **event, result="sent")


def stop_process(
    process: subprocess.Popen,
    grace_seconds: float,
    *,
    process_group: int | None = None,
    events: list[dict[str, Any]] | None = None,
) -> tuple[int | None, bool]:
    """Bound cleanup of the owned process group, including a leader's surviving children."""
    process_group = owned_process_group(process, process_group)
    events = [] if events is None else events
    if (returncode := process.poll()) is not None:
        if process_group is None:
            record_cleanup_event(
                events,
                stage="initial",
                syscall="Popen.poll",
                pid=process.pid,
                result="reaped",
                returncode=returncode,
            )
            return returncode, True
        if probe_process_group(process_group, events, "initial") is GroupState.ABSENT:
            return returncode, True
    if process_group is not None:
        try:
            current_group = os.getpgid(process.pid)
        except ProcessLookupError as error:
            record_cleanup_event(
                events,
                stage="ownership",
                syscall="getpgid",
                pid=process.pid,
                result="leader-absent",
                errno=error.errno,
            )
        except OSError as error:
            record_cleanup_event(
                events,
                stage="ownership",
                syscall="getpgid",
                pid=process.pid,
                result="error",
                errno=error.errno,
            )
            raise
        else:
            record_cleanup_event(
                events,
                stage="ownership",
                syscall="getpgid",
                pid=process.pid,
                process_group=current_group,
                result="observed",
            )
            if current_group != process_group:
                raise ValueError(
                    "Expected the walltime child to lead its process group"
                )
    send_termination(process, process_group, events, force=False)
    returncode, complete = wait_for_termination(
        process,
        grace_seconds,
        process_group=process_group,
        events=events,
        stage="after-term",
    )
    if complete:
        return returncode, True
    send_termination(process, process_group, events, force=True)
    return wait_for_termination(
        process,
        grace_seconds,
        process_group=process_group,
        events=events,
        stage="after-kill",
    )


def failure_details(error: Exception, phase: str) -> dict:
    details: dict[str, Any] = {"kind": type(error).__name__, "phase": phase}
    if isinstance(error, OSError):
        details["errno"] = error.errno
    elif isinstance(error, subprocess.CalledProcessError):
        details["returncode"] = error.returncode
    elif isinstance(error, ValueError):
        # Launch verification only raises fixed messages and validated suite names.
        details["message"] = str(error)
    return details


def run_suites(
    root: Path,
    directory: Path,
    metadata: dict,
    suites: list[dict],
    *,
    timeout_seconds: float | None = None,
    deadline: float | None = None,
    grace_seconds: float = TERMINATION_GRACE_SECONDS,
    environment: Mapping[str, str] | None = None,
    before_launch: Callable[[dict], None] | None = None,
) -> int:
    """Run the prepared commands, retaining each observed state before continuing."""
    partition([suite["name"] for suite in suites])
    if timeout_seconds is not None and deadline is not None:
        raise ValueError("Use one walltime deadline")
    if any(
        value is not None and (not math.isfinite(value) or value <= 0)
        for value in (timeout_seconds, deadline, grace_seconds)
    ):
        raise ValueError("Expected a finite, positive walltime limit")
    if timeout_seconds is not None:
        deadline = time.time() + timeout_seconds
    environment = dict(os.environ if environment is None else environment)
    expires = (
        time.monotonic() + max(0, deadline - time.time())
        if deadline is not None
        else None
    )
    directory.mkdir(parents=True, exist_ok=False)
    path = directory / "result.json"
    suite_states: list[dict[str, Any]] = [
        {**suite, "status": "pending"} for suite in suites
    ]
    state: dict[str, Any] = {
        **metadata,
        "version": RUN_VERSION,
        "status": "running",
        "started_at": timestamp(),
        "finished_at": None,
        "deadline": deadline,
        "suites": suite_states,
    }
    with RunSignals() as interruptions:
        write_run_state(path, state)
        print(f"Walltime run state: {path}", flush=True)
        exit_code = 0
        for suite in suite_states:
            process = None
            started = None
            phase = "verification"
            try:
                interruptions.check()
                if expires is not None and time.monotonic() >= expires:
                    raise ShardDeadlineError
                suite.update(status="verifying", verification_started_at=timestamp())
                write_run_state(path, state)
                if before_launch is not None:
                    before_launch(suite)
                suite["verified_at"] = timestamp()
                interruptions.check()
                if expires is not None and time.monotonic() >= expires:
                    raise ShardDeadlineError
                phase = "launch"
                started = time.monotonic()
                suite.update(status="starting", launch_started_at=timestamp())
                write_run_state(path, state)
                print(f"Starting walltime suite {suite['name']}", flush=True)
                process = subprocess.Popen(
                    suite["command"],
                    cwd=root,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    start_new_session=os.name == "posix",
                )
                suite.update(status="running", pid=process.pid, started_at=timestamp())
                if os.name == "posix":
                    suite["process_group"] = process.pid
                write_run_state(path, state)
                phase = "execution"
                remaining = expires - time.monotonic() if expires is not None else None
                try:
                    returncode = interruptions.wait(
                        process,
                        max(0, remaining) if remaining is not None else None,
                    )
                except subprocess.TimeoutExpired as error:
                    raise ShardDeadlineError from error
                suite.update(
                    status="success" if returncode == 0 else "failed",
                    returncode=returncode,
                    command_finished_at=timestamp(),
                )
                if returncode:
                    state["status"] = "failed"
                    exit_code = returncode if returncode > 0 else 128 - returncode
            except ShardDeadlineError:
                if process is None:
                    suite["status"] = "pending"
                    state["timeout_before_suite"] = suite["name"]
                else:
                    suite["status"] = "timed_out"
                state["status"] = "timed_out"
                exit_code = 124
            except RunInterruptedError as error:
                if process is None:
                    suite["status"] = "pending"
                    state["interrupted_before_suite"] = suite["name"]
                else:
                    suite.update(status="interrupted", signal=error.signum)
                state.update(status="interrupted", signal=error.signum)
                exit_code = 128 + error.signum
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                if interruptions.signum is not None:
                    suite.update(status="interrupted", signal=interruptions.signum)
                    state.update(status="interrupted", signal=interruptions.signum)
                    exit_code = 128 + interruptions.signum
                else:
                    suite.update(status="failed", error=failure_details(error, phase))
                    state["status"] = "failed"
                    exit_code = 1
            finally:
                # Signals outside the command wait are recorded instead of raised. This
                # also covers cancellation while launching or reaping a completed leader.
                interruptions.interrupt_wait = False
                if process is not None:
                    cleanup_events: list[dict[str, Any]] = []
                    suite.update(
                        cleanup_status="running",
                        cleanup_started_at=timestamp(),
                        cleanup_events=cleanup_events,
                    )
                    try:
                        write_run_state(path, state)
                    finally:
                        try:
                            returncode, complete = stop_process(
                                process,
                                grace_seconds,
                                process_group=suite.get("process_group"),
                                events=cleanup_events,
                            )
                        except (OSError, ValueError) as error:
                            returncode, complete = process.poll(), False
                            suite["cleanup_error"] = failure_details(error, "cleanup")
                    suite.setdefault("returncode", returncode)
                    suite["termination_complete"] = complete
                    suite.update(
                        cleanup_status="complete" if complete else "incomplete",
                        cleanup_finished_at=timestamp(),
                    )
                    if not complete and exit_code == 0:
                        suite["command_status"] = suite["status"]
                        suite["status"] = "cleanup_failed"
                        state["status"] = "failed"
                        exit_code = 1
            if interruptions.signum is not None:
                state["received_signal"] = interruptions.signum
                if exit_code == 0:
                    state.update(status="interrupted", signal=interruptions.signum)
                    exit_code = 128 + interruptions.signum
            suite["finished_at"] = timestamp()
            if started is not None:
                suite["elapsed_seconds"] = time.monotonic() - started
            write_run_state(path, state)
            print(f"Walltime suite {suite['name']}: {suite['status']}", flush=True)
            if exit_code:
                break
        if exit_code == 0:
            state["status"] = "success"
        state.update(
            finished_at=timestamp(),
            exit_code=exit_code,
            successful_suites=sum(
                suite["status"] == "success" for suite in suite_states
            ),
        )
        write_run_state(path, state)
        return exit_code


def positive_seconds(value: str) -> float:
    result = float(value)
    if not math.isfinite(result) or result <= 0:
        raise argparse.ArgumentTypeError(
            "Expected a finite, positive number of seconds"
        )
    return result


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan")
    run = commands.add_parser("run")
    run.add_argument("shard", type=int)
    run.add_argument("--output", type=Path, help="A new directory for the run state")
    limit = run.add_mutually_exclusive_group()
    limit.add_argument("--timeout-seconds", type=positive_seconds)
    limit.add_argument(
        "--deadline", type=positive_seconds, help="Unix timestamp of the shard deadline"
    )
    args = parser.parse_args()
    environment = MappingProxyType(dict(os.environ))
    cargo = invocation_executable("cargo", environment)
    if cargo is None:
        raise ValueError("Could not identify the cargo executable")
    plan = root / ".cache/bench-walltime-shards.json"
    artifacts = built_artifacts(root, cargo=str(cargo), environment=environment)
    source = source_identity(root, environment=environment)
    if args.command == "plan":
        expected = prepare_plan(
            source,
            producer_metadata(root, cargo=str(cargo), environment=environment),
            artifacts,
        )
        plan.parent.mkdir(parents=True, exist_ok=True)
        plan.write_text(json.dumps(expected, indent=2) + "\n")
        matrix = {
            "include": [
                {"index": item["index"], "total": item["total"]}
                for item in expected["shards"]
            ]
        }
        if output := environment.get("GITHUB_OUTPUT"):
            with Path(output).open("a") as stream:
                print(
                    "matrix=" + json.dumps(matrix, separators=(",", ":")), file=stream
                )
        print(json.dumps(expected, indent=2))
        return
    prepared = json.loads(plan.read_text())
    selected = verify_plan(prepared, source, artifacts, args.shard)
    artifact_directory = next(iter(artifacts.values())).parent
    print(
        f"Running walltime shard {args.shard}/{selected['total']}: {', '.join(selected['benches'])}",
        flush=True,
    )
    directory = (
        args.output
        or root
        / ".cache/bench-walltime-runs"
        / f"shard-{args.shard}-{uuid.uuid4().hex}"
    )
    consumer = consumer_metadata(root, environment)
    metadata = {
        "source": source,
        "producer": prepared["producer"],
        "consumer": consumer,
        "plan_sha256": digest(plan),
        "shard": selected,
    }
    sys.exit(
        run_suites(
            root,
            directory,
            metadata,
            suite_commands(
                prepared, selected, cargo=consumer["cargo"]["invocation_executable"]
            ),
            timeout_seconds=args.timeout_seconds,
            deadline=args.deadline,
            environment=environment,
            before_launch=lambda suite: verify_suite_launch(
                root,
                prepared,
                selected,
                consumer,
                environment,
                artifact_directory,
                suite,
            ),
        )
    )


if __name__ == "__main__":
    main()
