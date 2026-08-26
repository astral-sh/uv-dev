# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Measure a remote seed, a fresh-output-base replay, and uncached PEP 440 tests.

See bazel/README.md. Run with --dry-run to inspect commands without building.
Raw logs stay outside the checkout; summary.json contains only selected metrics.
"""

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]
HOSTS = ("remote.buildbuddy.io", "openai.buildbuddy.io")
BASE_REVISION = "a3343a269d6b5fe3289128d1030235bc5f905c0b"


def read_events(path):
    with path.open() as stream:
        for line in stream:
            if line.strip():
                yield json.loads(line)


def timestamp(event, field):
    value = event.get(field)
    if value is not None:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    return int(event[f"{field}Millis"]) / 1000


def summarize_bep(path):
    """Do not copy command lines, environments, workspace paths, or headers."""
    summary = {"runners": {}, "tests": []}
    start = finish = None
    for event in read_events(path):
        if "started" in event:
            started = event["started"]
            start = timestamp(started, "startTime")
            summary["bazel_version"] = started["buildToolVersion"]
        if "finished" in event:
            finished = event["finished"]
            finish = timestamp(finished, "finishTime")
            summary["success"] = (
                finished.get("overallSuccess", True)
                and finished["exitCode"].get("code", 0) == 0
            )
        if "buildMetrics" in event:
            action_summary = event["buildMetrics"].get("actionSummary", {})
            summary["runners"] = {
                runner["name"].replace(" ", "-"): int(runner.get("count", 0))
                for runner in action_summary.get("runnerCount", [])
            }
        if "testResult" in event:
            result = event["testResult"]
            execution = result.get("executionInfo", {})
            summary["tests"].append(
                {
                    "status": result.get("status"),
                    "strategy": execution.get("strategy"),
                    "cached_locally": result.get("cachedLocally", False),
                    "cached_remotely": result.get("cachedRemotely", False)
                    or execution.get("cachedRemotely", False),
                }
            )
    if start is None or finish is None or "success" not in summary:
        raise ValueError(
            "Incomplete build events; no successful measurement available."
        )
    summary["bazel_elapsed_seconds"] = round(finish - start, 3)
    return summary


def binary_from_bep(path, output_base):
    for event in read_events(path):
        for item in event.get("namedSetOfFiles", {}).get("files", []):
            uri = urlsplit(item.get("uri", ""))
            if uri.scheme == "bytestream" and item.get("name") == "crates/uv/uv":
                candidate = output_base / "execroot/_main"
                candidate = candidate.joinpath(
                    *item.get("pathPrefix", []), item["name"]
                )
            elif uri.scheme == "file" and uri.netloc in ("", "localhost"):
                candidate = Path(unquote(uri.path))
            else:
                continue
            if (
                candidate.as_posix().endswith("/bin/crates/uv/uv")
                and candidate.resolve().is_relative_to(output_base.resolve())
                and candidate.is_file()
            ):
                return candidate
    raise ValueError(
        "No downloaded uv binary in build events; inspect the local BEP file."
    )


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sanitized_rc():
    lines = []
    for line in (ROOT / ".bazelrc").read_text().splitlines():
        if line.strip() == "try-import %workspace%/user.bazelrc":
            continue
        if line.lstrip().startswith(("import ", "try-import ")):
            raise ValueError(
                "Unexpected Bazel rc import; review it before reproducing."
            )
        lines.append(line)
    return "\n".join(lines) + "\n"


def command(args, run_dir, phase):
    # Tests reuse the seed output base, as in the original experiment, but their
    # cached results are disabled. Seed and replay never share an output base.
    output_base = run_dir / ("seed" if phase == "tests" else phase)
    action = "fetch" if phase == "fetch" else "test" if phase == "tests" else "build"
    arguments = [
        args.bazel,
        "--nosystem_rc",
        "--nohome_rc",
        "--noworkspace_rc",
        f"--bazelrc={run_dir / 'experiment.bazelrc'}",
        "--bazelrc=/dev/null",
        f"--output_user_root={run_dir / 'bazel-root'}",
        f"--output_base={output_base}",
        action,
        "--config=remote-linux",
        "--lockfile_mode=error",
        "--disk_cache=",
        f"--repository_cache={run_dir / 'repository-cache'}",
        f"--repo_contents_cache={run_dir / 'repo-contents-cache'}",
        f"--remote_cache=grpcs://{args.host}",
        f"--remote_executor=grpcs://{args.host}",
        f"--remote_instance_name={args.instance}",
        f"--credential_helper={args.host}={ROOT / 'scripts/bazel/buildbuddy_credentials.py'}",
        "--bes_backend=",
        "--bes_results_url=",
        "--color=no",
        "--curses=no",
        # Keep locally downloaded output paths discoverable regardless of client OS.
        "--nobuild_event_json_file_path_conversion",
        f"--build_event_json_file={run_dir / (phase + '.bep.json')}",
    ]
    if phase == "tests":
        arguments.extend(
            ["--nocache_test_results", "--test_output=all", "//:pep440-tests"]
        )
    else:
        arguments.append("//:uv")
        if phase == "fetch":
            arguments.append("//:pep440-tests")
    return arguments


def verify_results(results):
    seed, replay, tests = (results[name] for name in ("seed", "replay", "tests"))
    errors = []
    if not all(result.get("success") for result in (seed, replay, tests)):
        errors.append("A Bazel build or test failed.")
    if seed["runners"].get("remote", 0) == 0:
        errors.append(
            "Seed executed no remote actions; use a new experiment namespace."
        )
    if seed["runners"].get("remote-cache-hit", 0) != 0:
        errors.append(
            "Seed had cache hits; this is not a cold action-cache measurement."
        )
    if replay["runners"].get("remote", 0) != 0:
        errors.append(
            "Replay executed remote actions; check Action Cache write permission."
        )
    if replay["runners"].get("remote-cache-hit", 0) != seed["runners"].get("remote", 0):
        errors.append("Replay cache hits do not match the seed's remote action count.")
    if not seed.get("binary_sha256") or seed.get("binary_sha256") != replay.get(
        "binary_sha256"
    ):
        errors.append("Seed and replay binary hashes differ or are missing.")
    if len(tests["tests"]) != 1 or not all(
        test["status"] == "PASSED"
        and test["strategy"] == "remote"
        and not test["cached_locally"]
        and not test["cached_remotely"]
        for test in tests["tests"]
    ):
        errors.append(
            "The PEP 440 test must pass once on a remote worker without cached results."
        )
    if tests.get("rust_tests_passed") != 67:
        errors.append("Expected 67 passing Rust tests in the test console log.")
    return errors


def run(args):
    if not re.fullmatch(r"uv-dev-experiment(?:/[A-Za-z0-9_-]+)+", args.instance):
        raise ValueError(
            "Use a fresh namespace under uv-dev-experiment/, e.g. uv-dev-experiment/my-run."
        )
    if args.run_dir.is_relative_to(ROOT) or ROOT.is_relative_to(args.run_dir):
        raise ValueError(
            "The run directory must be outside the checkout and not its parent."
        )
    if args.run_dir.exists():
        raise ValueError(
            "The run directory already exists; choose a new path for fresh output bases."
        )
    if args.key_file.resolve().is_relative_to(ROOT):
        raise ValueError("Keep the credential file outside the checkout.")
    config = sanitized_rc()
    commands = {
        phase: command(args, args.run_dir, phase)
        for phase in ("fetch", "seed", "replay", "tests")
    }
    if args.dry_run:
        for phase, arguments in commands.items():
            print(f"{phase}: {shlex.join(arguments)}")
        return
    if shutil.which(args.bazel) is None:
        raise ValueError("Bazelisk/Bazel is not installed or not on PATH.")
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True
    ).strip():
        raise ValueError(
            "Use a clean checkout so the recorded revision identifies every build input."
        )

    # Do not pass unrelated tokens or Bazel overrides into the build process.
    inherited = (
        "PATH",
        "HOME",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "UV_PYTHON_INSTALL_DIR",
    )
    environment = {name: os.environ[name] for name in inherited if name in os.environ}
    environment.update(
        UV_BAZEL_CREDENTIAL_HOST=args.host,
        UV_BAZEL_KEY_FILE=str(args.key_file),
        USE_BAZEL_VERSION=(ROOT / ".bazelversion").read_text().strip(),
        BAZELISK_SKIP_WRAPPER="1",
    )
    # Validate credentials using the helper without displaying or saving its stdout.
    check = subprocess.run(
        [sys.executable, str(ROOT / "scripts/bazel/buildbuddy_credentials.py"), "get"],
        input=json.dumps({"uri": f"grpcs://{args.host}"}),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        env=environment,
        timeout=10,
    )
    if check.returncode:
        raise ValueError(
            "Credential preflight failed. Check the host, key file, owner, and mode 0600."
        )

    args.run_dir.mkdir(mode=0o700, parents=True)
    (args.run_dir / ".gitignore").write_text("*\n")
    (args.run_dir / "experiment.bazelrc").write_text(config)
    results = {
        "schema_version": 1,
        "verified": False,
        "source_revision": revision,
        "original_base_revision": BASE_REVISION,
        "client": {"os": platform.system(), "architecture": platform.machine()},
        "date_utc": datetime.now(timezone.utc).isoformat(),
        "conditions": {
            "target": "//:uv",
            "test_target": "//:pep440-tests",
            "configuration": "remote-linux",
            "fresh_output_bases": True,
            "local_disk_cache": False,
            "repository_downloads": "prefetched before seed; reused by replay",
            "remote_instance": args.instance,
            "build_event_uploads": False,
        },
    }
    try:
        for phase, arguments in commands.items():
            print(f"{phase}: running (logs in {args.run_dir})", flush=True)
            log_path = args.run_dir / f"{phase}.console.log"
            started = time.monotonic()
            with log_path.open("w") as log:
                completed = subprocess.run(
                    arguments,
                    cwd=ROOT,
                    env=environment,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    timeout=3600,
                )
            wall_seconds = round(time.monotonic() - started, 3)
            if completed.returncode:
                raise ValueError(
                    f"Bazel {phase} failed; inspect {log_path}. Raw logs are private."
                )
            measurement = summarize_bep(args.run_dir / f"{phase}.bep.json")
            if measurement["bazel_version"] != environment["USE_BAZEL_VERSION"]:
                raise ValueError("The Bazel executable does not match .bazelversion.")
            measurement["process_wall_seconds"] = wall_seconds
            if phase in ("seed", "replay"):
                binary = binary_from_bep(
                    args.run_dir / f"{phase}.bep.json", args.run_dir / phase
                )
                measurement["binary_sha256"] = sha256(binary)
            if phase == "tests":
                counts = re.findall(
                    r"test result: ok\. (\d+) passed; 0 failed;", log_path.read_text()
                )
                measurement["rust_tests_passed"] = int(counts[-1]) if counts else None
            results[phase] = measurement
            print(
                f"{phase}: {measurement['bazel_elapsed_seconds']:.3f}s; {measurement['runners']}",
                flush=True,
            )
        errors = verify_results(results)
        results["verified"] = not errors
        results["validation_errors"] = errors
        if errors:
            raise ValueError(" ".join(errors))
        print(
            "Verified remote cache reuse, identical binaries, and 67 uncached remote tests."
        )
    finally:
        (args.run_dir / "summary.json").write_text(json.dumps(results, indent=2) + "\n")
        print(f"Selected metrics: {args.run_dir / 'summary.json'}")
        # These servers and output bases belong only to this run. Leave the logs
        # and caches for inspection, but do not leave three JVMs running.
        for phase in ("fetch", "seed", "replay"):
            if not (args.run_dir / phase).exists():
                continue
            arguments = commands[phase]
            startup = arguments[
                : arguments.index("fetch" if phase == "fetch" else "build")
            ]
            try:
                subprocess.run(
                    [*startup, "shutdown"],
                    cwd=ROOT,
                    env=environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=30,
                )
            except (OSError, subprocess.TimeoutExpired):
                print(
                    f"Could not stop the {phase} Bazel server; inspect the run directory.",
                    file=sys.stderr,
                )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", choices=HOSTS, required=True)
    parser.add_argument(
        "--key-file",
        type=lambda value: Path(value).expanduser().absolute(),
        required=True,
    )
    parser.add_argument(
        "--instance", default=f"uv-dev-experiment/repro-{uuid.uuid4().hex}"
    )
    parser.add_argument(
        "--run-dir",
        type=lambda value: Path(value).expanduser().resolve(),
        default=Path(tempfile.gettempdir()) / f"uv-bazel-repro-{uuid.uuid4().hex}",
    )
    parser.add_argument("--bazel", default="bazel")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print commands; do not read credentials, create files, or build.",
    )
    args = parser.parse_args()
    try:
        run(args)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
