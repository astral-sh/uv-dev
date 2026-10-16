"""Capture eight sequential, balanced managed-Python download route trials."""

import argparse
import datetime
import hashlib
import json
import os
import platform
import re
import socket
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

from windows_process_job import run_process_tree

BASE_REVISION = "8ebc08a577779a73c96816d5af1a5658c2aad1ee"
DOWNLOAD_KEY = "cpython-3.15.0-windows-x86_64-none"
BUILD = "20261013"
EXPECTED_ARCHIVE_SHA256 = (
    "743d2269e74a98f7c4281dbc2e8710c171be1965693720aac818ff0157e4d965"
)
DEADLINE_SECONDS = 120
ROUTES = {
    "A": "https://releases.astral.sh/github/python-build-standalone/releases/download",
    "B": "https://github.com/astral-sh/python-build-standalone/releases/download",
}
SEQUENCES = {1: "ABBABAAB", 2: "BAABABBA"}
HTTP_SCHEME = "".join(
    rf"(?:{character}|\\+u{ord(character):04x}|\\+u{ord(character.upper()):04x})"
    for character in "http"
)
HTTP_URL = re.compile(
    HTTP_SCHEME
    + r"(?:s|\\+u0073|\\+u0053)?(?::|\\+u003a)(?:\\*/|\\+u002f){2}[^\s<>\"`]+",
    re.IGNORECASE,
)
SOURCE_FILES = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".github/workflows/ci.yml",
    ".github/workflows/test.yml",
    "scripts/python-route-capture.py",
    "scripts/windows_process_job.py",
    "crates/uv-python-managed/download-metadata.json",
    "crates/uv-python-managed/src/downloads.rs",
    "crates/uv-python-managed/src/downloads/copy_diagnostics.rs",
]


def timestamp():
    return datetime.datetime.now(datetime.UTC).isoformat()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    pending = path.with_suffix(path.suffix + ".partial")
    pending.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    pending.replace(path)


def git(repository, *arguments):
    return subprocess.check_output(
        ["git", "-C", str(repository), *arguments], text=True
    ).rstrip("\r\n")


def archive_files(directory, expected_path):
    return [
        {
            "path": str(path.relative_to(directory)),
            "published_archive": path == expected_path,
            "size_bytes": path.stat().st_size,
            "sha256": digest(path),
        }
        for path in sorted(directory.rglob("*"))
        if path.is_file()
    ]


def trace_observations(directory):
    observations = {
        "managed_download_send_events": 0,
        "response_identities": [],
        "copy_observations": [],
        "parse_errors": [],
    }
    for path in sorted(directory.glob("*.log")):
        for line_number, line in enumerate(
            path.read_text(errors="replace").splitlines(), 1
        ):
            if "Sending Python download request to `" in line:
                observations["managed_download_send_events"] += 1
            if "Python download response identity" in line:
                try:
                    identity = json.loads(line.split("identity=", 1)[1])
                    observations["response_identities"].append(
                        {"trace_file": path.name, "line": line_number, **identity}
                    )
                except (ValueError, IndexError) as error:
                    observations["parse_errors"].append(
                        {
                            "trace_file": path.name,
                            "line": line_number,
                            "error": str(error),
                        }
                    )
            if "Python archive copy observation" in line:
                observations["copy_observations"].append(
                    {"trace_file": path.name, "line": line_number, "raw": line}
                )
    return observations


def sanitize_log_text(text):
    def redact(match):
        token = match.group()
        suffix = ""
        while token and token[-1] in ".,;)}'\\":
            suffix = token[-1] + suffix
            token = token[:-1]
        escaped = "\\" in token
        try:
            if escaped:
                token = json.loads('"' + token + '"')
                if "\\" in token:
                    raise ValueError("Ambiguous URL escaping")
            parts = urlsplit(token)
            if parts.scheme.lower() not in {"http", "https"} or not parts.hostname:
                raise ValueError("Invalid HTTP URL")
            safe = urlunsplit(
                (parts.scheme, parts.netloc.rsplit("@", 1)[-1], parts.path, "", "")
            )
            if escaped:
                safe = json.dumps(safe)[1:-1]
            return safe + suffix
        except ValueError:
            return "[redacted-http-url]" + suffix

    return HTTP_URL.sub(redact, text)


def copy_sanitized_logs(raw_directory, evidence):
    files = []
    paths = [raw_directory / "stdout.log", raw_directory / "stderr.log"]
    paths.extend(sorted((raw_directory / "child-traces").glob("*.log")))
    for source in paths:
        if not source.is_file():
            continue
        relative = source.relative_to(raw_directory)
        destination = evidence / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(
            sanitize_log_text(source.read_text(errors="replace")), encoding="utf-8"
        )
        files.append({"path": str(relative), "sha256": digest(destination)})
    return files


def supervisor_check(output, workspace):
    directory = workspace / "supervisor-check"
    directory.mkdir()
    evidence = output / "supervisor-check"
    evidence.mkdir()
    marker = directory / "child-created.txt"
    child = "import time; time.sleep(30)"
    parent = (
        "import pathlib, subprocess, sys, time; "
        f"process = subprocess.Popen([sys.executable, '-c', {child!r}]); "
        f"pathlib.Path({str(marker)!r}).write_text(str(process.pid)); "
        "time.sleep(30)"
    )
    result = run_process_tree(
        [sys.executable, "-c", parent],
        dict(os.environ),
        directory,
        directory / "stdout.log",
        directory / "stderr.log",
        3,
    )
    result["child_created"] = marker.exists()
    result["sanitized_logs"] = copy_sanitized_logs(directory, evidence)
    if marker.exists():
        (evidence / "child-created.txt").write_text(
            marker.read_text(), encoding="utf-8"
        )
    write_json(evidence / "result.json", result)
    if not (
        result["timed_out"]
        and result["job_processes"] >= 2
        and result["tree_exit_confirmed"]
        and marker.exists()
    ):
        raise RuntimeError("Process-tree deadline self-check failed")
    return result


def trial_environment(workspace, traces, route):
    environment = dict(os.environ)
    for name in [
        "UV_PYTHON_DOWNLOADS_JSON_URL",
        "UV_PYTHON",
        "UV_ASTRAL_MIRROR_URL",
        "UV_REQUEST_TIMEOUT",
        "HTTP_TIMEOUT",
        "UV_NO_PROGRESS",
    ]:
        environment.pop(name, None)
    overrides = {
        "UV_NO_CONFIG": "1",
        "UV_OFFLINE": "0",
        "UV_HTTP_RETRIES": "0",
        "UV_HTTP_TIMEOUT": "30",
        "UV_HTTP_CONNECT_TIMEOUT": "10",
        "UV_PYTHON_CPYTHON_BUILD": BUILD,
        "UV_PYTHON_DOWNLOADS": "automatic",
        "UV_PYTHON_INSTALL_MIRROR": ROUTES[route],
        "UV_PYTHON_CACHE_DIR": str(workspace / "archives"),
        "UV_PYTHON_INSTALL_DIR": str(workspace / "install"),
        "UV_CACHE_DIR": str(workspace / "cache"),
        "UV_PYTHON_BIN_DIR": str(workspace / "bin"),
        "UV_PYTHON_INSTALL_BIN": "0",
        "UV_PYTHON_INSTALL_REGISTRY": "0",
        "UV_COMPILE_BYTECODE": "0",
        "UV_LINK_MODE": "copy",
        "UV_INTERNAL__TEST_LOG_DIR": str(traces),
        "TMP": str(workspace / "tmp"),
        "TEMP": str(workspace / "tmp"),
        "RUST_LOG": "error",
        "RUST_BACKTRACE": "1",
    }
    environment.update(overrides)
    return environment, overrides


def run_trial(binary, output, workspace, index, route, archive_name):
    name = f"trial-{index:02d}-{route}"
    evidence = output / name
    evidence.mkdir()
    work = workspace / name
    work.mkdir()
    for directory in ["archives", "install", "cache", "bin", "tmp"]:
        (work / directory).mkdir()
    raw_logs = work / "raw-logs"
    raw_logs.mkdir()
    traces = raw_logs / "child-traces"
    traces.mkdir()
    environment, overrides = trial_environment(work, traces, route)
    arguments = [
        str(binary),
        "--no-config",
        "--color",
        "never",
        "python",
        "install",
        "--no-bin",
        "--no-registry",
        "--no-compile-bytecode",
        DOWNLOAD_KEY,
    ]
    result = {
        "schema_version": 1,
        "trial": index,
        "route": route,
        "requested_mirror": ROUTES[route],
        "started_at": timestamp(),
        "outcome": "running",
        "command": arguments,
        "environment_overrides": overrides,
        "workspace": str(work),
        "deadline_seconds": DEADLINE_SECONDS,
        "command_attempts": 1,
        "archive_cache_empty_before_start": not any((work / "archives").iterdir()),
        "install_directory_empty_before_start": not any((work / "install").iterdir()),
    }
    write_json(evidence / "result.json", result)
    process_started = time.monotonic()
    try:
        process = run_process_tree(
            arguments,
            environment,
            work,
            raw_logs / "stdout.log",
            raw_logs / "stderr.log",
            DEADLINE_SECONDS,
        )
        result.update(process)
        result["outcome"] = (
            "timeout"
            if process["timed_out"]
            else "success"
            if process["exit_code"] == 0
            else "error"
        )
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        result.update(
            {
                "outcome": "supervisor_error",
                "error": str(error),
                "tree_exit_confirmed": False,
                "elapsed_seconds": time.monotonic() - process_started,
                "timed_out": None,
                "exit_code": None,
            }
        )
    result["finished_at"] = timestamp()
    expected_path = work / "archives" / archive_name
    result["archive_files"] = []
    if result["tree_exit_confirmed"]:
        try:
            result["archive_files"] = archive_files(work / "archives", expected_path)
        except OSError as error:
            result["archive_evidence_error"] = str(error)
    result["expected_archive_sha256"] = EXPECTED_ARCHIVE_SHA256
    published = [
        entry for entry in result["archive_files"] if entry["published_archive"]
    ]
    result["archive_digest_verified"] = (
        len(published) == 1 and published[0]["sha256"] == EXPECTED_ARCHIVE_SHA256
    )
    result["sanitized_logs"] = []
    if result["tree_exit_confirmed"]:
        result["sanitized_logs"] = copy_sanitized_logs(raw_logs, evidence)
    result["traces"] = trace_observations(evidence / "child-traces")
    if result["outcome"] == "success" and (
        not result["archive_digest_verified"]
        or result["traces"]["managed_download_send_events"] != 1
        or len(result["traces"]["response_identities"]) != 1
        or result["traces"]["parse_errors"]
    ):
        result["outcome"] = "evidence_error"
    write_json(evidence / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--runner-index", type=int, choices=[1, 2], required=True)
    parser.add_argument("--sequence", choices=list(SEQUENCES.values()), required=True)
    parser.add_argument("--expected-revision", required=True)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("This capture requires Windows")
    if args.sequence != SEQUENCES[args.runner_index]:
        parser.error("The sequence does not match this runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_revision):
        parser.error("Expected revision must be a full commit SHA")
    repository = Path(__file__).resolve().parent.parent
    binary = args.uv.resolve()
    output = args.output.resolve()
    workspace = args.workspace.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "manifest.json").exists():
        parser.error("The output directory already contains a capture manifest")
    workspace.mkdir(parents=True)
    manifest = {
        "schema_version": 1,
        "status": "preparing",
        "started_at": timestamp(),
        "base_revision": BASE_REVISION,
        "expected_revision": args.expected_revision,
        "runner_index": args.runner_index,
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
        "build_profile": "fast-build-nightly",
        "build_default_features": False,
        "build_features": [
            "test-python",
            "test-pypi",
            "test-python-managed",
            "test-windows-registry",
            "native-auth",
            "windows-native",
        ],
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "python_version": sys.version,
        "sequence": list(args.sequence),
        "trial_count": 8,
        "binary": str(binary),
        "trials": [],
        "proxy_variables_present": {
            name: bool(os.environ.get(name))
            for name in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "NO_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
                "no_proxy",
            ]
        },
    }
    write_json(output / "manifest.json", manifest)
    try:
        revision = git(repository, "rev-parse", "HEAD")
        tracked_status = git(
            repository, "status", "--porcelain", "--untracked-files=no"
        )
        manifest.update(
            {
                "revision": revision,
                "tracked_status": tracked_status,
                "source_sha256": {
                    name: digest(repository / name) for name in SOURCE_FILES
                },
                "checkout_preparation": json.loads(
                    (output / "checkout.json").read_text(encoding="utf-8")
                ),
            }
        )
        write_json(output / "manifest.json", manifest)
        preparation = manifest["checkout_preparation"]
        if (
            preparation["expected_revision"] != args.expected_revision
            or preparation["after_reset"]["revision"] != args.expected_revision
            or preparation["after_reset"]["tracked_status"]
        ):
            raise RuntimeError("Pre-build checkout verification is missing or invalid")
        if revision != args.expected_revision:
            raise RuntimeError("Post-build checkout has the wrong revision")
        if tracked_status:
            raise RuntimeError("Post-build checkout has tracked changes")
        metadata = json.loads(
            (repository / "crates/uv-python-managed/download-metadata.json").read_text()
        )[DOWNLOAD_KEY]
        if metadata["build"] != BUILD or metadata["sha256"] != EXPECTED_ARCHIVE_SHA256:
            raise RuntimeError(
                "Pinned download metadata differs from the capture specification"
            )
        filename = urlsplit(metadata["url"]).path.rsplit("/", 1)[1].replace("%2B", "-")
        archive_name = f"{EXPECTED_ARCHIVE_SHA256[:9]}-{filename}"
        manifest.update(
            {
                "binary_sha256": digest(binary),
                "binary_size_bytes": binary.stat().st_size,
                "binary_version": subprocess.check_output(
                    [str(binary), "--version"], text=True, timeout=10
                ).strip(),
                "download_key": DOWNLOAD_KEY,
                "build": BUILD,
                "canonical_url": metadata["url"],
                "expected_archive_sha256": EXPECTED_ARCHIVE_SHA256,
                "archive_name": archive_name,
                "supervisor_check": supervisor_check(output, workspace),
                "status": "running",
            }
        )
        write_json(output / "manifest.json", manifest)
        for index, route in enumerate(args.sequence, 1):
            result = run_trial(binary, output, workspace, index, route, archive_name)
            manifest["trials"].append(result)
            write_json(output / "manifest.json", manifest)
            if not result["tree_exit_confirmed"]:
                raise RuntimeError(
                    "Process-tree exit was not confirmed; remaining trials were not started"
                )
        manifest["status"] = "completed"
    except (
        OSError,
        RuntimeError,
        ValueError,
        KeyError,
        subprocess.SubprocessError,
    ) as error:
        manifest.update({"status": "harness_error", "error": str(error)})
    manifest["finished_at"] = timestamp()
    write_json(output / "manifest.json", manifest)
    (output / "manifest.sha256").write_text(
        digest(output / "manifest.json") + "  manifest.json\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "status": manifest["status"],
                "runner_index": args.runner_index,
                "outcomes": [trial["outcome"] for trial in manifest["trials"]],
            },
            indent=2,
        )
    )
    return int(
        manifest["status"] != "completed"
        or any(trial["outcome"] != "success" for trial in manifest["trials"])
    )


if __name__ == "__main__":
    raise SystemExit(main())
