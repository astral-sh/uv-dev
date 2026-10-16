"""Read Linux core capture prerequisites without changing the runner configuration."""

import argparse
import hashlib
import json
import os
import re
import resource
import shutil
import socket
import stat
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

MAX_TEXT_BYTES = 16_384
MAX_CONFIG_FILES = 16
MAX_IDENTITY_BYTES = 64 * 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 3
OS_KEYS = {"ID", "ID_LIKE", "NAME", "PRETTY_NAME", "VERSION", "VERSION_ID"}
COREDUMP_KEYS = {
    "Storage",
    "Compress",
    "ProcessSizeMax",
    "ExternalSizeMax",
    "JournalSizeMax",
    "MaxUse",
    "KeepFree",
}
SERVICE_KEYS = {"LoadState", "ActiveState", "SubState", "UnitFileState", "FragmentPath"}
CONFIG_DIRS = (
    "/usr/lib/systemd/coredump.conf.d",
    "/usr/local/lib/systemd/coredump.conf.d",
    "/run/systemd/coredump.conf.d",
    "/etc/systemd/coredump.conf.d",
)
KERNEL_FILES = {
    "core_pattern": "/proc/sys/kernel/core_pattern",
    "core_uses_pid": "/proc/sys/kernel/core_uses_pid",
    "core_pipe_limit": "/proc/sys/kernel/core_pipe_limit",
    "suid_dumpable": "/proc/sys/fs/suid_dumpable",
}


def read_text(path):
    try:
        with Path(path).open("rb") as stream:
            contents = stream.read(MAX_TEXT_BYTES + 1)
        if len(contents) > MAX_TEXT_BYTES:
            return {"path": str(path), "error": "file exceeds bounded read"}
        return {"path": str(path), "value": contents.decode("utf-8", errors="replace")}
    except OSError as error:
        return {"path": str(path), "error": str(error)}


def identity(path):
    try:
        resolved = Path(path).resolve(strict=True)
        metadata = resolved.stat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_IDENTITY_BYTES:
            return {
                "path": str(path),
                "error": "identity requires a regular file at most 64 MiB",
            }
        digest = hashlib.sha256()
        total = 0
        with resolved.open("rb") as stream:
            while contents := stream.read(1024 * 1024):
                total += len(contents)
                if total > MAX_IDENTITY_BYTES:
                    return {"path": str(path), "error": "identity exceeded 64 MiB"}
                digest.update(contents)
        return {
            "path": str(path),
            "resolved_path": str(resolved),
            "size": metadata.st_size,
            "sha256": digest.hexdigest(),
        }
    except OSError as error:
        return {"path": str(path), "error": str(error)}


def command(arguments, directory):
    # Each caller supplies fixed read-only commands; no command comes from workflow input.
    record = {"argv": arguments}
    try:
        result = subprocess.run(
            arguments,
            cwd=directory,
            env={
                "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "LANG": "C",
                "LC_ALL": "C",
                "GIT_OPTIONAL_LOCKS": "0",
                "SYSTEMD_PAGER": "cat",
                "SYSTEMD_COLORS": "0",
            },
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=COMMAND_TIMEOUT_SECONDS,
            check=False,
        )
        record["exit_code"] = result.returncode
        for name, contents in (("stdout", result.stdout), ("stderr", result.stderr)):
            record[name] = contents[:MAX_TEXT_BYTES].decode("utf-8", errors="replace")
            record[f"{name}_truncated"] = len(contents) > MAX_TEXT_BYTES
    except (OSError, subprocess.TimeoutExpired) as error:
        record["error"] = str(error)
    return record


def project_config(path, keys, section=None):
    record = read_text(path)
    if "value" not in record:
        return record
    current_section = None
    projected = []
    for number, line in enumerate(record.pop("value").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("[") and line.endswith("]"):
            current_section = line[1:-1]
        elif "=" in line and (section is None or current_section == section):
            key, value = line.split("=", 1)
            if key.strip() in keys:
                projected.append(
                    {"line": number, "key": key.strip(), "value": value.strip()}
                )
    record["uncommented_assignments"] = projected
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--context", choices=("local", "depot"), required=True)
    parser.add_argument("--expected-revision", required=True)
    parser.add_argument("--shell-version", required=True)
    for name in ("core-soft", "core-hard", "fsize-soft", "fsize-hard"):
        parser.add_argument(f"--shell-{name}", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    revision = command(["git", "rev-parse", "HEAD"], repository)
    status = command(
        [
            "git",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ],
        repository,
    )
    tracked = command(
        ["git", "ls-files", "--error-unmatch", "scripts/inspect_linux_core_capture.py"],
        repository,
    )
    expected_valid = bool(re.fullmatch(r"[0-9a-f]{40}", arguments.expected_revision))
    revision_matches = (
        expected_valid
        and revision.get("stdout", "").strip() == arguments.expected_revision
    )
    tracked_clean = status.get("exit_code") == 0 and not status.get("stdout")
    report = {
        "schema_version": 1,
        "observed_at_utc": datetime.now(UTC).isoformat(),
        "context": arguments.context,
        "expected_runner_label": "depot-ubuntu-24.04-16"
        if arguments.context == "depot"
        else None,
        "hostname": socket.gethostname(),
        "source": {
            "expected_revision": arguments.expected_revision,
            "revision_matches": revision_matches,
            "revision": revision,
            "tracked_status": status,
            "script_tracked": tracked,
            "script": identity(__file__),
            "workflow": identity(repository / ".github/workflows/ci.yml"),
        },
        "github": {
            key: os.environ.get(key)
            for key in (
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_SHA",
                "RUNNER_OS",
                "RUNNER_ARCH",
            )
        },
        "kernel": dict(
            zip(
                ("sysname", "nodename", "release", "version", "machine"),
                os.uname(),
                strict=True,
            )
        ),
        "os_release": project_config("/etc/os-release", OS_KEYS),
        "python": {"version": sys.version, "executable": identity(sys.executable)},
        "limits": {
            "normal_step_shell": {
                "bash_version": arguments.shell_version,
                "core_soft_raw": arguments.shell_core_soft,
                "core_hard_raw": arguments.shell_core_hard,
                "fsize_soft_raw": arguments.shell_fsize_soft,
                "fsize_hard_raw": arguments.shell_fsize_hard,
            },
            "inherited_by_python_bytes": {
                "core": list(resource.getrlimit(resource.RLIMIT_CORE)),
                "fsize": list(resource.getrlimit(resource.RLIMIT_FSIZE)),
                "unlimited_sentinel": resource.RLIM_INFINITY,
            },
        },
        "kernel_core_settings": {
            key: read_text(path) for key, path in KERNEL_FILES.items()
        },
        "self": {
            "pid": os.getpid(),
            "coredump_filter": read_text("/proc/self/coredump_filter"),
        },
    }
    namespaces = {}
    for name in ("mnt", "pid", "user", "uts", "ipc", "net", "cgroup", "time"):
        path = f"/proc/self/ns/{name}"
        try:
            namespaces[name] = {"path": path, "target": os.readlink(path)}
        except OSError as error:
            namespaces[name] = {"path": path, "error": str(error)}
    report["self"]["namespaces"] = namespaces
    report["packages"] = command(
        [
            "dpkg-query",
            "-W",
            "-f=${Package}\t${Version}\t${Architecture}\t${db:Status-Abbrev}\n",
            "libc6",
            "libc-bin",
            "gdb",
            "systemd-coredump",
            "apport",
        ],
        repository,
    )
    report["libc"] = {
        "gnu_libc_version": command(["getconf", "GNU_LIBC_VERSION"], repository),
        "x86_64_library_file": identity("/lib/x86_64-linux-gnu/libc.so.6"),
        "x86_64_loader_file": identity("/lib64/ld-linux-x86-64.so.2"),
    }
    availability = {}
    for name in (
        "gdb",
        "coredumpctl",
        "apport-cli",
        "apport-unpack",
        "systemctl",
        "readelf",
    ):
        path = shutil.which(
            name, path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        )
        availability[name] = identity(path) if path else {"available": False}
    availability["apport_handler"] = identity("/usr/share/apport/apport")
    report["tool_availability"] = availability
    report["tool_versions"] = {
        "gdb": command(["gdb", "--nx", "--nh", "--batch", "--version"], repository),
        "coredumpctl": command(["coredumpctl", "--version"], repository),
        "systemctl": command(["systemctl", "--version"], repository),
    }
    services = {}
    for unit in (
        "apport.service",
        "systemd-coredump.socket",
    ):
        result = command(
            [
                "systemctl",
                "show",
                "--no-pager",
                "--property=" + ",".join(sorted(SERVICE_KEYS)),
                unit,
            ],
            repository,
        )
        result["properties"] = {
            key: value
            for line in result.pop("stdout", "").splitlines()
            if "=" in line
            for key, value in [line.split("=", 1)]
            if key in SERVICE_KEYS
        }
        services[unit] = result
    report["services"] = services
    report["coredump_template_enablement"] = command(
        ["systemctl", "is-enabled", "systemd-coredump@.service"], repository
    )
    configs = [project_config("/etc/systemd/coredump.conf", COREDUMP_KEYS, "Coredump")]
    directories = []
    for directory in CONFIG_DIRS:
        try:
            paths = sorted(
                path
                for path in Path(directory).iterdir()
                if path.name.endswith(".conf")
            )
            directories.append(
                {
                    "path": directory,
                    "exists": True,
                    "matching_files": len(paths),
                    "truncated": len(paths) > MAX_CONFIG_FILES,
                }
            )
            configs.extend(
                project_config(path, COREDUMP_KEYS, "Coredump")
                for path in paths[:MAX_CONFIG_FILES]
            )
        except FileNotFoundError:
            directories.append({"path": directory, "exists": False})
        except OSError as error:
            directories.append({"path": directory, "error": str(error)})
    report["config"] = {
        "apport": project_config("/etc/default/apport", {"enabled"}),
        "systemd_coredump": configs,
        "drop_in_directories": directories,
    }
    report["interpretation_limits"] = [
        "These are prerequisites observed in this process and runner, not evidence that a core was retained.",
        "The shell readings are raw bash ulimit output; Python reports the inherited limits in bytes.",
        "Only the inspecting process's namespace identifiers and coredump filter are read.",
        "Config entries are selected uncommented assignments, not a calculation of effective daemon configuration.",
        "The libc and loader hashes identify the selected filesystem paths, not another process's loaded mappings.",
        "Local context observations describe the recorded local hostname, not a Depot runner.",
    ]
    source_valid = revision_matches and (
        arguments.context == "local"
        or (tracked_clean and tracked.get("exit_code") == 0)
    )
    report["source_valid"] = source_valid
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        f"Wrote prerequisite observations to {arguments.output}; source_valid={source_valid}"
    )
    return 0 if source_valid else 1


if __name__ == "__main__":
    raise SystemExit(main())
