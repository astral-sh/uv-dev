"""Compare the existing local-fixture change against its original R2 readers."""

import json
import os
import subprocess
import tempfile
from pathlib import Path

from benchmark import CARGO, capture, measure

SOURCE = "54aab6e202c272790efa722f7040645ed86cfdf0"
ORIGINAL_FIXTURES = "371226bc1b7baa1b4d2aeab63e9000b39f56f969"
CONTROL = "feffc0b5bbf8aeca1dff4a6f042a58e44a5daf09"
AUTH_FIX = "51bcea71165dc26c1fd9ea6e6686ba413ae1f679"
PATCH = Path(__file__).with_name("r2-control.patch")
RESULTS = Path(os.environ["GITHUB_WORKSPACE"]) / "results"
SCRATCH = Path.home() / "code" / "tmp" / "uv-runner-rca"
REPLICA = int(os.environ["RCA_REPLICA"])
VARIANTS = ["r2", "local"] if REPLICA == 1 else ["local", "r2"]
SCHEDULES = [VARIANTS, VARIANTS[::-1], VARIANTS]


def verify(row, expected):
    if (
        row["returncode"]
        or row["tests_run"] != expected
        or row["tests_passed"] != expected
    ):
        raise RuntimeError(f"Incomplete {row['name']}: {row['test_summary']}")


assert capture(["git", "rev-parse", "HEAD"]) == SOURCE
assert not capture(["git", "status", "--porcelain"])
RESULTS.mkdir(exist_ok=True)
native = SCRATCH / "native"
native.mkdir(parents=True, exist_ok=True)
seed = SCRATCH / "python-seed"
seed.mkdir(exist_ok=True)
environment = dict(os.environ, TMPDIR=str(native), UV_PYTHON_CACHE_DIR=str(seed))
environment.pop("UV_RUNNER_EXPERIMENT_R2", None)
command = [*CARGO, "--test-threads", "20"]

# Check the unchanged published fixture commit before adding diagnostic hooks.
exact = measure(
    [*command, "-E", "test(/^extract::/)"], "exact-fixtures", environment, RESULTS
)
verify(exact, 39)
assert not capture(["git", "status", "--porcelain"])
subprocess.run(["git", "apply", "--check", str(PATCH)], check=True)
subprocess.run(["git", "apply", str(PATCH)], check=True)
metadata = {
    "source": SOURCE,
    "original_fixtures": ORIGINAL_FIXTURES,
    "control_source": CONTROL,
    "common_auth_fix": AUTH_FIX,
    "mode": "network",
    "runner": f"{os.environ['RCA_RUNNER']}-replica{REPLICA}",
    "provider_runner": os.environ["RCA_RUNNER"],
    "replica": REPLICA,
    "schedules": SCHEDULES,
    "lscpu": json.loads(capture(["lscpu", "-J"])),
    "affinity": sorted(os.sched_getaffinity(0)),
    "kernel": capture(["uname", "-sr"]),
    "source_diff": capture(["git", "diff"]),
    "scope": "39 extraction cases change transport; 7 pip-install R2 cases are unchanged",
}
for filename in ("cpu.max", "cpuset.cpus.effective", "memory.max"):
    path = Path("/sys/fs/cgroup") / filename
    if path.exists():
        metadata[filename] = path.read_text().strip()
(RESULTS / "machine.json").write_text(json.dumps(metadata, indent=2) + "\n")

# Both variants use the same compiled binaries. The experiment-only flag selects
# the original R2 helper, including its retry behavior, or the checked-in reader.
subprocess.run([*command, "--no-run"], env=environment, check=True)
for variant in VARIANTS:
    warm_environment = dict(environment)
    if variant == "r2":
        warm_environment["UV_RUNNER_EXPERIMENT_R2"] = "1"
    verify(measure(command, f"warmup-{variant}", warm_environment, RESULTS), 4941)

failures = []
for scope, expected in (("suite", 4941), ("extract", 39)):
    scoped_command = (
        command if scope == "suite" else [*command, "-E", "test(/^extract::/)"]
    )
    for round_index, schedule in enumerate(SCHEDULES, start=1):
        for variant in schedule:
            with tempfile.TemporaryDirectory(prefix="sample-", dir=native) as temporary:
                python_cache = Path(temporary) / "python-downloads"
                subprocess.run(
                    ["cp", "-a", "--reflink=auto", str(seed), str(python_cache)],
                    check=True,
                )
                sample_environment = dict(
                    environment, TMPDIR=temporary, UV_PYTHON_CACHE_DIR=str(python_cache)
                )
                if variant == "r2":
                    sample_environment["UV_RUNNER_EXPERIMENT_R2"] = "1"
                name = f"{scope}-round-{round_index}-{variant}"
                row = measure(scoped_command, name, sample_environment, RESULTS)
                row.update(
                    scope=scope,
                    variant=variant,
                    round=round_index,
                    expected_tests=expected,
                )
                (RESULTS / f"{name}.json").write_text(json.dumps(row, indent=2) + "\n")
                try:
                    verify(row, expected)
                except RuntimeError:
                    failures.append(name)
if failures:
    raise RuntimeError(f"Incomplete measurements: {failures}")
