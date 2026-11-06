"""Run one root excluded from the pinned harness, with the same lock command."""

import csv
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from uv_ecosystem_testing import Mode
from uv_ecosystem_testing.resolve import prepare_uv_command


def cgroup_events(filename):
    return {
        key: int(value)
        for key, value in (
            line.split()
            for line in (Path("/sys/fs/cgroup") / filename).read_text().splitlines()
        )
    }


def main():
    name = sys.argv[1]
    if not Path("/.dockerenv").exists() or not os.environ.get("UV_EXCLUDE_NEWER"):
        raise ValueError("run in Docker with an explicit cutoff")
    with Path("/harness/input.csv").open() as file:
        (row,) = csv.DictReader(file)
    if row["project"] != name or Path(name).name != name:
        raise ValueError("the input must contain exactly the requested root")

    project = Path("/out/corpus") / name
    project.mkdir(parents=True)
    command = prepare_uv_command(
        name,
        Path("/bin/uv-with-time-limit"),
        Mode.LOCK,
        Path("/cache"),
        False,
        project,
        "3.13",
        i_am_in_docker=True,
    )
    memory = cgroup_events("memory.events")
    pids = cgroup_events("pids.events")
    start = time.monotonic()
    # Stream diagnostics to disk so a container-wide OOM still leaves evidence.
    with (
        (project / "stdout.txt").open("wb") as stdout,
        (project / "stderr.txt").open("wb") as stderr,
        subprocess.Popen(
            command, cwd=project, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr
        ) as process,
    ):
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
    summary = {
        "package": name,
        "exit_code": status,
        "max_rss": usage.ru_maxrss,
        "time": time.monotonic() - start,
        "oom_kills": cgroup_events("memory.events")["oom_kill"] - memory["oom_kill"],
        "pids_limit_hits": cgroup_events("pids.events")["max"] - pids["max"],
    }
    (project / "summary.json").write_text(json.dumps(summary) + "\n")
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
