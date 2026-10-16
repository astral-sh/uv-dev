"""Capture a fixed Windows interpreter-cache comparison without test reruns."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import sys
import time
from pathlib import Path

from windows_process_job import run_process_tree

VERSIONS = ("3.9.21", "3.12.9", "3.13.2")
SETUP_MIRROR = "https://github.com/astral-sh/python-build-standalone/releases/download"
SOURCES = {
    "baseline": "b757f15552203f42d8081b2378955e725cd732ce",
    "proposal": "9b7089394f4c3b00e0d1f0a8636588a6c32dbd6a",
}
TESTS = (
    ("project", "run::run_recreated_virtualenv_uses_current_interpreter"),
    ("sync", "sync::sync_script"),
    ("sync", "sync::sync_locked_script"),
    ("sync", "sync::sync_dry_run"),
)
DIRECT = (
    "import json, sys; print(json.dumps(dict("
    "python_full_version='.'.join(map(str,sys.version_info[:3])),"
    "sys_prefix=sys.prefix,sys_base_prefix=sys.base_prefix,"
    "sys_executable=sys.executable)))"
)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def files(root):
    return {
        path.relative_to(root).as_posix(): {
            "sha256": digest(path),
            "bytes": path.stat().st_size,
        }
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def identity(path):
    stat = path.stat()
    return {
        "path": str(path.resolve()),
        "sha256": digest(path),
        "mtime_ns": str(stat.st_mtime_ns),
        "bytes": stat.st_size,
    }


def command(arguments, cwd, evidence, environment, *, timeout=120, check=True):
    evidence.mkdir(parents=True, exist_ok=False)
    write_json(
        evidence / "command.json",
        {
            "arguments": [str(value) for value in arguments],
            "cwd": str(cwd),
            "timeout_seconds": timeout,
        },
    )
    result = run_process_tree(
        arguments,
        environment,
        cwd,
        evidence / "stdout.log",
        evidence / "stderr.log",
        timeout,
    )
    write_json(evidence / "result.json", result)
    stderr = (evidence / "stderr.log").read_text(errors="replace")
    if "INTERPRETER_CAPTURE_ERROR:" in stderr:
        raise RuntimeError(f"Incomplete interpreter trace: {evidence}")
    if not result["tree_exit_confirmed"] or result["active_processes_after_wait"]:
        raise RuntimeError("Process tree did not exit")
    if check and (result["timed_out"] or result["exit_code"]):
        raise RuntimeError(f"Command failed: {evidence}")
    return result


def query(probe, executable, cache, evidence, environment):
    environment = dict(environment)
    environment["UV_INTERNAL__INTERPRETER_CACHE_CAPTURE"] = str(evidence / "trace")
    command([probe, executable, cache], executable.parent, evidence, environment)
    result = json.loads((evidence / "stdout.log").read_text(encoding="utf-8"))
    records = [
        json.loads(path.read_text(encoding="utf-8"))
        for path in (evidence / "trace").glob("*.json")
    ]
    records.sort(key=lambda record: record["sequence"])
    if not records or not any(
        record["event"] in ("hit", "queried") for record in records
    ):
        raise RuntimeError("Query did not retain a cache decision")
    return {
        "metadata": result,
        "events": [record["event"] for record in records],
        "records": records,
    }


def selection(result):
    selected = [record for record in result["records"] if record["event"] == "selected"]
    if len(selected) != 1 or selected[0]["cache"] is None:
        raise RuntimeError("Direct query did not retain exactly one cache selection")
    return selected[0]


def require_hit(result):
    if "hit" not in result["events"] or "spawn" in result["events"]:
        raise RuntimeError("Unchanged identity did not produce a cache hit")


def direct(executable, evidence, environment):
    command(
        [executable, "-I", "-B", "-c", DIRECT], executable.parent, evidence, environment
    )
    return json.loads((evidence / "stdout.log").read_text(encoding="utf-8"))


def equivalent(first, second):
    keys = ("python_full_version", "sys_prefix", "sys_base_prefix", "sys_executable")
    return all(
        os.path.normcase(first[key]) == os.path.normcase(second[key]) for key in keys
    )


def recreate(uv, python, venv, evidence, environment, timestamp):
    command(
        [uv, "venv", "--clear", "--no-cache", "--python", python, venv],
        venv.parent,
        evidence,
        environment,
    )
    executable = venv / "Scripts/python.exe"
    os.utime(executable, ns=(timestamp, timestamp))
    return executable


def replacement_case(
    arm, uv, probe, pythons, workspace, evidence, environment, *, same_mtime
):
    workspace.mkdir(parents=True)
    venv = workspace / "venv"
    cache = workspace / "query-cache"
    timestamp = 1_700_000_000_000_000_000
    executable = recreate(
        uv, pythons["3.12.9"], venv, evidence / "create-initial", environment, timestamp
    )
    initial_configuration = (venv / "pyvenv.cfg").read_bytes()
    initial_identity = identity(executable)
    first = query(probe, executable, cache, evidence / "initial-query", environment)
    if (
        first["metadata"]["python_full_version"] != "3.12.9"
        or "spawn" not in first["events"]
    ):
        raise RuntimeError("Initial query did not identify Python 3.12.9")
    repeated = query(probe, executable, cache, evidence / "initial-hit", environment)
    require_hit(repeated)
    old_cache = files(cache)
    executable = recreate(
        uv,
        pythons["3.9.21"],
        venv,
        evidence / "recreate",
        environment,
        timestamp if same_mtime else timestamp + 10_000_000_000,
    )
    if old_cache != files(cache):
        raise RuntimeError("Environment recreation modified the query cache")
    replacement_identity = identity(executable)
    replacement_configuration = (venv / "pyvenv.cfg").read_bytes()
    if initial_configuration == replacement_configuration:
        raise RuntimeError("Recreation did not change pyvenv.cfg")
    if (initial_identity["mtime_ns"] == replacement_identity["mtime_ns"]) != same_mtime:
        raise RuntimeError("Launcher timestamps do not match the assigned condition")
    observed = query(
        probe, executable, cache, evidence / "replacement-query", environment
    )
    initial_selection, replacement_selection = selection(first), selection(observed)
    fresh = query(
        probe,
        executable,
        workspace / "fresh-query-cache",
        evidence / "fresh-query",
        environment,
    )
    oracle = direct(executable, evidence / "direct-python", environment)
    if fresh["metadata"]["python_full_version"] != "3.9.21" or not equivalent(
        fresh["metadata"], oracle
    ):
        raise RuntimeError("Fresh-cache query disagrees with direct Python")
    stale = not equivalent(observed["metadata"], oracle)
    if not same_mtime and stale:
        raise RuntimeError("Changed-mtime invalidation control returned stale metadata")
    expected = same_mtime and arm == "baseline"
    same_key = (
        initial_selection["cache"]["path"] == replacement_selection["cache"]["path"]
    )
    canonical_equal = (
        initial_selection["canonical"] == replacement_selection["canonical"]
    )
    timestamp_equal = (
        initial_selection["lookup_timestamp"]
        == replacement_selection["lookup_timestamp"]
    )
    predicted = (
        stale == expected
        and canonical_equal
        and timestamp_equal == same_mtime
        and same_key == (arm == "baseline")
        and (not expected or "hit" in observed["events"])
    )
    result = {
        "case": "equal_mtime_recreation" if same_mtime else "changed_mtime_control",
        "arm": arm,
        "complete": True,
        "stale": stale,
        "matches_predicted_mechanism": predicted,
        "initial_identity": initial_identity,
        "replacement_identity": replacement_identity,
        "initial_configuration_sha256": hashlib.sha256(
            initial_configuration
        ).hexdigest(),
        "replacement_configuration_sha256": hashlib.sha256(
            replacement_configuration
        ).hexdigest(),
        "cache_unchanged_during_recreation": True,
        "initial_cache_key": initial_selection["cache"]["path"],
        "replacement_cache_key": replacement_selection["cache"]["path"],
        "canonical_path_equal": canonical_equal,
        "lookup_timestamp_equal": timestamp_equal,
        "replacement_cache_events": observed["events"],
        "observed": observed["metadata"],
        "fresh": fresh["metadata"],
        "direct": oracle,
    }
    write_json(evidence / "case.json", result)
    return result


def unchanged_case(arm, uv, probe, python, workspace, evidence, environment):
    workspace.mkdir(parents=True)
    executable = recreate(
        uv,
        python,
        workspace / "venv",
        evidence / "create",
        environment,
        1_700_000_000_000_000_000,
    )
    cache = workspace / "cache"
    first = query(probe, executable, cache, evidence / "initial", environment)
    repeated = query(probe, executable, cache, evidence / "repeated", environment)
    require_hit(repeated)
    fresh = query(
        probe, executable, workspace / "fresh-cache", evidence / "fresh", environment
    )
    oracle = direct(executable, evidence / "direct-python", environment)
    if not all(
        equivalent(value["metadata"], oracle) for value in (first, repeated, fresh)
    ):
        raise RuntimeError("Unchanged identity control disagrees with its oracles")
    result = {
        "case": "unchanged_identity_control",
        "arm": arm,
        "complete": True,
        "cache_hit": True,
        "oracle_agreement": True,
    }
    write_json(evidence / "case.json", result)
    return result


def supervisor_check(workspace, output, environment):
    workspace.mkdir(parents=True)
    marker = workspace / "child-created.txt"
    child = "import time; time.sleep(30)"
    parent = (
        "import pathlib, subprocess, sys, time; "
        + f"process = subprocess.Popen([sys.executable, '-c', {child!r}]); "
        + f"pathlib.Path({str(marker)!r}).write_text(str(process.pid)); time.sleep(30)"
    )
    result = command(
        [sys.executable, "-c", parent],
        workspace,
        output,
        environment,
        timeout=3,
        check=False,
    )
    if not result["timed_out"] or result["job_processes"] < 2 or not marker.exists():
        raise RuntimeError("Process-tree supervisor control failed")
    shutil.copyfile(marker, output / marker.name)
    return result


def run(arguments):
    output, workspace, sources = (
        arguments.output.resolve(),
        arguments.workspace.resolve(),
        arguments.sources.resolve(),
    )
    output.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ)
    for name in (
        "PYTHONEXECUTABLE",
        "__PYVENV_LAUNCHER__",
        "PYTHONHOME",
        "PYTHONPATH",
        "VIRTUAL_ENV",
        "UV_PYTHON",
        "UV_COMPILE_BYTECODE",
        "UV_CONFIG_FILE",
        "UV_INTERNAL__INTERPRETER_CACHE_CAPTURE",
    ):
        environment.pop(name, None)
    environment.update(
        {
            "UV_HTTP_RETRIES": "0",
            "UV_OFFLINE": "0",
            "UV_NO_CONFIG": "1",
            "UV_PYTHON_DOWNLOADS": "automatic",
            "UV_PYTHON_INSTALL_DIR": str(workspace / "pythons"),
            "UV_PYTHON_INSTALL_MIRROR": SETUP_MIRROR,
            "UV_PYTHON_CACHE_DIR": str(workspace / "archives"),
            "UV_PYTHON_DOWNLOADS_JSON_URL": (
                sources / "proposal/crates/uv-python-managed/download-metadata.json"
            ).as_uri(),
            "UV_PYTHON_INSTALL_BIN": "0",
            "UV_PYTHON_INSTALL_REGISTRY": "0",
            "UV_LINK_MODE": "copy",
            "INSTA_UPDATE": "no",
            "RUST_BACKTRACE": "1",
        }
    )
    record = {
        "status": "started",
        "sources": SOURCES,
        "platform": platform.platform(),
        "windows_version": platform.win32_ver(),
        "runner_name": os.environ.get("RUNNER_NAME"),
        "runner_arch": os.environ.get("RUNNER_ARCH"),
        "cases": [],
        "setup_mirror": SETUP_MIRROR,
        "managed_python_downloads_after_setup": "never",
        "direct_probes_offline": True,
        "sync_cases": "Preinstalled Python with the existing PyPI behavior and fresh per-test caches",
    }
    write_json(output / "result.json", record)
    try:
        record["supervisor_control"] = supervisor_check(
            workspace / "supervisor", output / "supervisor", environment
        )
        command(
            [
                arguments.setup_uv.resolve(),
                "python",
                "install",
                "--no-bin",
                "--no-registry",
                "--install-dir",
                workspace / "pythons",
                *VERSIONS,
            ],
            workspace,
            output / "python-setup",
            environment,
            timeout=360,
        )
        pythons = {
            version: workspace
            / "pythons"
            / f"cpython-{version}-windows-x86_64-none/python.exe"
            for version in VERSIONS
        }
        catalog = json.loads(
            (
                sources / "proposal/crates/uv-python-managed/download-metadata.json"
            ).read_text(encoding="utf-8")
        )
        archives = files(workspace / "archives")
        selected = {}
        for version, python in pythons.items():
            key = f"cpython-{version}-windows-x86_64-none"
            entry = catalog[key]
            sha256 = entry["sha256"]
            matching = [
                name for name, value in archives.items() if value["sha256"] == sha256
            ]
            if len(matching) != 1:
                raise RuntimeError(
                    f"Archive bytes do not match the pinned catalog for {key}"
                )
            actual = direct(python, output / "python-identity" / version, environment)
            if actual["python_full_version"] != version:
                raise RuntimeError(
                    "Installed interpreter version does not match its catalog entry"
                )
            launcher_directory = python.parent / "Lib/venv/scripts/nt"
            launchers = {
                path.name: identity(path)
                for path in sorted(launcher_directory.glob("*.exe"))
            }
            if not launchers:
                raise RuntimeError("The installation has no Windows venv launchers")
            selected[version] = {
                "catalog": entry,
                "archive": matching[0],
                "python": identity(python),
                "launchers": launchers,
                "metadata": actual,
            }
        record["interpreters"] = selected
        environment["UV_PYTHON_DOWNLOADS"] = "never"
        offline_environment = {**environment, "UV_OFFLINE": "1"}
        for arm in SOURCES:
            source = sources / arm
            binaries = source / "target/fast-build-nightly"
            uv = binaries / "uv.exe"
            probe = binaries / "examples/interpreter-cache-probe.exe"
            record.setdefault("binaries", {})[arm] = {
                "uv": identity(uv),
                "probe": identity(probe),
            }
            root = output / arm
            scratch = workspace / arm
            record["cases"].append(
                replacement_case(
                    arm,
                    uv,
                    probe,
                    pythons,
                    scratch / "equal-mtime",
                    root / "equal-mtime",
                    offline_environment,
                    same_mtime=True,
                )
            )
            record["cases"].append(
                unchanged_case(
                    arm,
                    uv,
                    probe,
                    pythons["3.12.9"],
                    scratch / "unchanged",
                    root / "unchanged",
                    offline_environment,
                )
            )
            record["cases"].append(
                replacement_case(
                    arm,
                    uv,
                    probe,
                    pythons,
                    scratch / "changed-mtime",
                    root / "changed-mtime",
                    offline_environment,
                    same_mtime=False,
                )
            )
            for binary, test in TESTS:
                evidence = root / test.replace("::", "-")
                test_environment = dict(environment)
                test_environment["CARGO_TARGET_DIR"] = str(source / "target")
                test_environment["UV_INTERNAL__INTERPRETER_CACHE_CAPTURE"] = str(
                    evidence / "trace"
                )
                junit = source / "target/nextest/ci-windows/junit.xml"
                junit.unlink(missing_ok=True)
                result = command(
                    [
                        arguments.nextest.resolve(),
                        "nextest",
                        "run",
                        "--cargo-metadata",
                        source / "capture-cargo-metadata.json",
                        "--binaries-metadata",
                        source / "capture-binaries.json",
                        "--profile",
                        "ci-windows",
                        "--retries",
                        "0",
                        "--test-threads",
                        "1",
                        "--success-output",
                        "immediate",
                        "--failure-output",
                        "immediate",
                        "--no-tests",
                        "fail",
                        "-E",
                        f"binary(={binary}) & test(={test})",
                    ],
                    source,
                    evidence,
                    test_environment,
                    check=False,
                )
                if junit.exists():
                    shutil.copyfile(junit, evidence / "junit.xml")
                traces = list((evidence / "trace").glob("*.json"))
                record["cases"].append(
                    {
                        "case": test,
                        "arm": arm,
                        "complete": result["tree_exit_confirmed"],
                        "process": result,
                        "trace_records": len(traces),
                        "natural_timestamps": test.startswith("sync::"),
                    }
                )
                if not traces or not (evidence / "junit.xml").exists():
                    raise RuntimeError(
                        "Existing test did not retain trace and JUnit evidence"
                    )
                write_json(output / "result.json", record)
        if len(record["cases"]) != 14:
            raise RuntimeError("The fixed comparison did not produce 14 cases")
        record["status"] = "capture_complete"
    except Exception as error:
        record["status"] = "incomplete"
        record["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        record["finished_unix_seconds"] = time.time()
        write_json(output / "result.json", record)
        write_json(output / "evidence-sha256.json", files(output))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--setup-uv", type=Path, required=True)
    parser.add_argument("--nextest", type=Path, required=True)
    arguments = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("This fixed capture requires native Windows")
    run(arguments)


if __name__ == "__main__":
    main()
