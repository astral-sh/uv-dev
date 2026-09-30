"""Record compiler termination and Linux memory pressure without changing build flags."""

from __future__ import annotations

import argparse
import json
import os
import re
import resource
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path


def read_text(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def memory_snapshot() -> dict[str, object]:
    paths = [Path("/sys/fs/cgroup")]
    membership = read_text(Path("/proc/self/cgroup"))
    for line in (membership or "").splitlines():
        if line.startswith("0::"):
            paths.insert(0, paths[-1] / line.removeprefix("0::").lstrip("/"))
    cgroups = {}
    for path in dict.fromkeys(paths):
        values = {
            name: value
            for name in (
                "memory.current",
                "memory.peak",
                "memory.max",
                "memory.high",
                "memory.swap.current",
                "memory.swap.max",
                "memory.events",
                "memory.events.local",
                "memory.pressure",
                "cpuset.cpus.effective",
            )
            if (value := read_text(path / name)) is not None
        }
        if values:
            cgroups[str(path)] = values
    # These files cover hosts still using the v1 memory controller.
    legacy = {
        name: value
        for name in (
            "memory.limit_in_bytes",
            "memory.max_usage_in_bytes",
            "memory.failcnt",
            "memory.oom_control",
        )
        if (value := read_text(Path("/sys/fs/cgroup/memory") / name)) is not None
    }
    if legacy:
        cgroups["/sys/fs/cgroup/memory"] = legacy
    meminfo = {
        line.split(":", 1)[0]: line.split(":", 1)[1].strip()
        for line in (read_text(Path("/proc/meminfo")) or "").splitlines()
        if line.split(":", 1)[0]
        in {"MemTotal", "MemAvailable", "SwapTotal", "SwapFree", "Dirty", "Writeback"}
    }
    vmstat = {
        name: int(value)
        for name, value in (
            line.split()
            for line in (read_text(Path("/proc/vmstat")) or "").splitlines()
        )
        if name in {"oom_kill", "pgmajfault", "pswpin", "pswpout"}
    }
    return {
        "time": time.time(),
        "kernel_boot_id": read_text(Path("/proc/sys/kernel/random/boot_id")),
        "meminfo": meminfo,
        "vmstat": vmstat,
        "cgroup_membership": membership,
        "cgroups": cgroups,
    }


def argument_value(arguments: list[str], name: str) -> str | None:
    for index, argument in enumerate(arguments):
        if argument == name and index + 1 < len(arguments):
            return arguments[index + 1]
        if argument.startswith(name + "="):
            return argument[len(name) + 1 :]
    return None


def exit_details(returncode: int) -> dict[str, object]:
    result: dict[str, object] = {"returncode": returncode}
    if returncode < 0:
        number = -returncode
        try:
            name = signal.Signals(number).name
        except ValueError:
            name = "unknown"
        result.update(signal=number, signal_name=name)
    return result


def exit_code(returncode: int) -> int:
    return 128 - returncode if returncode < 0 else returncode


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True) + "\n")


def rustc(arguments: list[str]) -> int:
    before = memory_snapshot()
    started = time.monotonic()
    crate = argument_value(arguments, "--crate-name")
    crate_type = argument_value(arguments, "--crate-type")
    if crate == "uv" and crate_type == "bin":
        print(
            "RUSTC_START_DIAGNOSTIC "
            + json.dumps(
                {
                    "pid": os.getpid(),
                    "crate": crate,
                    "crate_type": crate_type,
                    "before": before,
                },
                sort_keys=True,
            ),
            file=sys.stderr,
            flush=True,
        )
    completed = subprocess.run(
        [os.environ["UV_DIAGNOSTIC_REAL_RUSTC"], *arguments],
        check=False,
        # Cargo's jobserver descriptors must reach rustc to retain its concurrency.
        close_fds=False,
    )
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    record = {
        "pid": os.getpid(),
        "crate": crate,
        "crate_type": crate_type,
        "target": argument_value(arguments, "--target"),
        "emit": argument_value(arguments, "--emit"),
        "profile_generate": any(
            "profile-generate=" in argument for argument in arguments
        ),
        "profile_use": any("profile-use=" in argument for argument in arguments),
        "elapsed_seconds": time.monotonic() - started,
        "max_rss_bytes": int(
            usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024)
        ),
        "user_seconds": usage.ru_utime,
        "system_seconds": usage.ru_stime,
        "before": before,
        "after": memory_snapshot(),
        **exit_details(completed.returncode),
    }
    try:
        write_json(
            Path(os.environ["UV_DIAGNOSTIC_RESULTS"])
            / "rustc"
            / f"{os.getpid()}-{time.time_ns()}.json",
            record,
        )
    except OSError as error:
        print(
            f"rustc diagnostics could not write record: {error}",
            file=sys.stderr,
            flush=True,
        )
    if completed.returncode != 0 or (
        record["crate"] == "uv" and record["crate_type"] == "bin"
    ):
        print(
            "RUSTC_DIAGNOSTIC " + json.dumps(record, sort_keys=True),
            file=sys.stderr,
            flush=True,
        )
    # The original signal is recorded above. Return a nonzero shell-compatible status so
    # cargo-auditable cannot replace it with its generic signal-handling panic.
    return exit_code(completed.returncode)


def compiler_report(root: Path) -> dict[str, object]:
    records = [
        json.loads(path.read_text()) for path in sorted((root / "rustc").glob("*.json"))
    ]
    return {
        "invocations": len(records),
        "failures": [record for record in records if record["returncode"] != 0],
        "largest_compilers": sorted(
            records, key=lambda record: record["max_rss_bytes"], reverse=True
        )[:5],
    }


def observe_build(label: str, command: list[str]) -> int:
    root = Path(os.environ["UV_DIAGNOSTIC_RESULTS"])
    root.mkdir(parents=True, exist_ok=True)
    before = memory_snapshot()
    started = time.monotonic()
    next_report = started
    print(
        "BUILD_START_DIAGNOSTIC "
        + json.dumps({"label": label, "before": before}, sort_keys=True),
        flush=True,
    )
    with (root / f"{label}-memory.jsonl").open("w") as samples:
        process = subprocess.Popen(command, close_fds=False)
        while True:
            snapshot = memory_snapshot()
            samples.write(json.dumps(snapshot, sort_keys=True) + "\n")
            samples.flush()
            if time.monotonic() >= next_report:
                # BuildKit can disappear before files in a failed layer are exported.
                print(
                    "MEMORY_DIAGNOSTIC "
                    + json.dumps(
                        {"label": label, "snapshot": snapshot}, sort_keys=True
                    ),
                    flush=True,
                )
                next_report = time.monotonic() + 15
            try:
                returncode = process.wait(timeout=2)
                break
            except subprocess.TimeoutExpired:
                continue
    record = {
        "label": label,
        "elapsed_seconds": time.monotonic() - started,
        "before": before,
        "after": memory_snapshot(),
        **exit_details(returncode),
    }
    write_json(root / f"{label}-result.json", record)
    print("BUILD_DIAGNOSTIC " + json.dumps(record, sort_keys=True), flush=True)
    print(
        "COMPILER_REPORT " + json.dumps(compiler_report(root), sort_keys=True),
        flush=True,
    )
    return exit_code(returncode)


def monitor(output: Path, stop: Path) -> int:
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w") as stream:
        while not stop.exists():
            stream.write(json.dumps(memory_snapshot(), sort_keys=True) + "\n")
            stream.flush()
            time.sleep(2)
    return 0


def instrument_dockerfile(
    source: Path, scripts: Path, uv_digest: str, build_id: str | None = None
) -> int:
    if re.fullmatch(r"[0-9a-f]{64}", uv_digest) is None:
        raise ValueError("Expected the recorded uv image SHA-256 digest")
    copied = source / ".ci-2124"
    copied.mkdir()
    if build_id is not None:
        # Distinct inputs prevent BuildKit from caching or deduplicating the compilation.
        (copied / "build-id").write_text(build_id + "\n")
    for name in ("arm64-rustc-diagnostics.py", "arm64-rustc-wrapper.sh"):
        shutil.copy2(scripts / name, copied / name)
    (copied / "arm64-rustc-wrapper.sh").chmod(0o755)
    dockerfile = source / "Dockerfile"
    contents = dockerfile.read_text()
    bootstrap = "COPY --from=ghcr.io/astral-sh/uv:latest "
    if contents.count(bootstrap) != 1:
        raise ValueError("Expected exactly one uv bootstrap image")
    contents = contents.replace(
        bootstrap, f"COPY --from=ghcr.io/astral-sh/uv@sha256:{uv_digest} "
    )
    original = """RUN case "${TARGETPLATFORM}" in \\
  "linux/arm64") export JEMALLOC_SYS_WITH_LG_PAGE=16;; \\
  esac && \\
  cargo auditable zigbuild --bin uv --bin uvx --target $(cat rust_target.txt) --release"""
    replacement = """COPY .ci-2124 /root/.ci-2124
RUN case "${TARGETPLATFORM}" in \\
  "linux/arm64") export JEMALLOC_SYS_WITH_LG_PAGE=16;; \\
  esac && \\
  mkdir -p /root/code/tmp/uv-ci-2124 && \\
  export TMPDIR=/root/code/tmp/uv-ci-2124 && \\
  export UV_DIAGNOSTIC_UV=/usr/local/bin/uv \\
    UV_DIAGNOSTIC_PYTHON=/root/.venv/bin/python \\
    UV_DIAGNOSTIC_SCRIPT=/root/.ci-2124/arm64-rustc-diagnostics.py \\
    UV_DIAGNOSTIC_RESULTS=/root/.ci-2124/results \\
    UV_DIAGNOSTIC_REAL_RUSTC="$(rustup which rustc)" \\
    RUSTC=/root/.ci-2124/arm64-rustc-wrapper.sh && \\
  "$UV_DIAGNOSTIC_UV" run --no-config --no-project --no-python-downloads \\
    --python "$UV_DIAGNOSTIC_PYTHON" "$UV_DIAGNOSTIC_SCRIPT" build --label docker -- \\
    cargo auditable zigbuild --bin uv --bin uvx --target $(cat rust_target.txt) --release"""
    if contents.count(original) != 1:
        raise ValueError("Expected exactly one production Docker build command")
    dockerfile.write_text(contents.replace(original, replacement))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    compiler = commands.add_parser("rustc")
    compiler.add_argument("arguments", nargs=argparse.REMAINDER)
    build = commands.add_parser("build")
    build.add_argument("--label", required=True)
    build.add_argument("arguments", nargs=argparse.REMAINDER)
    watcher = commands.add_parser("monitor")
    watcher.add_argument("output", type=Path)
    watcher.add_argument("stop", type=Path)
    commands.add_parser("snapshot")
    report = commands.add_parser("report")
    report.add_argument("root", type=Path)
    dockerfile = commands.add_parser("dockerfile")
    dockerfile.add_argument("source", type=Path)
    dockerfile.add_argument("scripts", type=Path)
    dockerfile.add_argument("--uv-digest", required=True)
    dockerfile.add_argument("--build-id")
    args = parser.parse_args()
    match args.command:
        case "rustc":
            return rustc(
                args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
            )
        case "build":
            return observe_build(
                args.label,
                args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments,
            )
        case "monitor":
            return monitor(args.output, args.stop)
        case "snapshot":
            print(json.dumps(memory_snapshot(), sort_keys=True))
        case "report":
            print(json.dumps(compiler_report(args.root), sort_keys=True))
        case "dockerfile":
            return instrument_dockerfile(
                args.source, args.scripts, args.uv_digest, args.build_id
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
