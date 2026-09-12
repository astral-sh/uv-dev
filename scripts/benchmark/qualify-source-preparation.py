"""Qualify source-preparation breadth, recursion, and recovery with real local builds."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

BACKEND = r"""
import base64
import csv
import hashlib
import io
import json
import os
import time
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).parent
PROJECT = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]
NAME = PROJECT["name"].replace("-", "_")
VERSION = PROJECT.get("version", "0.1.0")
DIST_INFO = f"{NAME}-{VERSION}.dist-info"
METADATA = f"Metadata-Version: 2.3\nName: {PROJECT['name']}\nVersion: {VERSION}\nRequires-Python: >=3.11\n".encode()
WHEEL = b"Wheel-Version: 1.0\nGenerator: source-fd-qualification\nRoot-Is-Purelib: true\nTag: py3-none-any\n"

def event(phase, kind):
    value = json.dumps({"phase": phase, "kind": kind, "name": NAME, "pid": os.getpid(), "time_ns": time.monotonic_ns()}) + "\n"
    descriptor = os.open(os.environ["SOURCE_FD_LOG"], os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(descriptor, value.encode())
    finally:
        os.close(descriptor)

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    directory = Path(metadata_directory) / DIST_INFO
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "METADATA").write_bytes(METADATA)
    (directory / "WHEEL").write_bytes(WHEEL)
    return DIST_INFO

prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel

def get_requires_for_build_wheel(config_settings=None):
    return []

get_requires_for_build_editable = get_requires_for_build_wheel

def build(wheel_directory, editable):
    kind = "editable" if editable else "wheel"
    event("start", kind)
    try:
        if marker := os.environ.get("SOURCE_FD_FAIL_ONCE"):
            try:
                descriptor = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            except FileExistsError:
                pass
            else:
                os.close(descriptor)
                raise RuntimeError("intentional source fixture failure")
        time.sleep(float(os.environ["SOURCE_FD_BUILD_DELAY"]))
        files = {
            f"{DIST_INFO}/METADATA": METADATA,
            f"{DIST_INFO}/WHEEL": WHEEL,
        }
        if editable:
            files[f"{NAME}.pth"] = (ROOT.as_posix() + "\n").encode()
        else:
            files[f"{NAME}.py"] = (ROOT / f"{NAME}.py").read_bytes()
        record = io.StringIO()
        writer = csv.writer(record, lineterminator="\n")
        for name, data in files.items():
            digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
            writer.writerow([name, "sha256=" + digest, len(data)])
        writer.writerow([f"{DIST_INFO}/RECORD", "", ""])
        files[f"{DIST_INFO}/RECORD"] = record.getvalue().encode()
        filename = f"{NAME}-{VERSION}-py3-none-any.whl"
        with zipfile.ZipFile(Path(wheel_directory) / filename, "w", zipfile.ZIP_DEFLATED) as wheel:
            for name, data in files.items():
                wheel.writestr(name, data)
        return filename
    finally:
        event("end", kind)

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    return build(wheel_directory, False)

def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    return build(wheel_directory, True)
"""


def digest(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def descriptor_sample(
    pid: int, cache: Path, observer: Path | None
) -> dict[str, int] | None:
    if observer is not None:
        result = subprocess.run(
            [observer, str(pid), str(cache) + "/sdists-"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode == 1:
            return None
        if result.returncode != 0:
            raise RuntimeError(f"Descriptor observer failed: {result.stderr}")
        return json.loads(result.stdout)
    paths = {}
    if sys.platform.startswith("linux"):
        try:
            entries = list((Path("/proc") / str(pid) / "fd").iterdir())
        except FileNotFoundError:
            return None
        for entry in entries:
            try:
                paths[int(entry.name)] = os.readlink(entry)
            except FileNotFoundError:
                continue
    elif sys.platform == "darwin":
        result = subprocess.run(
            [
                "/usr/sbin/lsof",
                "-n",
                "-P",
                "-a",
                "-p",
                str(pid),
                "-d",
                "0-1048576",
                "-Ffn",
            ],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode not in {0, 1}:
            raise RuntimeError(result.stderr)
        descriptor = None
        for line in result.stdout.splitlines():
            if line.startswith("f"):
                descriptor = int(line[1:]) if line[1:].isdigit() else None
                if descriptor is not None:
                    paths[descriptor] = ""
            elif line.startswith("n") and descriptor is not None:
                paths[descriptor] = line[1:]
    else:
        raise RuntimeError(f"Descriptor sampling is unsupported on {sys.platform}")
    if not paths:
        return None
    sources = [
        path for path in paths.values() if path.startswith(str(cache) + "/sdists-")
    ]
    return {
        "descriptors": len(paths),
        "source_cache_descriptors": len(sources),
        "source_lock_descriptors": sum(
            path.removesuffix(" (deleted)").endswith("/.lock") for path in sources
        ),
    }


def write_package(
    path: Path,
    name: str,
    requirements: list[tuple[str, Path]],
    *,
    dynamic_version: bool = False,
) -> None:
    path.mkdir(parents=True)
    requires = [
        f"{dependency} @ {directory.as_uri()}" for dependency, directory in requirements
    ]
    version = 'dynamic = ["version"]' if dynamic_version else 'version = "0.1.0"'
    (path / "pyproject.toml").write_text(
        f'[project]\nname = "{name}"\n{version}\nrequires-python = ">=3.11"\n'
        f"[build-system]\nrequires = {json.dumps(requires)}\n"
        'build-backend = "backend"\nbackend-path = ["."]\n'
    )
    imports = "".join(
        f"import {dependency.replace('-', '_')}\n"
        f"assert {dependency.replace('-', '_')}.VALUE == 1000\n"
        for dependency, _ in requirements
    )
    (path / "backend.py").write_text(imports + BACKEND)
    (path / f"{name.replace('-', '_')}.py").write_text("VALUE = 1000\n")


def prepare_project(
    directory: Path, scenario: str, packages: int, depth: int, graphs: int
) -> tuple[Path, list[str], list[str]]:
    project = directory / "project"
    project.mkdir()
    (project / "pyproject.toml").write_text(
        '[project]\nname = "source-fd-root"\nversion = "0.1.0"\nrequires-python = ">=3.11"\n'
        '[tool.uv]\npackage = false\n[tool.uv.workspace]\nmembers = ["packages/*"]\n'
    )
    installed = []
    built = []
    if scenario == "wide":
        for number in range(packages):
            name = f"source-fd-{number:03}"
            write_package(project / "packages" / name, name, [])
            installed.append(name)
            built.append(name)
    elif scenario == "siblings":
        for graph in range(graphs):
            requirements = []
            for number in range(packages):
                name = f"source-fd-graph-{graph:03}-dependency-{number:03}"
                path = directory / "dependencies" / name
                write_package(path, name, [])
                requirements.append((name, path))
                built.append(name)
            name = f"source-fd-graph-{graph:03}"
            write_package(project / "packages" / name, name, requirements)
            installed.append(name)
            built.append(name)
    else:
        requirements = []
        if scenario in {"nested", "nested-wide", "unnamed-nested"}:
            for number in range(packages if scenario == "nested-wide" else 1):
                name = f"source-fd-dependency-{number:03}"
                path = directory / "dependencies" / name
                write_package(path, name, [])
                requirements.append((name, path))
                built.append(name)
        elif scenario == "nested-chain":
            for number in reversed(range(depth)):
                name = f"source-fd-dependency-{number:03}"
                path = directory / "dependencies" / name
                write_package(path, name, requirements)
                requirements = [(name, path)]
                built.append(name)
        elif scenario == "cycle":
            name = "source-fd-dependency-000"
            path = directory / "dependencies" / name
            write_package(
                path,
                name,
                [("source-fd-outer", project / "packages" / "source-fd-outer")],
            )
            requirements = [(name, path)]
            built.append(name)
        name = "source-fd-outer"
        write_package(
            project / "packages" / name,
            name,
            requirements,
            dynamic_version=scenario == "unnamed-nested",
        )
        installed.append(name)
        built.append(name)
    return project, installed, built


def read_events(path: Path) -> list[dict]:
    if not path.exists():
        return []
    # A backend appends each complete event in one write. Ignore an in-progress trailing record.
    return [
        json.loads(line)
        for line in path.read_text().splitlines(keepends=True)
        if line.endswith("\n")
    ]


def summarize_events(events: list[dict]) -> dict:
    active = peak = completed = 0
    for event in sorted(events, key=lambda event: event["time_ns"]):
        active += 1 if event["phase"] == "start" else -1
        peak = max(peak, active)
        completed += event["phase"] == "end"
    return {
        "started_builds": sum(event["phase"] == "start" for event in events),
        "completed_builds": completed,
        "peak_backend_builds": peak,
        "unfinished_builds": active,
    }


def stop_child(child: subprocess.Popen[str]) -> tuple[str, str]:
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return child.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        return child.communicate()


def run_child(
    args: argparse.Namespace,
    project: Path,
    cache: Path,
    environment: dict[str, str],
    events_path: Path,
    *,
    interrupt: bool = False,
) -> dict:
    command = [
        "sh",
        "-c",
        'ulimit -S -n "$1" && ulimit -H -n "$1" && shift && exec "$@"',
        "sh",
        str(args.fd_limit),
        str(args.uv),
        "--no-config",
        "--offline",
        "--no-progress",
        "--cache-dir",
        str(cache),
    ]
    if args.scenario == "unnamed-nested":
        command.extend(
            [
                "pip",
                "install",
                "--no-index",
                "--no-deps",
                "--python",
                str(args.python),
                "--target",
                str(project / "installed"),
                (project / "packages" / "source-fd-outer").as_uri(),
            ]
        )
    else:
        command.extend(
            [
                "sync",
                "--all-packages",
                "--no-default-groups",
                "--python",
                str(args.python),
            ]
        )
    previous_events = len(read_events(events_path))
    started = time.perf_counter()
    child = subprocess.Popen(
        command,
        cwd=project,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    samples = []
    sampling_errors = []
    stop_sampling = threading.Event()

    def observe() -> None:
        while not stop_sampling.is_set():
            try:
                sample = descriptor_sample(child.pid, cache, args.fd_observer)
                if sample is not None:
                    samples.append(
                        {"elapsed_seconds": time.perf_counter() - started, **sample}
                    )
            except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
                sampling_errors.append(str(error))
            stop_sampling.wait(args.sample_ms / 1000)

    observer = threading.Thread(target=observe, daemon=True)
    observer.start()
    timed_out = interrupted = False
    try:
        if interrupt:
            deadline = time.monotonic() + args.timeout
            while child.poll() is None and time.monotonic() < deadline:
                events = read_events(events_path)[previous_events:]
                if summarize_events(events)["unfinished_builds"] > 0:
                    interrupted = True
                    break
                time.sleep(0.005)
            timed_out = not interrupted and child.poll() is None
            stdout, stderr = stop_child(child)
        else:
            try:
                stdout, stderr = child.communicate(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                stdout, stderr = stop_child(child)
    finally:
        if child.poll() is None:
            stop_child(child)
        stop_sampling.set()
        observer.join(timeout=6)
    events = read_events(events_path)[previous_events:]
    return {
        "elapsed_seconds": time.perf_counter() - started,
        "exit_code": child.returncode,
        "timed_out": timed_out,
        "interrupted": interrupted,
        "stdout": stdout,
        "stderr": stderr,
        "backend_events": events,
        **summarize_events(events),
        "descriptor_samples": samples,
        "sampling_errors": sampling_errors,
        "observed_descriptor_peak": max(
            (sample["descriptors"] for sample in samples), default=None
        ),
        "observed_source_cache_peak": max(
            (sample["source_cache_descriptors"] for sample in samples), default=None
        ),
        "observed_source_lock_peak": max(
            (sample["source_lock_descriptors"] for sample in samples), default=None
        ),
    }


def verify_installed(
    args: argparse.Namespace, project: Path, environment: dict[str, str]
) -> list[str]:
    target = project / "installed" if args.scenario == "unnamed-nested" else None
    result = subprocess.run(
        [
            args.python if target else project / ".venv/bin/python",
            "-I",
            "-c",
            (
                "import importlib, importlib.metadata as metadata, json, sys\n"
                "if sys.argv[1]:\n"
                "    sys.path.insert(0, sys.argv[1])\n"
                "distributions = metadata.distributions(path=[sys.argv[1]]) if sys.argv[1] else metadata.distributions()\n"
                "names = sorted(distribution.metadata['Name'] for distribution in distributions "
                "if distribution.metadata['Name'].startswith('source-fd-'))\n"
                "for name in names:\n"
                "    assert importlib.import_module(name.replace('-', '_')).VALUE == 1000, name\n"
                "print(json.dumps(names))"
            ),
            str(target) if target else "",
        ],
        env=environment,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(result.stdout)


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument(
        "--work-directory", type=Path, default=root / ".cache/source-qualification"
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--scenario",
        choices=[
            "wide",
            "nested",
            "nested-wide",
            "nested-chain",
            "siblings",
            "unnamed-nested",
            "cycle",
        ],
        default="wide",
    )
    parser.add_argument("--packages", type=int, default=192)
    parser.add_argument("--depth", type=int, default=4)
    parser.add_argument("--graphs", type=int, default=2)
    parser.add_argument("--fd-limit", type=int, default=128)
    parser.add_argument("--builds", type=int, default=1)
    parser.add_argument("--downloads", type=int, default=50)
    parser.add_argument("--build-delay", type=float, default=0.05)
    parser.add_argument("--sample-ms", type=float, default=25)
    parser.add_argument("--fd-observer", type=Path)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument(
        "--fault", choices=["none", "failure", "interrupt"], default="none"
    )
    parser.add_argument(
        "--expect",
        choices=["success", "emfile", "timeout", "cycle", "either"],
        default="success",
    )
    args = parser.parse_args()
    if os.name != "posix" or args.fd_limit < 16:
        parser.error("a POSIX system and descriptor limit of at least 16 are required")
    if min(args.packages, args.depth, args.graphs, args.builds, args.downloads) < 1:
        parser.error(
            "package count, depth, graph count, builds, and downloads must be positive"
        )
    if args.sample_ms <= 0 or args.timeout <= 0 or args.build_delay < 0:
        parser.error(
            "sampling and timeout must be positive; build delay must be nonnegative"
        )
    if args.fault != "none" and args.expect != "success":
        parser.error("fault recovery requires --expect success")
    args.uv = args.uv.resolve(strict=True)
    args.python = args.python.resolve(strict=True)
    if args.fd_observer is not None:
        args.fd_observer = args.fd_observer.resolve(strict=True)
    elif not (sys.platform.startswith("linux") or sys.platform == "darwin"):
        parser.error("descriptor sampling requires Linux, macOS, or --fd-observer")
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    attempts = []
    report = {
        "success": False,
        "platform": sys.platform,
        "harness_sha256": digest(Path(__file__)),
        "uv": str(args.uv),
        "uv_sha256": digest(args.uv),
        "uv_version": subprocess.check_output(
            [args.uv, "--version"], text=True
        ).strip(),
        "python": str(args.python),
        "python_version": subprocess.check_output(
            [args.python, "-I", "--version"], text=True
        ).strip(),
        "scenario": args.scenario,
        "packages": args.packages,
        "depth": args.depth,
        "graphs": args.graphs,
        "fd_limit": args.fd_limit,
        "concurrent_builds": args.builds,
        "concurrent_downloads": args.downloads,
        "build_delay": args.build_delay,
        "timeout_seconds": args.timeout,
        "fault": args.fault,
        "expected_outcome": args.expect,
        "descriptor_sampling": "native-helper"
        if args.fd_observer
        else "procfs"
        if sys.platform.startswith("linux")
        else "lsof",
        "fd_observer": str(args.fd_observer) if args.fd_observer else None,
        "fd_observer_sha256": digest(args.fd_observer) if args.fd_observer else None,
        "fd_observer_source_sha256": digest(
            Path(__file__).with_name("observe-source-fds.c")
        ),
        "descriptor_sample_ms": args.sample_ms,
        "attempts": attempts,
    }
    try:
        with tempfile.TemporaryDirectory(
            dir=args.work_directory, prefix="sources-"
        ) as temporary:
            directory = Path(temporary)
            project, expected_installed, expected_built = prepare_project(
                directory, args.scenario, args.packages, args.depth, args.graphs
            )
            report["expected_installed"] = expected_installed
            report["expected_built"] = expected_built
            events_path = directory / "events.jsonl"
            cache = directory / "cache"
            environment = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith("UV_")
                and key
                not in {
                    "VIRTUAL_ENV",
                    "CONDA_PREFIX",
                    "PYTHONPATH",
                    "PYTHONHOME",
                    "RUST_LOG",
                    "SOURCE_FD_LOG",
                    "SOURCE_FD_BUILD_DELAY",
                    "SOURCE_FD_FAIL_ONCE",
                }
            }
            environment.update(
                UV_PYTHON_DOWNLOADS="never",
                UV_CONCURRENT_BUILDS=str(args.builds),
                UV_CONCURRENT_DOWNLOADS=str(args.downloads),
                SOURCE_FD_LOG=str(events_path),
                SOURCE_FD_BUILD_DELAY=str(args.build_delay),
            )
            if args.fault != "none":
                fault_environment = environment.copy()
                if args.fault == "failure":
                    fault_environment["SOURCE_FD_FAIL_ONCE"] = str(
                        directory / "failed-once"
                    )
                else:
                    fault_environment["SOURCE_FD_BUILD_DELAY"] = str(
                        max(10, args.build_delay)
                    )
                first = run_child(
                    args,
                    project,
                    cache,
                    fault_environment,
                    events_path,
                    interrupt=args.fault == "interrupt",
                )
                attempts.append(first)
                if args.fault == "failure":
                    if (
                        first["exit_code"] == 0
                        or "intentional source fixture failure" not in first["stderr"]
                    ):
                        raise AssertionError(
                            f"Backend failure was not observed: {first}"
                        )
                elif not first["interrupted"] or first["exit_code"] == 0:
                    raise AssertionError(
                        f"An active backend was not interrupted: {first}"
                    )
            result = run_child(args, project, cache, environment, events_path)
            attempts.append(result)
            outcome = (
                "timeout"
                if result["timed_out"]
                else "success"
                if result["exit_code"] == 0
                else "emfile"
                if "Too many open files" in result["stderr"]
                else "cycle"
                if "cyclic build dependency" in result["stderr"].lower()
                else "failure"
            )
            report["outcome"] = outcome
            if outcome == "success":
                installed = verify_installed(args, project, environment)
                report["installed"] = installed
                if installed != sorted(expected_installed):
                    raise AssertionError(f"Installed distributions differ: {installed}")
                completed = {
                    event["name"].replace("_", "-")
                    for attempt in attempts
                    for event in attempt["backend_events"]
                    if event["phase"] == "end"
                }
                if missing := set(expected_built).difference(completed):
                    raise AssertionError(
                        f"Build backends were not observed: {sorted(missing)}"
                    )
            if args.expect != "either" and outcome != args.expect:
                raise AssertionError(
                    f"Expected {args.expect}, observed {outcome}: {result['stderr']}"
                )
        report["success"] = True
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        output = json.dumps(report, indent=2) + "\n"
        if args.output is not None:
            args.output.write_text(output)
        else:
            print(output, end="")


if __name__ == "__main__":
    main()
