#!/usr/bin/env python3
"""Compare synthetic uv build-hook canaries with and without Linux bubblewrap."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

BACKEND = """import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import zipfile

def probe(label):
    settings = json.loads(os.environ["RESEARCH_PROBES"])
    result = {"hook": label, "secret_visible": "RESEARCH_FAKE_SECRET" in os.environ}
    try:
        result["outside_read"] = Path(settings["read"]).read_text() == "synthetic readable canary"
    except OSError:
        result["outside_read"] = False
    for name in ("allowed", "outside"):
        try:
            Path(settings[name]).write_text("synthetic canary\\n")
            result[name + "_write"] = True
        except OSError:
            result[name + "_write"] = False
    try:
        with socket.create_connection(("127.0.0.1", settings["port"]), timeout=0.2):
            result["host_network"] = True
    except OSError:
        result["host_network"] = False
    with open(settings["log"], "a") as stream:
        stream.write(json.dumps(result) + "\\n")

def record(hook):
    probe(hook)
    subprocess.run([sys.executable, "-c", "import backend, sys; backend.probe('descendant:' + sys.argv[1])", hook], check=True)

METADATA = "Metadata-Version: 2.2\\nName: confined-fixture\\nVersion: 1.0\\n"

def get_requires_for_build_wheel(config_settings=None):
    record("get_requires_for_build_wheel")
    return []

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    record("prepare_metadata_for_build_wheel")
    target = Path(metadata_directory) / "confined_fixture-1.0.dist-info"
    target.mkdir()
    (target / "METADATA").write_text(METADATA)
    return target.name

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    record("build_wheel")
    filename = "confined_fixture-1.0-py3-none-any.whl"
    prefix = "confined_fixture-1.0.dist-info/"
    with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
        wheel.writestr(prefix + "METADATA", METADATA)
        wheel.writestr(prefix + "WHEEL", "Wheel-Version: 1.0\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n")
        wheel.writestr(prefix + "RECORD", "")
    return filename
"""


def sandbox_command(
    bwrap: str, writable: Path, environment: dict[str, str]
) -> list[str]:
    command = [
        bwrap,
        "--die-with-parent",
        "--new-session",
        "--unshare-net",
        "--unshare-pid",
        "--ro-bind",
        "/",
        "/",
        "--bind",
        str(writable),
        str(writable),
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--clearenv",
    ]
    for name, value in environment.items():
        command.extend(["--setenv", name, value])
    return command + ["--"]


def decoded_output(output: str | bytes | None) -> str:
    if isinstance(output, bytes):
        return output.decode(errors="replace")
    return output or ""


def observe(uv: str, workspace: Path, mode: str, port: int, bwrap: str | None) -> dict:
    writable = workspace / mode
    writable.mkdir()
    source = writable / "source"
    source.mkdir()
    (source / "backend.py").write_text(BACKEND)
    (source / "pyproject.toml").write_text(
        '[build-system]\nrequires = []\nbuild-backend = "backend"\nbackend-path = ["."]\n'
        '[project]\nname = "confined-fixture"\nversion = "1.0"\ndynamic = ["dependencies"]\n'
    )
    requirements = writable / "requirements.in"
    requirements.write_text(f"confined-fixture @ {source.as_uri()}\n")
    log = writable / "probes.jsonl"
    readable = workspace / f"readable-{mode}.txt"
    readable.write_text("synthetic readable canary")
    settings = {
        "read": str(readable),
        "allowed": str(writable / "allowed.txt"),
        "outside": str(workspace / f"outside-{mode}.txt"),
        "log": str(log),
        "port": port,
    }
    temporary = writable / "tmp"
    temporary.mkdir()
    environment = {
        "TMPDIR": str(temporary),
        "PATH": os.defpath,
        "HOME": str(writable),
        "LC_ALL": "C.UTF-8",
        "RESEARCH_PROBES": json.dumps(settings),
        "UV_PYTHON_DOWNLOADS": "never",
    }
    prefix = []
    if mode == "confined":
        if bwrap is None:
            raise RuntimeError("bubblewrap is required for confinement")
        prefix = sandbox_command(bwrap, writable, environment)
    environment["RESEARCH_FAKE_SECRET"] = "synthetic-value-only"
    commands = [
        [
            uv,
            "--no-config",
            "pip",
            "compile",
            str(requirements),
            "--no-index",
            "--python",
            sys.executable,
            "--cache-dir",
            str(writable / "metadata-cache"),
        ],
        [
            uv,
            "--no-config",
            "build",
            str(source),
            "--wheel",
            "--no-index",
            "--python",
            sys.executable,
            "--cache-dir",
            str(writable / "build-cache"),
            "--out-dir",
            str(writable / "dist"),
        ],
    ]
    runs = []
    probes = []
    for number, command in enumerate(commands):
        probe_start = len(probes)
        entered = writable / f"entered-{number}"
        launcher = [
            sys.executable,
            "-c",
            (
                "from pathlib import Path; import os, sys; "
                "Path(sys.argv[1]).write_text('started'); os.execv(sys.argv[2], sys.argv[2:])"
            ),
            str(entered),
        ]
        started = time.perf_counter()
        timed_out = False
        try:
            process = subprocess.run(
                prefix + launcher + command,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
                timeout=60,
            )
        except subprocess.TimeoutExpired as error:
            timed_out = True
            returncode = None
            stdout = decoded_output(error.stdout)
            stderr = decoded_output(error.stderr)
        else:
            returncode = process.returncode
            stdout = process.stdout
            stderr = process.stderr
        probes = (
            [json.loads(line) for line in log.read_text().splitlines()]
            if log.exists()
            else []
        )
        runs.append(
            {
                "command": command[2],
                "returncode": returncode,
                "timed_out": timed_out,
                "entered": entered.exists(),
                "probe_start": probe_start,
                "probe_count": len(probes) - probe_start,
                "seconds": time.perf_counter() - started,
                "stdout": stdout,
                "stderr": stderr,
            }
        )
        if timed_out or returncode:
            status = (
                "timed-out"
                if timed_out
                else "setup-failed"
                if not entered.exists()
                else "build-failed"
                if len(probes) > probe_start
                else "command-failed-before-hooks"
            )
            return {
                "mode": mode,
                "runs": runs,
                "probes": probes,
                "completed": False,
                "status": status,
            }
    errors = []
    expected_hooks = {
        "get_requires_for_build_wheel",
        "prepare_metadata_for_build_wheel",
        "build_wheel",
        "descendant:get_requires_for_build_wheel",
        "descendant:prepare_metadata_for_build_wheel",
        "descendant:build_wheel",
    }
    if {probe["hook"] for probe in probes} != expected_hooks:
        errors.append("Missing hook observations")
    for probe in probes:
        expected = mode == "baseline"
        if (
            not probe["allowed_write"]
            or not probe["outside_read"]
            or any(
                probe[key] != expected
                for key in ("outside_write", "host_network", "secret_visible")
            )
        ):
            errors.append(f"Unexpected {mode} canary result: {probe}")
    return {
        "mode": mode,
        "runs": runs,
        "probes": probes,
        "completed": not errors,
        "status": "validation-failed" if errors else "complete",
        "errors": errors,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument(
        "--baseline-only",
        action="store_true",
        help="Run the unconfined control; makes no confinement claim",
    )
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("This experiment requires Linux")
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    bwrap = shutil.which("bwrap")
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with (
        tempfile.TemporaryDirectory(
            prefix="build-confinement-", dir=args.work_dir
        ) as directory,
        socket.socket() as listener,
    ):
        listener.bind(("127.0.0.1", 0))
        listener.listen(64)
        workspace = Path(directory).resolve()
        baseline = observe(
            str(Path(uv).resolve()),
            workspace,
            "baseline",
            listener.getsockname()[1],
            bwrap,
        )
        with open(uv, "rb") as binary:
            uv_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
        report = {
            "uv_sha256": uv_sha256,
            "uv_executable": str(Path(uv).resolve()),
            "uv": subprocess.check_output([uv, "--version"], text=True).strip(),
            "python": sys.version,
            "observations": [baseline],
        }
        if not args.baseline_only:
            if bwrap is None:
                report["confinement_error"] = "bubblewrap executable not found"
            else:
                report["observations"].append(
                    observe(
                        str(Path(uv).resolve()),
                        workspace,
                        "confined",
                        listener.getsockname()[1],
                        bwrap,
                    )
                )
        print(json.dumps(report, indent=2))
        if report.get("confinement_error") or not all(
            row["completed"] for row in report["observations"]
        ):
            raise SystemExit(2)


if __name__ == "__main__":
    main()
