"""Collect bounded evidence for Linux `SIGSEGV` records matching the configured executable path."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import selectors
import shutil
import signal
import stat
import subprocess
import time
import urllib.request
from pathlib import Path

from native_crash_elf import CaptureError, core_identity, executable_build_id, require

MAX_CORE = 256 * 1024 * 1024
MAX_ELF = 256 * 1024 * 1024
MAX_JSON = 256 * 1024
MESSAGE_ID = "fc2e22bc6ee647b6b90729ab34a250b1"
FIELDS = (
    "_BOOT_ID",
    "MESSAGE_ID",
    "COREDUMP_PID",
    "COREDUMP_UID",
    "COREDUMP_GID",
    "COREDUMP_EXE",
    "COREDUMP_SIGNAL",
    "COREDUMP_TIMESTAMP",
    "COREDUMP_DUMPABLE",
    "COREDUMP_TRUNCATED",
    "COREDUMP_FILENAME",
    "COREDUMP_PACKAGE_JSON",
)
TUPLE_FIELDS = (
    "_BOOT_ID",
    "MESSAGE_ID",
    "COREDUMP_PID",
    "COREDUMP_UID",
    "COREDUMP_GID",
    "COREDUMP_EXE",
    "COREDUMP_SIGNAL",
    "COREDUMP_TIMESTAMP",
    "COREDUMP_FILENAME",
)
SOURCE_FILES = (
    ".github/workflows/test.yml",
    ".github/workflows/ci.yml",
    "scripts/run_owned_native_crash.py",
    "scripts/native_crash_probe.c",
    "scripts/collect_native_crashes.py",
    "scripts/native_crash_elf.py",
    "scripts/native_crash_frames.py",
    "scripts/native_crash_debugger.json",
)


def minimal_environment() -> dict[str, str]:
    return {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "DEBUGINFOD_URLS": ""}


def bounded_command(
    command: list[str],
    *,
    seconds: float,
    limit: int,
    destination: Path | None = None,
    environment: dict | None = None,
    cwd: Path | None = None,
) -> bytes:
    """Bound standard output and time; discard standard error. Callers handle standard output."""
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=environment or minimal_environment(),
        cwd=cwd,
        start_new_session=True,
    )
    output = bytearray()
    target = None
    deadline = time.monotonic() + seconds
    size = 0
    try:
        target = destination.open("xb") if destination else None
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                require(remaining > 0, "command_deadline")
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    size += len(chunk)
                    require(size <= limit, "command_output_limit")
                    if target:
                        target.write(chunk)
                    else:
                        output.extend(chunk)
            remaining = deadline - time.monotonic()
            require(remaining > 0, "command_deadline")
            try:
                code = process.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                raise CaptureError("command_deadline") from error
            require(code == 0, "command_failed")
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired as error:
            raise CaptureError("command_cleanup_deadline") from error
        if process.stdout:
            process.stdout.close()
        if target:
            target.close()
    return bytes(output)


def sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path: Path, value: dict) -> None:
    data = (json.dumps(value, indent=2) + "\n").encode()
    require(len(data) <= MAX_JSON, "json_export_limit")
    path.write_bytes(data)


def boot_id() -> str:
    value = Path("/proc/sys/kernel/random/boot_id").read_text().strip().replace("-", "")
    require(re.fullmatch("[0-9a-f]{32}", value) is not None, "invalid_boot_id")
    return value


def source_hashes(workspace: Path) -> dict[str, str]:
    return {name: sha256(workspace / name) for name in SOURCE_FILES}


def prepare(workspace: Path, state: Path) -> None:
    state.mkdir(mode=0o700)
    workspace = workspace.resolve(strict=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", workspace / "target")).resolve()
    require(target == workspace / "target", "unsupported_target_directory")
    revision = (
        bounded_command(
            ["git", "rev-parse", "HEAD"], cwd=workspace, seconds=5, limit=128
        )
        .decode()
        .strip()
    )
    require(
        re.fullmatch("[0-9a-f]{40}", revision) is not None, "invalid_source_revision"
    )
    write_json(
        state / "context.json",
        {
            "schema_version": 1,
            "start_us": time.time_ns() // 1000,
            "boot_id": boot_id(),
            "uid": os.getuid(),
            "gid": os.getgid(),
            "workspace": str(workspace),
            "executable": str(target / "fast-build-nightly/uv"),
            "source_revision": revision,
            "source_sha256": source_hashes(workspace),
            "profile": "fast-build-nightly",
            "cargo_features_argument": "test-python-patch,test-universal,native-auth,secret-service",
        },
    )


def journal_matches(context: dict) -> list[str]:
    matches = [
        f"_BOOT_ID={context['boot_id']}",
        f"COREDUMP_UID={context['uid']}",
        f"COREDUMP_GID={context['gid']}",
        f"COREDUMP_EXE={context['executable']}",
        "COREDUMP_SIGNAL=11",
        f"MESSAGE_ID={MESSAGE_ID}",
    ]
    if "expected_pid" in context:
        matches.append(f"COREDUMP_PID={context['expected_pid']}")
    return matches


def query_records(
    context: dict, end_us: int, extra: list[str] | None = None
) -> list[dict]:
    command = [
        "journalctl",
        "--quiet",
        "--no-pager",
        "--all",
        "--output=json",
        f"--output-fields={','.join(FIELDS)}",
        f"--since=@{context['start_us'] / 1e6:.6f}",
        f"--until=@{end_us / 1e6:.6f}",
        *journal_matches(context),
        *(extra or []),
    ]
    raw = bounded_command(command, seconds=5, limit=MAX_JSON)
    lines = raw.splitlines()
    require(len(lines) <= 8, "journal_record_limit")
    result = [json.loads(line) for line in lines]
    require(all(isinstance(item, dict) for item in result), "invalid_journal_record")
    return result


def select_records(records: list[dict], context: dict, end_us: int) -> list[dict]:
    selected = []
    identities = set()
    for entry in records:
        for key in FIELDS:
            if key in entry:
                require(isinstance(entry[key], str), "invalid_journal_field")
        expected = {
            "_BOOT_ID": context["boot_id"],
            "MESSAGE_ID": MESSAGE_ID,
            "COREDUMP_UID": str(context["uid"]),
            "COREDUMP_GID": str(context["gid"]),
            "COREDUMP_EXE": context["executable"],
            "COREDUMP_SIGNAL": "11",
        }
        if "expected_pid" in context:
            expected["COREDUMP_PID"] = str(context["expected_pid"])
        require(
            all(entry.get(key) == value for key, value in expected.items()),
            "journal_identity_mismatch",
        )
        require(entry.get("COREDUMP_DUMPABLE") == "1", "unsupported_dumpability")
        require(entry.get("COREDUMP_TRUNCATED", "0") == "0", "truncated_core")
        pid = entry.get("COREDUMP_PID", "")
        timestamp = entry.get("COREDUMP_TIMESTAMP", "")
        require(
            pid.isdecimal() and 0 < int(pid) < 2**31 and timestamp.isdecimal(),
            "invalid_process_identity",
        )
        crash_us = int(timestamp)
        require(crash_us % 1_000_000 == 0, "unsupported_timestamp")
        # The kernel supplies whole seconds; boundary seconds cannot prove membership.
        require(
            context["start_us"] <= crash_us and crash_us + 1_000_000 <= end_us,
            "crash_outside_interval",
        )
        filename = entry.get("COREDUMP_FILENAME", "")
        expected_name = f"/var/lib/systemd/coredump/core.uv.{context['uid']}.{context['boot_id']}.{pid}.{timestamp}"
        require(
            filename
            in [expected_name + suffix for suffix in ("", ".zst", ".lz4", ".xz")],
            "unsupported_core_filename",
        )
        identity = tuple(entry[key] for key in TUPLE_FIELDS)
        require(identity not in identities, "ambiguous_journal_identity")
        identities.add(identity)
        selected.append(entry)
    return sorted(
        selected,
        key=lambda entry: (
            int(entry["COREDUMP_TIMESTAMP"]),
            int(entry["COREDUMP_PID"]),
        ),
    )


def journal_build_id(entry: dict, executable: str) -> str | None:
    metadata = json.loads(entry.get("COREDUMP_PACKAGE_JSON", "{}"))
    require(isinstance(metadata, dict), "invalid_package_metadata")
    matching = [
        value
        for name, value in metadata.items()
        if name in (executable, Path(executable).name)
    ]
    require(len(matching) <= 1, "ambiguous_journal_build_id")
    if not matching:
        return None
    require(isinstance(matching[0], dict), "invalid_package_metadata")
    value = matching[0].get("buildId")
    if value is not None:
        require(
            isinstance(value, str)
            and re.fullmatch("[0-9a-f]{8,128}", value) is not None,
            "invalid_journal_build_id",
        )
    return value


def copy_executable(source: Path, destination: Path, uid: int) -> dict:
    def identity(metadata: os.stat_result) -> tuple:
        return (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_size,
            metadata.st_mtime_ns,
            metadata.st_ctime_ns,
        )

    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as opened:
        before = os.fstat(opened.fileno())
        require(
            stat.S_ISREG(before.st_mode)
            and before.st_uid == uid
            and 0 < before.st_size <= MAX_ELF,
            "unsupported_executable",
        )
        digest = hashlib.sha256()
        size = 0
        with destination.open("xb") as copied:
            while chunk := opened.read(1024 * 1024):
                size += len(chunk)
                require(size <= MAX_ELF, "executable_size_limit")
                digest.update(chunk)
                copied.write(chunk)
        require(
            identity(before)
            == identity(os.fstat(opened.fileno()))
            == identity(source.stat(follow_symlinks=False)),
            "executable_changed",
        )
        require(size == before.st_size, "executable_changed")
    return {
        "sha256": digest.hexdigest(),
        "bytes": size,
        "build_id": executable_build_id(destination),
        "device": before.st_dev,
        "inode": before.st_ino,
        "mtime_ns": before.st_mtime_ns,
        "ctime_ns": before.st_ctime_ns,
    }


def stage_debugger(workspace: Path, private: Path) -> tuple[Path, dict]:
    manifest = json.loads(
        (workspace / "scripts/native_crash_debugger.json").read_text()
    )
    root = private / "debugger"
    root.mkdir()
    for item in manifest["packages"]:
        package = private / f"{item['package']}.deb"
        require(
            item["url"].startswith("https://archive.ubuntu.com/ubuntu/pool/"),
            "unsupported_debugger_origin",
        )
        with (
            urllib.request.urlopen(item["url"], timeout=10) as response,
            package.open("xb") as output,
        ):
            size = 0
            while chunk := response.read(65536):
                size += len(chunk)
                require(size <= item["bytes"], "debugger_package_size")
                output.write(chunk)
        require(
            size == item["bytes"] and sha256(package) == item["sha256"],
            "debugger_package_digest",
        )
        bounded_command(
            ["dpkg-deb", "--extract", str(package), str(root)], seconds=5, limit=4096
        )
    gdb = root / "usr/bin/gdb"
    require(sha256(gdb) == manifest["gdb_sha256"], "debugger_digest")
    environment = minimal_environment() | {
        "LD_LIBRARY_PATH": str(root / "usr/lib/x86_64-linux-gnu")
    }
    bounded_command(
        [str(gdb), "--nx", "--nh", "--batch", "--version"],
        seconds=5,
        limit=4096,
        environment=environment,
    )
    return root, {
        "sha256": manifest["gdb_sha256"],
        "version": manifest["packages"][0]["version"],
    }


def validate_frames(value: dict) -> dict:
    require(
        isinstance(value, dict)
        and set(value) == {"schema_version", "threads", "threads_truncated"},
        "invalid_frame_schema",
    )
    require(
        value["schema_version"] == 1 and isinstance(value["threads_truncated"], bool),
        "invalid_frame_schema",
    )
    require(
        isinstance(value["threads"], list) and len(value["threads"]) <= 16,
        "invalid_frame_schema",
    )
    for thread in value["threads"]:
        require(
            isinstance(thread, dict)
            and set(thread)
            <= {"lwp", "selected", "frames", "frames_truncated", "error"},
            "invalid_frame_schema",
        )
        require(
            isinstance(thread.get("lwp"), int)
            and isinstance(thread.get("selected"), bool),
            "invalid_frame_schema",
        )
        require(
            thread.get("error") in (None, "frame_unavailable"), "invalid_frame_schema"
        )
        require(
            isinstance(thread.get("frames"), list) and len(thread["frames"]) <= 32,
            "invalid_frame_schema",
        )
        for frame in thread["frames"]:
            require(
                isinstance(frame, dict) and set(frame) == {"pc", "function", "type"},
                "invalid_frame_schema",
            )
            require(
                isinstance(frame["pc"], str)
                and re.fullmatch("0x[0-9a-f]{1,16}", frame["pc"]) is not None,
                "invalid_frame_schema",
            )
            require(isinstance(frame["type"], int), "invalid_frame_schema")
            name = frame["function"]
            require(
                name is None
                or isinstance(name, str)
                and len(name) <= 256
                and all(32 <= ord(character) < 127 for character in name),
                "invalid_frame_schema",
            )
    return value


def export_frames(
    workspace: Path, root: Path, private: Path, executable: Path, core: Path, index: int
) -> dict:
    output = private / f"frames-{index}.json"
    environment = minimal_environment() | {
        "LD_LIBRARY_PATH": str(root / "usr/lib/x86_64-linux-gnu"),
        "UV_CRASH_FRAMES_PATH": str(output),
        "XDG_CACHE_HOME": str(private / "cache"),
    }
    command = [
        str(root / "usr/bin/gdb"),
        "--nx",
        "--nh",
        "--batch",
        f"--data-directory={root / 'usr/share/gdb'}",
        "-iex",
        "set auto-load off",
        "-iex",
        "set debuginfod enabled off",
        "-iex",
        "set pagination off",
        f"--se={executable}",
        f"--core={core}",
        "-x",
        str(workspace / "scripts/native_crash_frames.py"),
    ]
    bounded_command(command, seconds=10, limit=MAX_JSON, environment=environment)
    require(output.stat().st_size <= MAX_JSON, "frame_output_limit")
    return validate_frames(json.loads(output.read_text()))


def collect(
    workspace: Path, state: Path, result: dict, expected_pid: int | None = None
) -> None:
    end_us = time.time_ns() // 1000
    context = json.loads((state / "context.json").read_text())
    require(context["schema_version"] == 1, "unsupported_context")
    if expected_pid is not None:
        require(0 < expected_pid < 2**31, "invalid_expected_pid")
        context["expected_pid"] = expected_pid
    require(
        context["boot_id"] == boot_id()
        and context["uid"] == os.getuid()
        and context["gid"] == os.getgid(),
        "job_identity_changed",
    )
    require(
        context["workspace"] == str(workspace.resolve(strict=True))
        and context["source_sha256"] == source_hashes(workspace),
        "source_identity_changed",
    )
    executable = Path(context["executable"])
    require(
        executable == workspace / "target/fast-build-nightly/uv"
        and executable.resolve(strict=True) == executable,
        "unexpected_executable_path",
    )
    result["context"] = context | {"end_us": end_us, "test_name_attribution": False}
    result["stage"] = "journal_query"
    records = select_records(query_records(context, end_us), context, end_us)
    if expected_pid is not None:
        require(len(records) <= 1, "ambiguous_expected_pid")
    result["matching_records"] = len(records)
    if not records:
        result["outcome"] = "no_matching_core_records"
        return
    private = state / "private"
    private.mkdir(mode=0o700)
    copied = private / "uv"
    identity = copy_executable(executable, copied, context["uid"])
    result["executable"] = identity
    result["captures"] = []
    debugger = None
    for index, entry in enumerate(records[:2]):
        capture = {
            "pid": int(entry["COREDUMP_PID"]),
            "crash_us": int(entry["COREDUMP_TIMESTAMP"]),
            "journal_identity": {key: entry[key] for key in TUPLE_FIELDS},
        }
        result["captures"].append(capture)
        try:
            capture["stage"] = "record_identity"
            journal_id = journal_build_id(entry, str(executable))
            require(
                journal_id is None or journal_id == identity["build_id"],
                "journal_build_id_mismatch",
            )
            exact = [f"{key}={entry[key]}" for key in TUPLE_FIELDS]
            repeated = select_records(
                query_records(context, end_us, exact), context, end_us
            )
            require(
                len(repeated) == 1
                and all(repeated[0][key] == entry[key] for key in TUPLE_FIELDS),
                "retrieval_identity_changed",
            )
            capture["stage"] = "external_core_access"
            try:
                descriptor = os.open(
                    entry["COREDUMP_FILENAME"], os.O_RDONLY | os.O_NOFOLLOW
                )
            except PermissionError as error:
                capture["external_core_access"] = "permission_denied"
                raise CaptureError("core_access_denied") from error
            except FileNotFoundError as error:
                capture["external_core_access"] = "unavailable"
                raise CaptureError("core_not_available") from error
            try:
                require(
                    stat.S_ISREG(os.fstat(descriptor).st_mode), "unsupported_core_file"
                )
            finally:
                os.close(descriptor)
            capture["external_core_access"] = "readable"
            capture["stage"] = "core_retrieval"
            core = private / f"core-{index}"
            bounded_command(
                ["coredumpctl", "--no-pager", "--quiet", "dump", *exact],
                seconds=15,
                limit=MAX_CORE,
                destination=core,
            )
            capture["stage"] = "core_identity"
            observed = core_identity(core, str(executable))
            require(
                observed
                == {
                    "pid": capture["pid"],
                    "uid": context["uid"],
                    "gid": context["gid"],
                    "signal": 11,
                    "build_id": identity["build_id"],
                },
                "core_identity_mismatch",
            )
            capture.update(
                {
                    "identity": observed,
                    "core_bytes": core.stat().st_size,
                    "journal_build_id": journal_id,
                    "outcome": "identity_validated",
                }
            )
            binary_export = state / "executable"
            binary_export.mkdir(exist_ok=True)
            if not (binary_export / "uv").exists():
                shutil.copyfile(copied, binary_export / "uv")
            if debugger is None:
                capture["stage"] = "debugger_staging"
                debugger, result["debugger"] = stage_debugger(workspace, private)
                result["debugger_host_files"] = {
                    str(path): {
                        "sha256": sha256(path),
                        "build_id": executable_build_id(path),
                    }
                    for path in (
                        Path("/lib/x86_64-linux-gnu/libc.so.6"),
                        Path("/lib64/ld-linux-x86-64.so.2"),
                    )
                }
            capture["stage"] = "frame_export"
            capture["frames"] = export_frames(
                workspace, debugger, private, copied, core, index
            )
            if len(json.dumps(result, indent=2).encode()) > MAX_JSON - 4096:
                capture.pop("frames")
                raise CaptureError("total_frame_export_limit")
            capture["outcome"] = "frames_captured"
        except (CaptureError, OSError, ValueError) as error:
            if isinstance(error, CaptureError) and str(error) == "collector_deadline":
                raise
            capture["error"] = (
                str(error) if isinstance(error, CaptureError) else type(error).__name__
            )
    result["records_not_processed"] = max(0, len(records) - 2)
    result["outcome"] = "collection_finished"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("prepare", "collect"))
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--expected-pid", type=int)
    arguments = parser.parse_args()
    workspace = arguments.workspace.resolve()
    state = arguments.state.resolve()
    os.umask(0o077)

    def expired(_signal, _frame):
        raise CaptureError("collector_deadline")

    signal.signal(signal.SIGALRM, expired)
    # Leave time to reap the collector's child and write its bounded result.
    signal.setitimer(signal.ITIMER_REAL, 15 if arguments.mode == "prepare" else 105)
    result = {"schema_version": 1, "outcome": "collection_failed"}
    try:
        if arguments.mode == "prepare":
            prepare(workspace, state)
            result["outcome"] = "context_recorded"
        else:
            collect(workspace, state, result, arguments.expected_pid)
    except (CaptureError, OSError, ValueError, KeyError) as error:
        result["error"] = (
            str(error) if isinstance(error, CaptureError) else type(error).__name__
        )
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        if arguments.mode == "collect":
            export = state / "export"
            export.mkdir(parents=True, exist_ok=True)
            write_json(export / "capture.json", result)
        print(f"Native crash diagnostics: {result.get('error', result['outcome'])}")


if __name__ == "__main__":
    main()
