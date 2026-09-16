"""Check that sequential prebuilt uv suites retain their CodSpeed walltime results."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import sys
from collections.abc import Callable, Mapping
from pathlib import Path
from types import MappingProxyType
from typing import Any

SHARDS_PATH = Path(__file__).with_name("walltime-shards.py")
SHARDS_SPEC = importlib.util.spec_from_file_location("walltime_shards", SHARDS_PATH)
if SHARDS_SPEC is None or SHARDS_SPEC.loader is None:
    raise ImportError("Could not load the walltime shard helpers")
shards = importlib.util.module_from_spec(SHARDS_SPEC)
SHARDS_SPEC.loader.exec_module(shards)

CARGO_CODSPEED_VERSION = "cargo-codspeed 5.0.1"
RUNNER_VERSION = "codspeed-runner 5.0.2"
EXPECTED_BENCHMARKS = {
    "uv": "hash_sha256",
    "workspace_discovery": "discover_workspace_from_all_members",
}


def describe_result(name: str, contents: bytes) -> dict:
    if re.fullmatch(r"[1-9][0-9]*\.json", name) is None:
        raise ValueError("Unexpected cargo-codspeed aggregate filename")
    value = json.loads(contents)
    creator = value.get("creator") if isinstance(value, dict) else None
    benchmarks = value.get("benchmarks") if isinstance(value, dict) else None
    instrument = value.get("instrument") if isinstance(value, dict) else None
    if (
        not isinstance(creator, dict)
        or creator.get("name") != "codspeed-rust"
        or creator.get("version") != "5.0.1"
        or type(creator.get("pid")) is not int
        or str(creator["pid"]) != name[:-5]
        or instrument != {"type": "walltime"}
        or not isinstance(benchmarks, list)
        or not benchmarks
        or any(
            not isinstance(benchmark, dict)
            or not isinstance(benchmark.get("name"), str)
            or not isinstance(benchmark.get("uri"), str)
            for benchmark in benchmarks
        )
    ):
        raise ValueError("Unexpected cargo-codspeed aggregate metadata")
    return {
        "sha256": hashlib.sha256(contents).hexdigest(),
        "size": len(contents),
        "creator": creator,
        "benchmarks": [
            {"name": benchmark["name"], "uri": benchmark["uri"]}
            for benchmark in benchmarks
        ],
    }


def read_results(profile: Path) -> tuple[dict, dict[str, bytes]]:
    directory = profile / "results"
    if directory.is_symlink():
        raise ValueError("Unexpected symlink for CodSpeed results")
    if not directory.exists():
        return {}, {}
    inventory = {}
    contents = {}
    for path in sorted(directory.glob("*.json")):
        if path.is_symlink() or not path.is_file():
            raise ValueError("Unexpected CodSpeed result file type")
        data = path.read_bytes()
        inventory[path.name] = describe_result(path.name, data)
        contents[path.name] = data
    return inventory, contents


def verify_transition(previous: dict, current: dict, benchmark: str) -> str:
    if any(current.get(name) != entry for name, entry in previous.items()):
        raise ValueError("A completed walltime aggregate was removed or changed")
    added = set(current) - set(previous)
    if len(added) != 1:
        raise ValueError("Expected one new walltime aggregate for the selected suite")
    name = added.pop()
    if [item["name"] for item in current[name]["benchmarks"]] != [benchmark]:
        raise ValueError("The walltime aggregate contains unexpected benchmarks")
    return name


def retain_result(directory: Path, name: str, contents: bytes) -> None:
    path = directory / name
    if path.exists():
        if path.read_bytes() != contents:
            raise ValueError("A retained walltime aggregate was changed")
        return
    with path.open("xb") as stream:
        stream.write(contents)
        stream.flush()
        os.fsync(stream.fileno())


def is_selected_result(entry: dict) -> bool:
    names = [item["name"] for item in entry["benchmarks"]]
    return len(names) == 1 and names[0] in EXPECTED_BENCHMARKS.values()


def retain_final_observation(
    directory: Path,
    metadata: dict,
    inventory: dict,
    contents: dict[str, bytes],
    command_exit: int,
) -> dict:
    observed = directory / "final-observation"
    observed.mkdir()
    aggregates = observed / "aggregates"
    aggregates.mkdir()
    selected = {}
    unexpected = []
    for name, entry in inventory.items():
        if is_selected_result(entry):
            retain_result(aggregates, name, contents[name])
            selected[name] = entry
        else:
            unexpected.append(name)
    manifest = observed / "observation.json"
    shards.write_run_state(
        manifest,
        {
            **metadata,
            "version": 1,
            "observed_at": shards.timestamp(),
            "command_exit_code": command_exit,
            "inventory": selected,
            "unexpected_results": unexpected,
        },
    )
    return {
        "manifest": "final-observation/observation.json",
        "sha256": shards.digest(manifest),
        "unexpected_results": unexpected,
    }


def filtered_commands(plan: dict, *, cargo: str) -> list[dict]:
    # These short samples establish result publication, not a performance comparison.
    arguments = [
        "--",
        "--exact",
        "--sample-size",
        "10",
        "--warm-up-time",
        "0.1",
        "--measurement-time",
        "0.1",
    ]
    return [
        {
            "name": name,
            "command": shards.suite_command(name, cargo=cargo)
            + [benchmark]
            + arguments,
            "artifact": plan["artifacts"][name],
        }
        for name, benchmark in EXPECTED_BENCHMARKS.items()
    ]


def verify_context(
    source: dict, consumer: dict, environment: Mapping[str, str]
) -> None:
    if environment.get("BENCHMARK_SOURCE_SHA") != source["commit"]:
        raise ValueError("The aggregation probe belongs to different source")
    if environment.get("CODSPEED_SKIP_UPLOAD") != "true":
        raise ValueError("The aggregation probe requires disabled CodSpeed uploads")
    if any(
        name in environment
        for name in ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "ACTIONS_ID_TOKEN_REQUEST_URL")
    ):
        raise ValueError("The aggregation probe requires disabled GitHub OIDC")
    if (
        not consumer["inside_codspeed_runner"]
        or consumer["codspeed_runner_mode"] != "walltime"
        or consumer["cargo_codspeed"]["version"] != CARGO_CODSPEED_VERSION
        or consumer["codspeed"] is None
        or consumer["codspeed"]["version"] != RUNNER_VERSION
    ):
        raise ValueError("Unexpected CodSpeed aggregation tool context")


def check_aggregation(
    root: Path,
    directory: Path,
    profile: Path,
    metadata: dict,
    commands: list[dict],
    *,
    environment: Mapping[str, str],
    before_launch: Callable[[dict], None] | None = None,
    timeout_seconds: float | None = None,
    deadline: float | None = None,
    grace_seconds: float = shards.TERMINATION_GRACE_SECONDS,
) -> int:
    if [command["name"] for command in commands] != list(EXPECTED_BENCHMARKS):
        raise ValueError("Unexpected walltime aggregation suite order")
    directory.mkdir(parents=True, exist_ok=False)
    retained = directory / "aggregates"
    retained.mkdir()
    path = directory / "aggregation.json"
    state: dict[str, Any] = {
        **metadata,
        "version": 1,
        "status": "running",
        "started_at": shards.timestamp(),
        "finished_at": None,
        "profile_directory": str(profile),
        "expected_benchmarks": EXPECTED_BENCHMARKS,
        "observations": [],
    }
    shards.write_run_state(path, state)
    previous = {}
    launched = 0

    def checkpoint(current: dict, contents: dict[str, bytes], benchmark: str) -> None:
        nonlocal previous
        added = verify_transition(previous, current, benchmark)
        retain_result(retained, added, contents[added])
        state["observations"].append(
            {
                "after_benchmark": benchmark,
                "observed_at": shards.timestamp(),
                "added": added,
                "inventory": current,
            }
        )
        previous = current
        shards.write_run_state(path, state)

    def verify_launch(suite: dict) -> None:
        nonlocal launched
        current, contents = read_results(profile)
        if launched == 0:
            state["initial_inventory"] = current
            shards.write_run_state(path, state)
            if current:
                raise ValueError("Expected a fresh CodSpeed aggregation profile")
        elif launched == 1:
            checkpoint(current, contents, EXPECTED_BENCHMARKS["uv"])
        else:
            raise ValueError("Unexpected additional aggregation command")
        if before_launch is not None:
            before_launch(suite)
        launched += 1

    command_exit = shards.run_suites(
        root,
        directory / "commands",
        metadata,
        commands,
        timeout_seconds=timeout_seconds,
        deadline=deadline,
        grace_seconds=grace_seconds,
        environment=environment,
        before_launch=verify_launch,
    )
    exit_code = command_exit
    state["command_exit_code"] = command_exit
    state["commands_result_sha256"] = shards.digest(directory / "commands/result.json")
    try:
        current, contents = read_results(profile)
        state["final_inventory"] = current
        # A later command can change a completed aggregate. Keep its final bytes
        # separate from the accepted prefix before checking that transition.
        observation = retain_final_observation(
            directory, metadata, current, contents, command_exit
        )
        state["final_observation"] = observation
        shards.write_run_state(path, state)
        if observation["unexpected_results"]:
            raise ValueError("Unexpected benchmarks in final walltime aggregates")
        if command_exit == 0:
            if launched != 2:
                raise ValueError("The aggregation commands did not both start")
            checkpoint(
                current,
                contents,
                EXPECTED_BENCHMARKS["workspace_discovery"],
            )
        else:
            # Failed commands can still leave useful output. Only the two named
            # benchmark records belong in this probe's evidence artifact.
            for name, entry in current.items():
                if is_selected_result(entry):
                    retain_result(retained, name, contents[name])
    except (OSError, ValueError) as error:
        state["evidence_error"] = shards.failure_details(error, "aggregation")
        if exit_code == 0:
            exit_code = 1
    state.update(
        status="success" if exit_code == 0 else "failed",
        finished_at=shards.timestamp(),
        exit_code=exit_code,
    )
    shards.write_run_state(path, state)
    return exit_code


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    limit = parser.add_mutually_exclusive_group()
    limit.add_argument("--deadline", type=shards.positive_seconds)
    limit.add_argument("--timeout-seconds", type=shards.positive_seconds)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    environment = MappingProxyType(dict(os.environ))
    consumer = shards.consumer_metadata(root, environment)
    source = shards.source_identity(root, environment=environment)
    verify_context(source, consumer, environment)
    profile = Path(environment["CODSPEED_PROFILE_FOLDER"])
    if not profile.is_absolute():
        raise ValueError("Expected an absolute CodSpeed profile directory")
    profile = profile.resolve(strict=True)
    plan_path = root / ".cache/bench-walltime-shards.json"
    plan = json.loads(plan_path.read_text())
    cargo = consumer["cargo"]["invocation_executable"]
    artifacts = shards.built_artifacts(root, cargo=cargo, environment=environment)
    selected = {name: item for item in plan["shards"] for name in item["benches"]}
    for name in EXPECTED_BENCHMARKS:
        shards.verify_plan(plan, source, artifacts, selected[name]["index"], bench=name)
    artifact_directory = next(iter(artifacts.values())).parent
    metadata = {
        "source": source,
        "producer": plan["producer"],
        "consumer": consumer,
        "python": shards.tool_identity(
            root, sys.executable, "--version", environment=environment
        ),
        "plan_sha256": shards.digest(plan_path),
        "codspeed_skip_upload": True,
        "github_oidc_environment_present": False,
    }
    sys.exit(
        check_aggregation(
            root,
            args.output,
            profile,
            metadata,
            filtered_commands(plan, cargo=cargo),
            environment=environment,
            before_launch=lambda suite: shards.verify_suite_launch(
                root,
                plan,
                selected[suite["name"]],
                consumer,
                environment,
                artifact_directory,
                suite,
            ),
            deadline=args.deadline,
            timeout_seconds=(
                args.timeout_seconds
                if args.timeout_seconds is not None
                else 240
                if args.deadline is None
                else None
            ),
        )
    )


if __name__ == "__main__":
    main()
