"""Run one owned C-probe fault to inspect the runner's core collection path."""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import resource
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import collect_native_crashes as capture
from native_crash_elf import CaptureError, executable_build_id, require

CORE_PATTERN = (
    "|/usr/lib/systemd/systemd-coredump %P %u %g %s %t 9223372036854775808 %h %d"
)
HANDLER_SHA256 = "e7fd66e6ea9b20fb9959b2c00409aac7ccc0ee2afcabe0307521f121ea88d07e"
COREDUMPCTL_SHA256 = "bd1112a9861afa76d516a0aa30556b48191a8348ad09e7cd971e361b960a0305"
PROBE_SHA256 = "ced541b46f4cdd4955204949c90197e5134653790db0b5bc3c45ba387f03fb01"


def settings() -> dict:
    paths = {
        "core_pattern": "/proc/sys/kernel/core_pattern",
        "core_pipe_limit": "/proc/sys/kernel/core_pipe_limit",
        "core_uses_pid": "/proc/sys/kernel/core_uses_pid",
        "suid_dumpable": "/proc/sys/fs/suid_dumpable",
        "coredump_filter": "/proc/self/coredump_filter",
    }
    result = {name: Path(path).read_text().strip() for name, path in paths.items()}
    require(all(len(value) <= 4096 for value in result.values()), "oversized_setting")
    result["rlimit_core"] = list(resource.getrlimit(resource.RLIMIT_CORE))
    result["rlimit_fsize"] = list(resource.getrlimit(resource.RLIMIT_FSIZE))
    return result


def file_identity(path: Path) -> dict:
    resolved = path.resolve(strict=True)
    require(resolved.stat().st_size <= capture.MAX_ELF, "identity_file_size")
    return {"path": str(resolved), "sha256": capture.sha256(resolved)}


def next_whole_second() -> float:
    # A kernel crash timestamp names a whole second, which must fit inside the interval.
    started = time.monotonic()
    boundary = (time.time_ns() // 1_000_000_000 + 1) * 1_000_000_000
    delay = max(0, (boundary - time.time_ns()) / 1_000_000_000 + 0.01)
    require(delay <= 1.1, "clock_moved_backwards")
    time.sleep(delay)
    require(time.time_ns() >= boundary, "clock_did_not_advance")
    return time.monotonic() - started


def run_probe(executable: Path, report: dict, report_path: Path) -> int:
    environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
    started = time.time_ns() // 1000
    process = subprocess.Popen(
        [str(executable), "fault"],
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    report["probe"] = {
        "launch_count": 1,
        "pid": process.pid,
        "start_us": started,
        "environment_keys": sorted(environment),
        "deadline_seconds": 10,
    }
    try:
        capture.write_json(report_path, report)
        try:
            code = process.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise CaptureError("probe_deadline") from error
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired as error:
            raise CaptureError("probe_cleanup_deadline") from error
        report["probe"].update(
            {
                "end_us": time.time_ns() // 1000,
                "returncode": process.returncode,
                "reaped": process.returncode is not None,
            }
        )
    require(code == -signal.SIGSEGV, "unexpected_probe_status")
    return process.pid


def execute(
    workspace: Path, state: Path, expected_revision: str, report: dict, deadline: float
) -> None:
    require(
        re.fullmatch("[0-9a-f]{40}", expected_revision) is not None,
        "invalid_expected_revision",
    )
    revision = (
        capture.bounded_command(
            ["git", "rev-parse", "HEAD"], seconds=5, limit=128, cwd=workspace
        )
        .decode()
        .strip()
    )
    status = capture.bounded_command(
        ["git", "status", "--porcelain=v1", "--untracked-files=normal"],
        seconds=5,
        limit=4096,
        cwd=workspace,
    )
    report["source"] = {
        "expected_revision": expected_revision,
        "revision": revision,
        "clean": not status,
        "sha256": capture.source_hashes(workspace),
    }
    require(revision == expected_revision and not status, "source_gate_failed")
    report["before"] = settings()
    report["host"] = {
        "hostname": platform.node(),
        "kernel": platform.release(),
        "boot_id": capture.boot_id(),
        "uid": os.getuid(),
        "gid": os.getgid(),
    }
    report["host"]["os_release"] = {
        key: value.strip('"')
        for line in Path("/etc/os-release").read_text().splitlines()
        if "=" in line
        for key, value in [line.split("=", 1)]
        if key in ("ID", "VERSION_ID", "PRETTY_NAME")
    }
    packages = (
        capture.bounded_command(
            [
                "dpkg-query",
                "--show",
                "--showformat=${Package}=${Version}\n",
                "libc6",
                "libc-bin",
                "systemd-coredump",
            ],
            seconds=5,
            limit=4096,
        )
        .decode()
        .splitlines()
    )
    report["host"]["packages"] = dict(line.split("=", 1) for line in packages)
    report["host_files"] = {
        name: file_identity(Path(path))
        for name, path in {
            "handler": "/usr/lib/systemd/systemd-coredump",
            "coredumpctl": "/usr/bin/coredumpctl",
            "libc": "/lib/x86_64-linux-gnu/libc.so.6",
            "loader": "/lib64/ld-linux-x86-64.so.2",
        }.items()
    }
    require(report["before"]["core_pattern"] == CORE_PATTERN, "unexpected_core_pattern")
    require(
        report["host_files"]["handler"]["sha256"] == HANDLER_SHA256,
        "unexpected_handler",
    )
    require(
        report["host_files"]["coredumpctl"]["sha256"] == COREDUMPCTL_SHA256,
        "unexpected_coredumpctl",
    )
    source = workspace / "scripts/native_crash_probe.c"
    require(capture.sha256(source) == PROBE_SHA256, "unexpected_probe_source")
    compiler = shutil.which("cc", path="/usr/bin:/bin")
    require(compiler is not None, "compiler_unavailable")
    report["compiler"] = file_identity(Path(compiler))
    version = (
        capture.bounded_command([compiler, "--version"], seconds=5, limit=4096)
        .decode()
        .splitlines()[0]
    )
    report["compiler"]["version"] = version[:256]
    collector_state = state / "collector"
    capture.prepare(workspace, collector_state)
    context_path = collector_state / "context.json"
    context = json.loads(context_path.read_text())
    context.pop("profile")
    context.pop("cargo_features_argument")
    context["executable_kind"] = "owned C probe, not the uv application"
    capture.write_json(context_path, context)
    executable = Path(context["executable"])
    executable.parent.mkdir(parents=True, exist_ok=True)
    require(not executable.exists(), "probe_output_exists")
    command = [
        compiler,
        "-g",
        "-O0",
        "-fno-inline",
        "-fno-omit-frame-pointer",
        "-Wl,--build-id=sha1",
        str(source),
        "-o",
        str(executable),
    ]
    report["compile_command"] = command
    capture.bounded_command(command, seconds=15, limit=4096, cwd=workspace)
    report["probe_elf"] = file_identity(executable) | {
        "build_id": executable_build_id(executable),
        "bytes": executable.stat().st_size,
    }
    require(deadline - time.monotonic() >= 132, "insufficient_capture_budget")
    report["before_fault_bracket_seconds"] = next_whole_second()
    pid = run_probe(executable, report, state / "export/probe.json")
    report["after_fault_bracket_seconds"] = next_whole_second()
    report["expected_pid_constraint"] = pid
    capture.bounded_command(
        [
            sys.executable,
            "-B",
            str(workspace / "scripts/collect_native_crashes.py"),
            "collect",
            "--workspace",
            str(workspace),
            "--state",
            str(collector_state),
            "--expected-pid",
            str(pid),
        ],
        seconds=115,
        limit=4096,
        cwd=workspace,
    )
    result_path = collector_state / "export/capture.json"
    require(result_path.stat().st_size <= capture.MAX_JSON, "collector_result_size")
    result = json.loads(result_path.read_text())
    require(result.get("schema_version") == 1, "collector_result_schema")
    capture.write_json(state / "export/capture.json", result)
    report["collector_result_sha256"] = capture.sha256(state / "export/capture.json")
    if result.get("outcome") == "no_matching_core_records":
        raise CaptureError("matching_record_not_visible")
    require(result.get("matching_records") == 1, "collector_record_selection_failed")
    captures = result.get("captures", [])
    require(
        len(captures) == 1 and captures[0].get("pid") == pid, "collector_pid_mismatch"
    )
    observation = captures[0]
    report["core_access"] = observation.get("external_core_access", "not_established")
    require(
        observation.get("identity", {}).get("pid") == pid,
        "core_identity_not_established",
    )
    require(
        observation["identity"]["build_id"] == report["probe_elf"]["build_id"],
        "probe_build_id_mismatch",
    )
    require(observation.get("outcome") == "frames_captured", "frame_capture_failed")
    found = any(
        frame["function"] == "owned_signal_site"
        for thread in observation["frames"]["threads"]
        for frame in thread["frames"]
    )
    require(found, "probe_frame_unavailable")
    report["outcome"] = "owned_probe_capture_passed"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--expected-revision", required=True)
    arguments = parser.parse_args()
    workspace = arguments.workspace.resolve(strict=True)
    state = arguments.state.resolve()
    os.umask(0o077)
    state.mkdir(mode=0o700)
    (state / "export").mkdir()
    report = {
        "schema_version": 1,
        "outcome": "owned_probe_capture_failed",
        "probe": {"launch_count": 0},
        "executable_kind": "owned C probe, not the uv application",
    }

    def expired(_signal, _frame):
        raise CaptureError("diagnostic_deadline")

    signal.signal(signal.SIGALRM, expired)
    deadline = time.monotonic() + 150
    signal.setitimer(signal.ITIMER_REAL, 150)
    try:
        execute(workspace, state, arguments.expected_revision, report, deadline)
    except (CaptureError, OSError, ValueError, KeyError) as error:
        report["error"] = (
            str(error) if isinstance(error, CaptureError) else type(error).__name__
        )
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        if "before" in report:
            report["after"] = settings()
            report["settings_unchanged"] = report["before"] == report["after"]
            if not report["settings_unchanged"]:
                report["outcome"] = "owned_probe_capture_failed"
                report["error"] = "settings_changed"
        capture.write_json(state / "export/probe.json", report)
    print(f"Owned native probe: {report.get('error', report['outcome'])}")
    raise SystemExit(0 if report["outcome"] == "owned_probe_capture_passed" else 1)


if __name__ == "__main__":
    main()
