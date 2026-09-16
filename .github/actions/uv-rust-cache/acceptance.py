# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Retain the source and cache evidence for the opt-in Linux nextest acceptance."""

from __future__ import annotations

import argparse
import base64
import collections
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
import re
import stat
import subprocess
import sys
import tempfile
import time
import types
import uuid
import xml.etree.ElementTree as ET
from collections.abc import Callable
from itertools import pairwise
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "uv_rust_cache", Path(__file__).with_name("cache.py")
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Could not load the Rust-cache identity helper")
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)

FORMAT = "uv-rust-cache-acceptance"
VERSION = 1
REPOSITORY = "astral-sh/uv-dev"
BASE = "f36ab068f6d834c7e13d5b7b10c7aca80d04cd65"
CANDIDATE = "65d6ba4fe450b7df59d8a1cda83083a02865ccd0"
TREES = {
    BASE: "3f2b669811b4703bfbdf5c81202feb5eec086926",
    CANDIDATE: "dce5fc7df51d290bfaeb81841d33e4ab079001e1",
}
PROCESS_OWNER = {
    "commit": "f1c904fb0930efeaf82232ea1884c41a18b93dd8",
    "path": "scripts/benchmark/walltime-shards.py",
    "blob": "26730b1b5fc5f9e8111fb9d6a24ca349dfe9390b",
    "sha256": "695d291a528a74d93e3f478623731a6eaaf5d9f02906d84278d25d6e39b23adf",
}
WORKFLOW_PATHS = (
    ".github/workflows/linux-runner-acceptance.yml",
    ".github/workflows/ci-rust-cache-acceptance-job.yml",
)
SOURCE_CASES = (
    "downloads-seed",
    "baseline-cold",
    "baseline-exact",
    "candidate-fallback",
    "candidate-exact",
    "candidate-relocated",
)
FAULT_CASES = ("malformed-cache-fixture", "malformed-cache-consumer")
CASE_DATA = {
    "downloads-seed": (BASE, "source", "seed", True, "unattempted"),
    "baseline-cold": (BASE, "source", "full", True, "miss"),
    "baseline-exact": (BASE, "source", "full", False, "exact"),
    "candidate-fallback": (CANDIDATE, "source", "full", True, "baseline"),
    "candidate-exact": (CANDIDATE, "source", "full", False, "exact"),
    "candidate-relocated": (CANDIDATE, "relocated-source", "full", False, "exact"),
    "malformed-cache-fixture": (CANDIDATE, "source", "fault-fixture", False, "exact"),
    "malformed-cache-consumer": (CANDIDATE, "source", "full", False, "malformed"),
}
RUNTIME_ENVIRONMENT = {
    "UV_HTTP_RETRIES": "5",
    "RUST_BACKTRACE": "1",
    "UV_INTERNAL__TEST_COW_FS": "/btrfs",
    "UV_INTERNAL__TEST_NOCOW_FS": "/tmpfs",
    "UV_INTERNAL__TEST_ALT_FS": "/tmpfs",
    "UV_INTERNAL__TEST_LOWLINKS_FS": "/minix",
    "INSTA_UPDATE": "new",
}
MALFORMED_RUSTC_INFO = b'{"uv-rust-cache-acceptance":\n'
MAX_METADATA_BYTES = 64 * 1024 * 1024
MAX_LOG_BYTES = 512 * 1024 * 1024
MAX_FINGERPRINT_BYTES = 32 * 1024 * 1024
MAX_FINGERPRINT_JSON_BYTES = 128 * 1024 * 1024
MAX_EVIDENCE_FILES = 20_000
MAX_EVIDENCE_FILE_BYTES = 512 * 1024 * 1024
MAX_EVIDENCE_BYTES = 2 * 1024 * 1024 * 1024
MAX_PAYLOAD_ENTRIES = 500_000
OUTCOMES = {"success", "failure", "cancelled", "skipped", "unknown"}
OUTCOME_NAMES = {
    "setup",
    "setup-mold",
    "setup-rust",
    "setup-uv",
    "setup-python",
    "setup-keyring",
    "setup-filesystems",
    "setup-nextest",
    "seed-prepare",
    "seed-downloads",
    "seed-observe",
    "restore",
    "observation",
    "workload",
    "prune",
    "seed-save-plan",
    "seed-save",
    "save",
    "save-downloads",
    "save-target",
    "fault-prepare",
    "fault-save",
}
STAGES = ("source", "source-and-malformed-cache")
JOB_BUDGET_SECONDS = 40 * 60
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


class AcceptanceError(ValueError):
    """An acceptance observation cannot establish its declared contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AcceptanceError(message)


def timestamp() -> str:
    seconds, nanoseconds = divmod(time.time_ns(), 1_000_000_000)
    return (
        time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(seconds))
        + f".{nanoseconds // 1_000_000:03d}Z"
    )


def write_bytes(path: Path, contents: bytes) -> str:
    with path.open("xb") as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(contents)
        stream.flush()
        os.fsync(stream.fileno())
    return hashlib.sha256(contents).hexdigest()


def write_json(path: Path, value: Any) -> str:
    return write_bytes(path, cache.json_bytes(value))


def replace_json(path: Path, value: Any) -> None:
    with tempfile.NamedTemporaryFile(
        prefix=path.name + ".", dir=path.parent, delete=False
    ) as stream:
        temporary = Path(stream.name)
        os.fchmod(stream.fileno(), 0o600)
        stream.write(cache.json_bytes(value))
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def file_state(value: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        value.st_dev,
        value.st_ino,
        value.st_mode,
        value.st_size,
        value.st_mtime_ns,
        value.st_ctime_ns,
    )


def bounded_bytes(path: Path, limit: int) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        require(
            stat.S_ISREG(before.st_mode) and before.st_size <= limit,
            "Evidence is not a bounded regular file",
        )
        contents = stream.read(limit + 1)
        after = os.fstat(stream.fileno())
    require(
        len(contents) <= limit
        and file_state(before) == file_state(after) == file_state(path.lstat()),
        "Evidence changed while it was read",
    )
    return contents


def regular_file_reference(path: Path, *, limit: int | None = None) -> dict[str, Any]:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        require(
            stat.S_ISREG(before.st_mode) and (limit is None or before.st_size <= limit),
            "Evidence is not a bounded regular file",
        )
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        after = os.fstat(stream.fileno())
    require(
        file_state(before) == file_state(after) == file_state(path.lstat()),
        "Evidence changed while it was hashed",
    )
    return {"sha256": checksum, "size": before.st_size}


def json_value(contents: bytes) -> Any:
    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for key, item in items:
            require(key not in value, "Duplicate JSON field")
            value[key] = item
        return value

    def nonfinite(_value: str) -> Any:
        raise AcceptanceError("Non-finite JSON value")

    return json.loads(contents, object_pairs_hook=pairs, parse_constant=nonfinite)


def read_json(path: Path, limit: int = MAX_METADATA_BYTES) -> Any:
    return json_value(bounded_bytes(path, limit))


def github_identity(environment: dict[str, str]) -> dict[str, str]:
    keys = (
        "GITHUB_REPOSITORY",
        "GITHUB_SHA",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_JOB",
        "GITHUB_WORKFLOW_REF",
        "GITHUB_WORKFLOW_SHA",
    )
    require(
        environment.get("GITHUB_REPOSITORY") == REPOSITORY,
        "Unexpected acceptance repository",
    )
    require(
        environment.get("GITHUB_EVENT_NAME") == "workflow_dispatch",
        "Acceptance requires an explicit workflow dispatch",
    )
    require(
        cache.OID.fullmatch(environment.get("GITHUB_SHA", "")) is not None,
        "Expected the full workflow commit",
    )
    workflow = environment.get("GITHUB_WORKFLOW_REF", "")
    require(
        environment.get("GITHUB_WORKFLOW_SHA") == environment["GITHUB_SHA"]
        and any(
            workflow.startswith(
                REPOSITORY + "/" + WORKFLOW_PATHS[0] + "@refs/" + kind + "/"
            )
            for kind in ("heads", "tags")
        )
        and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", environment.get("GITHUB_JOB", ""))
        is not None,
        "The acceptance workflow identity is incomplete or inconsistent",
    )
    for name in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"):
        require(
            re.fullmatch(r"[1-9][0-9]{0,19}", environment.get(name, "")) is not None,
            "Invalid workflow run identity",
        )
    return {name: environment[name] for name in keys if name in environment}


def controller_identity(environment: dict[str, str]) -> dict[str, Any]:
    github = github_identity(environment)
    git = cache.executable_identity("git", environment)
    source = cache.source_identity(
        ROOT, REPOSITORY, github["GITHUB_SHA"], git, environment
    )
    return {
        "commit": source["commit"],
        "tree": source["tree"],
        "path": str(ROOT),
        "helper_sha256": cache.file_identity(Path(__file__))["sha256"],
        "cache_helper_sha256": cache.file_identity(
            Path(__file__).with_name("cache.py")
        )["sha256"],
        "workflow_blobs": {
            path: cache.git_command(ROOT, git, environment, "rev-parse", f"HEAD:{path}")
            for path in WORKFLOW_PATHS
        },
    }


def plan_value(case: str, environment: dict[str, str]) -> dict[str, Any]:
    require(case in CASE_DATA, "Unknown acceptance case")
    stage = environment.get("UV_RUST_CACHE_ACCEPTANCE_STAGE", "")
    require(
        stage in STAGES and (case in SOURCE_CASES or stage == STAGES[1]),
        "Acceptance case is outside the selected stage",
    )
    github = github_identity(environment)
    workspace = cache.safe_path(environment["GITHUB_WORKSPACE"]).resolve(strict=True)
    require(
        ROOT.is_relative_to(workspace), "Controller is outside the Actions workspace"
    )
    namespace = cache.validate_key_namespace(
        f"acceptance-{github['GITHUB_RUN_ID']}-{github['GITHUB_RUN_ATTEMPT']}"
    )
    fault_namespace = cache.validate_key_namespace(namespace + "-fault")
    commit, directory, mode, save_allowed, expectation = CASE_DATA[case]
    deadline = environment.get("UV_RUST_CACHE_JOB_DEADLINE", "")
    started = environment.get("UV_RUST_CACHE_JOB_STARTED", "")
    require(
        re.fullmatch(r"[1-9][0-9]{0,11}", deadline) is not None
        and re.fullmatch(r"[1-9][0-9]{0,11}", started) is not None
        and int(deadline) - int(started) == JOB_BUDGET_SECONDS,
        "Missing or unexpected acceptance job deadline",
    )
    return {
        "format": FORMAT + "-plan",
        "version": VERSION,
        "case": case,
        "stage": stage,
        "mode": mode,
        "source": {
            "repository": REPOSITORY,
            "commit": commit,
            "tree": TREES[commit],
            "relative_directory": directory,
            "path": str(workspace / directory),
        },
        "namespace": fault_namespace
        if case == "malformed-cache-consumer"
        else namespace,
        "fault_namespace": fault_namespace,
        "save_allowed": save_allowed,
        "expected_target": expectation,
        "workspace": str(workspace),
        "job_started_unix": int(started),
        "deadline_unix": int(deadline),
        "controller": controller_identity(environment),
        "process_owner": PROCESS_OWNER,
        "github": github,
    }


def load_plan(path: Path, checksum: str, environment: dict[str, str]) -> dict[str, Any]:
    value = cache.read_json(path, checksum)
    require(isinstance(value.get("case"), str), "Invalid acceptance plan")
    require(
        cache.canonical_json(value)
        == cache.canonical_json(plan_value(value["case"], environment)),
        "Acceptance source or run identity changed",
    )
    return value


def evidence_directory(plan: dict[str, Any]) -> Path:
    return Path(plan["workspace"]) / "results" / plan["case"]


def actual_source(plan: dict[str, Any], environment: dict[str, str]) -> dict[str, Any]:
    expected = plan["source"]
    git = cache.executable_identity("git", environment)
    source = cache.source_identity(
        Path(expected["path"]),
        expected["repository"],
        expected["commit"],
        git,
        environment,
    )
    require(source["tree"] == expected["tree"], "Unexpected acceptance source tree")
    return source


def phase(
    plan: dict[str, Any], name: str, event: str, *, outcome: str | None = None
) -> None:
    require(
        name in {"setup", "restore", "workload", "prune", "save", "fault"}
        and event in {"start", "end"},
        "Invalid acceptance phase",
    )
    directory = evidence_directory(plan)
    value = {
        "case": plan["case"],
        "phase": name,
        "event": event,
        "observed_at": timestamp(),
        "monotonic_ns": time.monotonic_ns(),
    }
    if event == "end":
        start = read_json(directory / f"phase-{name}-start.json")
        require(
            start["case"] == plan["case"]
            and start["phase"] == name
            and start["event"] == "start"
            and type(start["monotonic_ns"]) is int
            and start["monotonic_ns"] <= value["monotonic_ns"],
            "Invalid phase start",
        )
        value["elapsed_ns"] = value["monotonic_ns"] - start["monotonic_ns"]
    if outcome is not None:
        require(
            outcome in {"success", "failure", "cancelled", "skipped", "unknown"},
            "Invalid phase outcome",
        )
        value["outcome"] = outcome
    write_json(directory / f"phase-{name}-{event}.json", value)


def write_environment(path: Path, values: dict[str, str]) -> None:
    with path.open("a", encoding="utf-8") as stream:
        for name, value in values.items():
            require(
                re.fullmatch(r"[A-Z][A-Z0-9_]*", name) is not None
                and not any(c in value for c in "\r\n"),
                "Invalid workflow environment value",
            )
            stream.write(f"{name}={value}\n")


def begin(plan: dict[str, Any], environment: dict[str, str], github_env: Path) -> None:
    directory = evidence_directory(plan)
    directory.mkdir(parents=True, exist_ok=False)
    source = actual_source(plan, environment)
    temporary = (
        Path(environment["HOME"])
        / "code/tmp/uv-rust-cache-acceptance"
        / plan["github"]["GITHUB_RUN_ID"]
        / plan["github"]["GITHUB_RUN_ATTEMPT"]
        / plan["case"]
    )
    temporary.mkdir(mode=0o700, parents=True, exist_ok=False)
    write_environment(github_env, {"TMPDIR": str(temporary)})
    write_json(directory / "plan.json", plan)
    machine = {
        "uname": list(os.uname()),
        "cpu_count": os.cpu_count(),
        "lscpu": subprocess.check_output(["lscpu", "--json"], text=True, timeout=15),
        "memory": subprocess.check_output(["free", "-b"], text=True, timeout=15),
        "workspace_filesystem": subprocess.check_output(
            ["findmnt", "-n", "-o", "FSTYPE,TARGET", "-T", plan["workspace"]],
            text=True,
            timeout=15,
        ),
    }
    write_json(
        directory / "start.json",
        {
            "format": FORMAT + "-start",
            "version": VERSION,
            "plan": plan,
            "actual_source": source,
            "machine": machine,
            "temporary_directory": str(temporary),
            "started_at": timestamp(),
        },
    )
    phase(plan, "setup", "start")


def load_process_groups(
    repository: Path, environment: dict[str, str]
) -> tuple[Any, dict[str, Any]]:
    git = cache.executable_identity("git", environment)
    revision = PROCESS_OWNER["commit"] + ":" + PROCESS_OWNER["path"]
    require(
        cache.git_command(repository, git, environment, "rev-parse", revision)
        == PROCESS_OWNER["blob"],
        "Unexpected process-control source blob",
    )
    cache.verify_executable("git", git, environment)
    contents = subprocess.check_output(
        [git["invocation"], "show", revision],
        cwd=repository,
        env=environment,
        timeout=15,
    )
    require(
        len(contents) <= 128 * 1024
        and hashlib.sha256(contents).hexdigest() == PROCESS_OWNER["sha256"],
        "Unexpected process-control source contents",
    )
    cache.verify_executable("git", git, environment)
    require(
        cache.git_command(repository, git, environment, "rev-parse", revision)
        == PROCESS_OWNER["blob"],
        "Process-control source changed",
    )
    module = types.ModuleType("uv_rust_cache_process_groups")
    module.__file__ = revision
    sys.modules[module.__name__] = module
    # Both the Git blob and its complete byte digest are checked before execution.
    exec(compile(contents, revision, "exec"), module.__dict__)  # noqa: S102
    return module, {
        **PROCESS_OWNER,
        "repository_path": str(repository.resolve(strict=True)),
    }


def recorded_manifest(plan: dict[str, Any], state: dict[str, Any]) -> dict[str, Any]:
    require(
        isinstance(state, dict)
        and set(state)
        == {"format", "version", "identity", "identity_sha256", "restore"}
        and state["format"] == cache.FORMAT + "-restored"
        and type(state["version"]) is int
        and state["version"] == cache.VERSION,
        "Unsupported restored-cache evidence",
    )
    manifest = state["identity"]
    require(
        isinstance(manifest, dict)
        and set(manifest)
        == {"format", "version", "workload", "identity", "policy", "keys"}
        and manifest["format"] == cache.FORMAT
        and type(manifest["version"]) is int
        and manifest["version"] == cache.VERSION
        and manifest["workload"] == cache.WORKLOAD
        and isinstance(manifest["identity"], dict)
        and isinstance(manifest["policy"], dict)
        and isinstance(manifest["keys"], dict)
        and hashlib.sha256(cache.json_bytes(manifest)).hexdigest()
        == state["identity_sha256"],
        "Invalid cache identity evidence",
    )
    identity = manifest["identity"]
    require(
        isinstance(identity.get("source"), dict)
        and identity["source"]["commit"] == plan["source"]["commit"]
        and identity["source"]["tree"] == plan["source"]["tree"]
        and identity["source"]["path"] == plan["source"]["path"]
        and identity["source"]["repository"] == REPOSITORY
        and isinstance(identity["source"].get("inputs"), list),
        "Restore manifest belongs to another source",
    )
    require(
        isinstance(identity.get("implementation"), dict)
        and identity["implementation"]["commit"] == plan["controller"]["commit"]
        and identity["implementation"]["tree"] == plan["controller"]["tree"]
        and identity["implementation"]["script_sha256"]
        == plan["controller"]["cache_helper_sha256"],
        "Restore manifest belongs to another helper source",
    )
    paths = identity.get("paths")
    require(
        isinstance(paths, dict)
        and set(paths)
        == {
            "workspace",
            "source",
            "cargo_home",
            "target",
            "download_paths",
            "target_paths",
        }
        and paths["workspace"] == plan["workspace"]
        and paths["source"] == plan["source"]["path"]
        and paths["target"] == str(Path(plan["workspace"]) / "target")
        and isinstance(paths["cargo_home"], str)
        and Path(paths["cargo_home"]).is_absolute()
        and Path(paths["cargo_home"]).name == ".cargo"
        and paths["download_paths"] == list(cache.DOWNLOAD_PATHS)
        and paths["target_paths"] == list(cache.TARGET_PATHS)
        and identity["implementation"].get("cache_action") == cache.CACHE_ACTION
        and identity.get("github") == plan["github"],
        "Restore manifest has another workload location or implementation",
    )
    require(
        type(manifest["policy"].get("save_allowed")) is bool
        and manifest["policy"]
        == {"save_allowed": plan["save_allowed"], "key_namespace": plan["namespace"]},
        "Restore manifest has another cache policy",
    )
    require(
        manifest["keys"] == cache.cache_keys(identity, plan["namespace"])
        and isinstance(state["restore"], dict)
        and set(state["restore"]) == {"downloads", "target"},
        "Cache keys or observations changed",
    )
    for kind, value in state["restore"].items():
        require(
            isinstance(value, dict)
            and set(value) == {"requested", "primary", "matched", "exact"}
            and isinstance(value["requested"], str)
            and isinstance(value["primary"], str)
            and (value["matched"] is None or isinstance(value["matched"], str))
            and type(value["exact"]) is bool,
            "Invalid recorded cache observation",
        )
        require(
            value
            == cache.restore_observation(
                manifest["keys"],
                kind,
                value["primary"],
                value["matched"] or "",
                str(value["exact"]).lower(),
            ),
            "Cache observation does not match its identity",
        )
    return state


def manifest_state(
    plan: dict[str, Any], path: Path, checksum: str, environment: dict[str, str]
) -> dict[str, Any]:
    state = cache.read_json(path, checksum)
    cache.save_plan(state, environment, "success")
    return recorded_manifest(plan, state)


def expectations(plan: dict[str, Any], state: dict[str, Any]) -> dict[str, bool]:
    target = state["restore"]["target"]
    downloads = state["restore"]["downloads"]
    expected = plan["expected_target"]
    target_ok = (
        not target["primary"] and target["matched"] is None
        if expected == "unattempted"
        else target["primary"] == target["requested"] and target["matched"] is None
        if expected == "miss"
        else target["matched"] == state["identity"]["keys"]["target_restore"] + BASE
        and not target["exact"]
        if expected == "baseline"
        else target["exact"]
    )
    downloads_ok = (
        downloads["primary"] == downloads["requested"] and downloads["matched"] is None
        if plan["mode"] == "seed" or expected == "malformed"
        else downloads["exact"]
    )
    return {"downloads": downloads_ok, "target": target_ok}


def observe(
    plan: dict[str, Any],
    path: Path,
    checksum: str,
    environment: dict[str, str],
    github_output: Path,
) -> None:
    state = manifest_state(plan, path, checksum, environment)
    directory = evidence_directory(plan)
    contents = bounded_bytes(path, cache.MAX_JSON_BYTES)
    require(
        hashlib.sha256(contents).hexdigest() == checksum, "Restore manifest changed"
    )
    write_bytes(directory / "restore-manifest.json", contents)
    checks = expectations(plan, state)
    if plan["expected_target"] == "malformed":
        observed = bounded_bytes(
            Path(state["identity"]["identity"]["paths"]["target"]) / ".rustc_info.json",
            cache.MAX_CONFIG_BYTES,
        )
        checks["malformed_metadata_restored"] = observed == MALFORMED_RUSTC_INFO
        write_bytes(directory / "restored-rustc-info.json", observed)
    write_json(
        directory / "restore-observation.json",
        {
            "source_manifest": str(path),
            "source_manifest_sha256": checksum,
            "service_observation_origin": "official-cache-actions",
            "restore": state["restore"],
            "expectations": checks,
        },
    )
    cache.write_outputs(
        github_output,
        {
            **cache.common_outputs(state["identity"]),
            "manifest": str(path),
            "manifest-sha256": checksum,
            "restore-contract": str(all(checks.values())).lower(),
        },
    )


def fingerprint_inventory(target: Path) -> dict[str, Any]:
    directory = target / cache.PROFILE / ".fingerprint"
    entries: list[dict[str, Any]] = []
    total = 0
    if directory.exists():
        require(
            directory.is_dir() and not directory.is_symlink(),
            "Invalid fingerprint root",
        )
        for path in sorted(directory.rglob("*")):
            if path.is_dir() and not path.is_symlink():
                continue
            require(
                not path.is_symlink() and path.is_file(),
                "Unsupported fingerprint entry",
            )
            contents = bounded_bytes(path, cache.MAX_CONFIG_BYTES)
            total += len(contents)
            require(
                total <= MAX_FINGERPRINT_BYTES,
                "Fingerprint inventory exceeds its budget",
            )
            item: dict[str, Any] = {
                "path": path.relative_to(target).as_posix(),
                "size": len(contents),
                "sha256": hashlib.sha256(contents).hexdigest(),
                "contents_base64": base64.b64encode(contents).decode("ascii"),
            }
            if path.suffix == ".json":
                try:
                    item["configuration"] = json_value(contents)
                except (ValueError, UnicodeError):
                    item["configuration_error"] = "invalid-json"
            entries.append(item)
    return {
        "entries": entries,
        "files": len(entries),
        "bytes": total,
        "sha256": cache.digest(entries),
    }


def payload_inventory(target: Path, *, hash_contents: bool) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    roots = [target / ".rustc_info.json", target / cache.PROFILE]
    pending = list(roots)
    while pending:
        path = pending.pop()
        relative = path.relative_to(target).as_posix()
        if relative in {f"{cache.PROFILE}/incremental", f"{cache.PROFILE}/.cargo-lock"}:
            continue
        try:
            value = path.lstat()
        except FileNotFoundError:
            continue
        item: dict[str, Any] = {"path": relative, "mode": stat.S_IMODE(value.st_mode)}
        if stat.S_ISDIR(value.st_mode):
            item["kind"] = "directory"
            pending.extend(path.iterdir())
        elif stat.S_ISLNK(value.st_mode):
            item.update(kind="symlink", target=os.readlink(path))
        elif stat.S_ISREG(value.st_mode):
            item.update(kind="file", size=value.st_size)
            if hash_contents:
                item["sha256"] = regular_file_reference(path)["sha256"]
        else:
            raise AcceptanceError("Unsupported compiled-cache entry")
        entries.append(item)
        require(
            len(entries) <= MAX_PAYLOAD_ENTRIES,
            "Compiled-cache inventory exceeds its entry limit",
        )
    entries.sort(key=lambda item: item["path"])
    return {
        "entries": entries if hash_contents else None,
        "files": sum(item["kind"] == "file" for item in entries),
        "bytes": sum(item.get("size", 0) for item in entries),
        "symlinks": sum(item["kind"] == "symlink" for item in entries),
        "content_tree_sha256": cache.digest(entries) if hash_contents else None,
    }


def record_inventory(plan: dict[str, Any], name: str, target: Path) -> None:
    require(name in {"restored", "built", "pruned"}, "Invalid inventory phase")
    directory = evidence_directory(plan)
    fingerprints = fingerprint_inventory(target)
    require(
        len(cache.json_bytes(fingerprints)) <= MAX_FINGERPRINT_JSON_BYTES,
        "Fingerprint evidence exceeds its JSON budget",
    )
    write_json(directory / f"fingerprints-{name}.json", fingerprints)
    write_json(
        directory / f"payload-{name}.json",
        payload_inventory(target, hash_contents=False),
    )


def normalize_locations(value: Any, source: Path, target: Path) -> Any:
    """Normalize only leading, component-delimited paths in comparison records."""
    if isinstance(value, dict):
        return {
            key: normalize_locations(item, source, target)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [normalize_locations(item, source, target) for item in value]
    if not isinstance(value, str):
        return value
    for path, label in ((source, "$SOURCE"), (target, "$TARGET")):
        for prefix, replacement in (
            ("path+" + path.as_uri(), "path+file://" + label),
            (path.as_uri(), "file://" + label),
            (str(path), label),
        ):
            if value == prefix or value.startswith((prefix + "/", prefix + "#")):
                return replacement + value[len(prefix) :]
    return value


def inside_target(value: str, target: Path) -> bool:
    path = Path(value)
    return path.is_absolute() and Path(os.path.normpath(path)).is_relative_to(target)


def string_list(value: Any, *, nonempty: bool = False) -> bool:
    return (
        isinstance(value, list)
        and (bool(value) or not nonempty)
        and all(isinstance(item, str) and bool(item) for item in value)
        and len(value) == len(set(value))
    )


def output_layout(filename: str, target: Path) -> str:
    """Retain native versus explicit-target layout without a source-derived crate hash."""
    require(
        inside_target(filename, target), "Cargo artifact is outside the selected target"
    )
    parts = Path(filename).relative_to(target).parts
    if len(parts) > 1 and parts[0] == cache.PROFILE:
        return "native"
    if (
        len(parts) > 2
        and parts[1] == cache.PROFILE
        and re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*", parts[0]) is not None
    ):
        return "target:" + parts[0]
    raise AcceptanceError("Cargo artifact uses an unexpected output layout")


def cargo_records(
    contents: bytes, source: Path, target: Path, *, require_complete: bool
) -> dict[str, Any]:
    require(len(contents) <= MAX_LOG_BYTES, "Cargo output exceeds its parsing budget")
    artifacts = []
    finished = []
    other_lines: list[bytes] = []
    reasons: collections.Counter[str] = collections.Counter()
    for line in contents.splitlines():
        if not line.strip():
            continue
        if not line.lstrip().startswith(b"{"):
            # Cargo cannot control arbitrary output from tools or procedural macros.
            # Keep those bytes in the raw log without counting them as Cargo events.
            other_lines.append(line)
            continue
        value = json_value(line)
        require(
            isinstance(value, dict) and isinstance(value.get("reason"), str),
            "Invalid Cargo JSON record",
        )
        require(
            value["reason"]
            in {
                "compiler-artifact",
                "compiler-message",
                "build-script-executed",
                "build-finished",
            },
            "Unknown Cargo JSON record",
        )
        reasons[value["reason"]] += 1
        if value["reason"] == "build-finished":
            require(
                type(value.get("success")) is bool,
                "Invalid Cargo build-finished record",
            )
            finished.append(value["success"])
        elif value["reason"] == "compiler-artifact":
            require(
                type(value.get("fresh")) is bool
                and isinstance(value.get("package_id"), str)
                and bool(value["package_id"])
                and isinstance(value.get("manifest_path"), str)
                and Path(value["manifest_path"]).is_absolute()
                and isinstance(value.get("target"), dict)
                and isinstance(value.get("profile"), dict)
                and string_list(value.get("features"))
                and string_list(value.get("filenames"), nonempty=True)
                and (
                    value.get("executable") is None
                    or isinstance(value["executable"], str)
                ),
                "Invalid Cargo artifact record",
            )
            cargo_target = value["target"]
            profile = value["profile"]
            require(
                string_list(cargo_target.get("kind"), nonempty=True)
                and string_list(cargo_target.get("crate_types"), nonempty=True)
                and isinstance(cargo_target.get("name"), str)
                and bool(cargo_target["name"])
                and isinstance(cargo_target.get("src_path"), str)
                and Path(cargo_target["src_path"]).is_absolute()
                and isinstance(cargo_target.get("edition"), str)
                and bool(cargo_target["edition"])
                and all(
                    type(cargo_target.get(name)) is bool
                    for name in ("doc", "doctest", "test")
                )
                and (
                    "required-features" not in cargo_target
                    or string_list(cargo_target["required-features"])
                )
                and isinstance(profile.get("opt_level"), str)
                and bool(profile["opt_level"])
                and "debuginfo" in profile
                and (
                    profile["debuginfo"] is None
                    or (
                        type(profile["debuginfo"]) is int
                        and profile["debuginfo"] in {0, 1, 2}
                    )
                    or (
                        isinstance(profile["debuginfo"], str)
                        and profile["debuginfo"]
                        in {"line-directives-only", "line-tables-only"}
                    )
                )
                and all(
                    type(profile.get(name)) is bool
                    for name in ("debug_assertions", "overflow_checks", "test")
                ),
                "Invalid Cargo target or profile",
            )
            for filename in value["filenames"]:
                require(
                    inside_target(filename, target),
                    "Cargo artifact is outside the selected target",
                )
            require(
                value.get("executable") is None
                or (
                    inside_target(value["executable"], target)
                    and value["executable"] in value["filenames"]
                ),
                "Invalid Cargo executable location",
            )
            unit = {
                key: value[key]
                for key in ("package_id", "manifest_path", "target", "profile")
            }
            unit["features"] = sorted(value["features"])
            logical = normalize_locations(unit, source, target)
            logical["output_layouts"] = sorted(
                {output_layout(filename, target) for filename in value["filenames"]}
            )
            normalized = {
                **logical,
                "host": cache.HOST,
                "filenames": normalize_locations(
                    sorted(value["filenames"]), source, target
                ),
                "executable": normalize_locations(
                    value.get("executable"), source, target
                ),
            }
            artifacts.append(
                {
                    "record": value,
                    "unit": normalized,
                    "unit_sha256": cache.digest(normalized),
                    "logical_unit_sha256": cache.digest(logical),
                }
            )
    if require_complete:
        require(
            finished == [True] and bool(artifacts),
            "Cargo did not report one successful complete build",
        )
    return {
        "records": artifacts,
        "record_reasons": dict(sorted(reasons.items())),
        "other_stdout_lines": len(other_lines),
        "other_stdout_sha256": hashlib.sha256(b"\n".join(other_lines)).hexdigest(),
        "build_finished": finished,
        "compiler_artifact_records": len(artifacts),
        "fresh_records": sum(item["record"]["fresh"] for item in artifacts),
        "nonfresh_records": sum(not item["record"]["fresh"] for item in artifacts),
        "unit_multiset": dict(
            sorted(
                collections.Counter(item["unit_sha256"] for item in artifacts).items()
            )
        ),
        "logical_unit_multiset": dict(
            sorted(
                collections.Counter(
                    item["logical_unit_sha256"] for item in artifacts
                ).items()
            )
        ),
    }


def duration_seconds(value: str) -> float:
    parts = re.findall(r"([0-9]+(?:\.[0-9]+)?)(h|m|s)", value.replace(" ", ""))
    require(
        bool(parts)
        and "".join(number + unit for number, unit in parts) == value.replace(" ", ""),
        "Invalid reported duration",
    )
    order = ["hms".index(unit) for _, unit in parts]
    require(order == sorted(set(order)), "Invalid reported duration")
    result = sum(
        float(number) * {"h": 3600, "m": 60, "s": 1}[unit] for number, unit in parts
    )
    require(math.isfinite(result), "Invalid reported duration")
    return result


def reported_times(contents: bytes) -> dict[str, Any]:
    text = ANSI.sub("", contents.decode("utf-8", errors="replace"))

    def observed_line(prefix: str, pattern: str) -> re.Match[str]:
        lines = [
            line.strip()
            for line in text.splitlines()
            if line.strip() == prefix or line.lstrip().startswith(prefix + " ")
        ]
        require(len(lines) == 1, "Missing or ambiguous nextest timing summary")
        match = re.fullmatch(pattern, lines[0])
        if match is None:
            raise AcceptanceError("Unsupported nextest timing summary")
        return match

    build = observed_line(
        "Finished `fast-build-nightly` profile",
        r"Finished `fast-build-nightly` profile \[[^\]\n]+\] target\(s\) in ([0-9.hms ]+)",
    )
    starting = observed_line(
        "Starting",
        r"Starting ([0-9]+) (test|tests) across ([0-9]+) (binary|binaries)(?: \(([^()\n]+) skipped\))?",
    )
    summary = observed_line(
        "Summary",
        r"Summary \[\s*([0-9.hms ]+)\] ([0-9]+) (test|tests) run: ([^\n]+)",
    )
    tests, binaries = int(starting[1]), int(starting[3])
    require(
        tests > 0
        and binaries > 0
        and starting[2] == ("test" if tests == 1 else "tests")
        and starting[4] == ("binary" if binaries == 1 else "binaries")
        and int(summary[2]) == tests
        and summary[3] == ("test" if tests == 1 else "tests"),
        "Nextest test counts disagree",
    )
    skip_counts: dict[str, int] = {}
    for part in starting[5].split(" and ") if starting[5] else ():
        match = re.fullmatch(r"([1-9][0-9]*) (test|tests|binary|binaries)", part)
        if match is None:
            raise AcceptanceError("Unsupported nextest skip counts")
        count = int(match[1])
        kind = "tests" if match[2] in {"test", "tests"} else "binaries"
        singular = "test" if kind == "tests" else "binary"
        require(
            kind not in skip_counts and match[2] == (singular if count == 1 else kind),
            "Invalid nextest skip counts",
        )
        skip_counts[kind] = count
    require(
        list(skip_counts) in ([], ["tests"], ["binaries"], ["tests", "binaries"]),
        "Invalid nextest skip-count order",
    )

    # A complete successful run ends with the skipped count. Failure fields or
    # unsupported reporter modes cannot be accepted from a matching prefix.
    success = re.fullmatch(
        r"([0-9]+) passed(?: \(([^()\n]+)\))?, ([0-9]+) skipped", summary[4]
    )
    if success is None:
        raise AcceptanceError("Nextest did not report a complete successful run")
    passed, skipped = int(success[1]), int(success[3])
    require(
        passed == tests and skipped == skip_counts.get("tests", 0),
        "Nextest completion counts disagree",
    )
    annotations: dict[str, int] = {}
    for part in success[2].split(", ") if success[2] else ():
        match = re.fullmatch(r"([1-9][0-9]*) (slow|flaky|leaky)", part)
        if match is None:
            raise AcceptanceError("Unsupported nextest passed-test annotation")
        require(
            match[2] not in annotations and int(match[1]) <= passed,
            "Invalid nextest passed-test annotation",
        )
        annotations[match[2]] = int(match[1])
    require(
        list(annotations)
        == [name for name in ("slow", "flaky", "leaky") if name in annotations],
        "Invalid nextest passed-test annotation order",
    )
    return {
        "cargo_reported_build_seconds": duration_seconds(build[1]),
        "nextest_reported_test_seconds": duration_seconds(summary[1]),
        "tests": tests,
        "binaries": binaries,
        "skipped": skipped,
        "skipped_binaries": skip_counts.get("binaries", 0),
        "passed": passed,
        "passed_annotations": annotations,
        "summary": summary[4],
    }


def junit_summary(contents: bytes) -> dict[str, Any]:
    require(
        len(contents) <= MAX_METADATA_BYTES
        and b"<!DOCTYPE" not in contents
        and b"<!ENTITY" not in contents,
        "Unsupported JUnit document",
    )
    root = ET.fromstring(contents)
    require(root.tag == "testsuites", "Unexpected JUnit root")
    identifier = root.get("uuid", "")
    parsed_uuid = uuid.UUID(identifier)
    require(
        str(parsed_uuid) == identifier and parsed_uuid.version == 4,
        "Invalid nextest run UUID",
    )
    started = root.get("timestamp", "")
    parsed_started = dt.datetime.fromisoformat(started)
    require(parsed_started.tzinfo is not None, "JUnit timestamp has no time zone")
    elapsed = float(root.get("time", "nan"))
    require(math.isfinite(elapsed) and elapsed >= 0, "Invalid JUnit elapsed time")
    inventory: list[tuple[str, str, str]] = []
    setup = 0
    failures = errors = skipped = flaky = 0
    for suite in root:
        require(suite.tag == "testsuite", "Unexpected JUnit element")
        name = suite.get("name", "")
        require(bool(name), "JUnit suite has no name")
        for case in suite.findall("testcase"):
            identity = (name, case.get("classname", ""), case.get("name", ""))
            require(all(identity), "JUnit testcase has no identity")
            if name.startswith("@setup-script:"):
                setup += 1
            else:
                inventory.append(identity)
            failures += len(list(case.iter("failure")))
            errors += len(list(case.iter("error")))
            skipped += len(list(case.iter("skipped")))
            flaky += len(list(case.iter("flakyFailure"))) + len(
                list(case.iter("flakyError"))
            )
    require(
        bool(inventory) and len(inventory) == len(set(inventory)),
        "JUnit report has an empty or duplicate test inventory",
    )
    ordered = [list(item) for item in sorted(inventory)]
    return {
        "run_uuid": identifier,
        "timestamp": started,
        "started_unix_ns": int(parsed_started.timestamp() * 1_000_000_000),
        "reported_seconds": elapsed,
        "testcases": len(inventory),
        "setup_script_cases": setup,
        "failures": failures,
        "errors": errors,
        "skipped": skipped,
        "flaky_attempts": flaky,
        "test_inventory": ordered,
        "test_inventory_sha256": cache.digest(ordered),
    }


def error_details(error: BaseException) -> dict[str, Any]:
    result: dict[str, Any] = {"kind": type(error).__name__}
    if isinstance(error, OSError):
        result["errno"] = error.errno
    if isinstance(error, subprocess.CalledProcessError):
        result["returncode"] = error.returncode
    if isinstance(error, (AcceptanceError, cache.CacheError)):
        result["message"] = str(error)
    return result


def setup_evidence(plan: dict[str, Any], environment: dict[str, str]) -> None:
    source = Path(plan["source"]["path"])
    uv = cache.executable_identity("uv", environment)
    version = cache.command(
        [uv["invocation"], "--version"],
        source,
        environment,
        "identify the Python bootstrap tool",
    )
    require(
        version.startswith("uv 0.12.10 (")
        and version.endswith("x86_64-unknown-linux-gnu)"),
        "Unexpected Python bootstrap tool",
    )
    raw = cache.command(
        [
            uv["invocation"],
            "--no-config",
            "--offline",
            "--no-python-downloads",
            "python",
            "list",
            "--only-installed",
            "--managed-python",
            "--output-format",
            "json",
        ],
        source,
        environment,
        "inspect the installed Python fixtures",
    )
    installed = json_value(raw.encode())
    require(
        isinstance(installed, list)
        and all(
            isinstance(item, dict)
            and isinstance(item.get("version"), str)
            and isinstance(item.get("key"), str)
            and isinstance(item.get("path"), str)
            for item in installed
        ),
        "Invalid installed-Python inventory",
    )
    requests_path = source / ".python-versions"
    requested = [
        line.partition("#")[0].strip()
        for line in bounded_bytes(requests_path, cache.MAX_CONFIG_BYTES)
        .decode()
        .splitlines()
    ]
    requested = [value for value in requested if value]
    require(
        bool(requested) and set(requested) <= {item["version"] for item in installed},
        "A required Python fixture is missing",
    )
    pythons = [
        {
            "version": item["version"],
            "key": item.get("key"),
            "executable": cache.file_identity(Path(item["path"])),
        }
        for item in installed
        if item["version"] in requested
    ]
    filesystems = {}
    for path, expected in (
        ("/btrfs", "btrfs"),
        ("/tmpfs", "tmpfs"),
        ("/minix", "minix"),
    ):
        actual = subprocess.check_output(
            ["findmnt", "-n", "-o", "FSTYPE", "--target", path], text=True, timeout=15
        ).strip()
        require(
            actual == expected and os.path.ismount(path),
            "The test filesystem layout differs",
        )
        filesystems[path] = {"type": actual, "device": Path(path).stat().st_dev}
    require(
        bool(environment.get("DBUS_SESSION_BUS_ADDRESS")),
        "The private test keyring is unavailable",
    )
    cache.verify_executable("uv", uv, environment)
    write_json(
        evidence_directory(plan) / "setup-evidence.json",
        {
            "source": actual_source(plan, environment),
            "uv": {**uv, "version": version},
            "python_requests": {**file_reference(requests_path), "values": requested},
            "installed_pythons": pythons,
            "nextest_configuration": file_reference(source / ".config/nextest.toml"),
            "pruner": file_reference(source / "scripts/prune_cargo_workspace_cache.py"),
            "filesystems": filesystems,
            "dbus_address_sha256": hashlib.sha256(
                environment["DBUS_SESSION_BUS_ADDRESS"].encode()
            ).hexdigest(),
        },
    )


def runtime_environment(
    plan: dict[str, Any], state: dict[str, Any], environment: dict[str, str]
) -> tuple[dict[str, str], dict[str, Any]]:
    child = {
        name: value
        for name, value in environment.items()
        if not name.startswith("UV_RUST_CACHE_")
    }
    writable_target = (
        plan["mode"] == "full"
        and plan["save_allowed"]
        and not state["restore"]["target"]["exact"]
    )
    selected = {
        **RUNTIME_ENVIRONMENT,
        "CARGO_NET_RETRY": "10",
        "CARGO_TERM_COLOR": "always",
        "RUSTUP_MAX_RETRIES": "10",
        "CARGO_UNSTABLE_MTIME_ON_USE": str(writable_target).lower(),
        "INSTA_PENDING_DIR": str(evidence_directory(plan) / "pending-snapshots"),
    }
    child.update(selected)
    start = read_json(evidence_directory(plan) / "start.json")
    require(
        child.get("TMPDIR") == start["temporary_directory"],
        "The workload temporary directory changed",
    )
    if plan["mode"] == "full":
        require(
            bool(child.get("DBUS_SESSION_BUS_ADDRESS")),
            "The private test keyring is unavailable",
        )
    observation: dict[str, Any] = {**selected, "TMPDIR": child["TMPDIR"]}
    if child.get("DBUS_SESSION_BUS_ADDRESS"):
        observation["DBUS_SESSION_BUS_ADDRESS_sha256"] = hashlib.sha256(
            child["DBUS_SESSION_BUS_ADDRESS"].encode()
        ).hexdigest()
    return child, observation


def run_command(
    directory: Path,
    name: str,
    command: list[str],
    source: Path,
    environment: dict[str, str],
    process_groups: Any,
    metadata: dict[str, Any],
    *,
    deadline_unix: float,
    before_launch: Callable[[], None],
    grace_seconds: float = 5,
) -> dict[str, Any]:
    """Retain one command's original outcome and prove its owned group is gone."""
    require(
        os.name == "posix" and name in {"fetch", "nextest", "prune"},
        "Unsupported acceptance command",
    )
    require(
        bool(command) and Path(command[0]).is_absolute(),
        "Expected the observed executable invocation",
    )
    require(
        math.isfinite(deadline_unix)
        and deadline_unix > 0
        and math.isfinite(grace_seconds)
        and grace_seconds > 0,
        "Invalid acceptance command deadline",
    )
    child_environment = dict(environment)
    expires = time.monotonic() + max(0, deadline_unix - time.time())
    path = directory / f"command-{name}.json"
    stdout_path = directory / f"{name}.stdout"
    stderr_path = directory / f"{name}.stderr"
    require(
        not path.exists() and not path.is_symlink(),
        "Acceptance command was already recorded",
    )
    state: dict[str, Any] = {
        "format": FORMAT + "-command",
        "version": VERSION,
        "name": name,
        "command": command,
        "cwd": str(source),
        "metadata": metadata,
        "deadline_unix": deadline_unix,
        "status": "verifying",
        "started_at": timestamp(),
        "launcher_pid": os.getpid(),
        "child_pid": None,
        "process_group": None,
        "command_returncode": None,
        "termination_complete": False,
        "cleanup_events": [],
    }
    child: subprocess.Popen[bytes] | None = None
    command_started_ns = command_finished_ns = None
    exit_code = 1
    with process_groups.RunSignals() as interruptions:
        with stdout_path.open("xb") as stdout, stderr_path.open("xb") as stderr:
            for stream in (stdout, stderr):
                os.fchmod(stream.fileno(), 0o600)
            try:
                replace_json(path, state)
                interruptions.check()
                before_launch()
                interruptions.check()
                if time.monotonic() >= expires:
                    raise subprocess.TimeoutExpired(command, 0)
                state.update(
                    status="starting",
                    verified_at=timestamp(),
                    command_started_unix_ns=time.time_ns(),
                )
                replace_json(path, state)
                print(
                    f"Starting acceptance {name} for {metadata.get('case', 'local-control')}",
                    flush=True,
                )
                command_started_ns = time.monotonic_ns()
                child = subprocess.Popen(
                    command,
                    cwd=source,
                    env=child_environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    start_new_session=True,
                )
                state.update(
                    status="running", child_pid=child.pid, process_group=child.pid
                )
                replace_json(path, state)
                returncode = interruptions.wait(
                    child, max(0, expires - time.monotonic())
                )
                command_finished_ns = time.monotonic_ns()
                state.update(
                    status="success" if returncode == 0 else "failed",
                    command_returncode=returncode,
                    command_finished_unix_ns=time.time_ns(),
                )
                exit_code = returncode if returncode >= 0 else 128 - returncode
            except subprocess.TimeoutExpired:
                state["status"] = "timed_out"
                exit_code = 124
            except process_groups.RunInterruptedError as error:
                state.update(status="interrupted", signal=error.signum)
                exit_code = 128 + error.signum
            except (
                OSError,
                ValueError,
                KeyError,
                TypeError,
                AttributeError,
                subprocess.SubprocessError,
            ) as error:
                state.update(status="failed", error=error_details(error))
                exit_code = 1
            finally:
                interruptions.interrupt_wait = False
                if command_started_ns is not None and command_finished_ns is None:
                    command_finished_ns = time.monotonic_ns()
                    state.setdefault("command_finished_unix_ns", time.time_ns())
                if child is not None:
                    state.update(
                        cleanup_status="running", cleanup_started_at=timestamp()
                    )
                    try:
                        replace_json(path, state)
                    finally:
                        try:
                            returncode, complete = process_groups.stop_process(
                                child,
                                grace_seconds,
                                process_group=child.pid,
                                events=state["cleanup_events"],
                            )
                        except (OSError, ValueError) as error:
                            returncode, complete = child.poll(), False
                            state["cleanup_error"] = error_details(error)
                    if state["command_returncode"] is None:
                        state["command_returncode"] = returncode
                    state.update(
                        termination_complete=complete,
                        cleanup_status="complete" if complete else "incomplete",
                        cleanup_finished_at=timestamp(),
                    )
                    if not complete and exit_code == 0:
                        state["status"] = "cleanup_failed"
                        exit_code = 1
                    if exit_code == 0 and any(
                        item["event"].get("stage") in {"term", "kill"}
                        and item["event"].get("result") == "sent"
                        for item in state["cleanup_events"]
                    ):
                        state["status"] = "left_descendants"
                        exit_code = 1
                else:
                    state["termination_complete"] = True
                for stream in (stdout, stderr):
                    stream.flush()
                    os.fsync(stream.fileno())
        if interruptions.signum is not None:
            state["received_signal"] = interruptions.signum
            if exit_code == 0:
                state.update(status="interrupted", signal=interruptions.signum)
                exit_code = 128 + interruptions.signum
        state.update(
            command_started_monotonic_ns=command_started_ns,
            command_finished_monotonic_ns=command_finished_ns,
            command_elapsed_ns=command_finished_ns - command_started_ns
            if command_started_ns is not None and command_finished_ns is not None
            else None,
            stdout=regular_file_reference(stdout_path),
            stderr=regular_file_reference(stderr_path),
            exit_code=exit_code,
            finished_at=timestamp(),
        )
        replace_json(path, state)
    print(f"Acceptance {name}: {state['status']}", flush=True)
    return state


def command_succeeded(record: dict[str, Any]) -> bool:
    return (
        record.get("status") == "success"
        and record.get("exit_code") == 0
        and record.get("command_returncode") == 0
        and record.get("termination_complete") is True
        and type(record.get("child_pid")) is int
        and record["child_pid"] > 0
        and record.get("process_group") == record["child_pid"]
        and record.get("received_signal") is None
        and any(
            item["event"].get("syscall") == "killpg"
            and item["event"].get("process_group") == record["process_group"]
            and item["event"].get("signal") == 0
            and item["event"].get("result") == "absent"
            for item in record.get("cleanup_events", [])
        )
        and not any(
            item["event"].get("stage") in {"term", "kill"}
            and item["event"].get("result") == "sent"
            for item in record.get("cleanup_events", [])
        )
    )


def file_reference(path: Path) -> dict[str, Any]:
    return regular_file_reference(path)


def executable_artifacts(records: dict[str, Any], target: Path) -> list[dict[str, Any]]:
    result = []
    for name in sorted(
        {
            item["record"]["executable"]
            for item in records["records"]
            if item["record"].get("executable") is not None
        }
    ):
        path = Path(name)
        require(
            inside_target(name, target)
            and path.resolve(strict=True).is_relative_to(target),
            "Compiled executable escapes the selected target",
        )
        result.append({"path": str(path.relative_to(target)), **file_reference(path)})
    require(bool(result), "Cargo reported no executable artifacts")
    return result


def validate_junit_invocation(
    summary: dict[str, Any], command: dict[str, Any], times: dict[str, Any]
) -> None:
    # quick-junit writes RFC3339 milliseconds; allow one second for truncation.
    require(
        command["command_started_unix_ns"] - 1_000_000_000
        <= summary["started_unix_ns"]
        <= command["command_finished_unix_ns"] + 1_000_000_000,
        "JUnit belongs to another invocation",
    )
    require(
        summary["testcases"] == times["tests"] == times["passed"]
        and summary["failures"] == summary["errors"] == summary["skipped"] == 0,
        "JUnit and nextest completion disagree",
    )


def preserve_junit(plan: dict[str, Any], target: Path) -> dict[str, Any] | None:
    source = target / "nextest/ci-linux/junit.xml"
    if not source.exists() and not source.is_symlink():
        return None
    require(
        stat.S_ISREG(source.lstat().st_mode) and not source.is_symlink(),
        "JUnit is not an ordinary report file",
    )
    before = source.stat()
    contents = bounded_bytes(source, MAX_METADATA_BYTES)
    checksum = write_bytes(evidence_directory(plan) / "junit.xml", contents)
    return {
        "source_path": str(source),
        "sha256": checksum,
        "size": len(contents),
        "mtime_ns": before.st_mtime_ns,
        "summary": junit_summary(contents),
    }


def analyze_workload(
    plan: dict[str, Any], command: dict[str, Any], target: Path
) -> dict[str, Any]:
    directory = evidence_directory(plan)
    result: dict[str, Any] = {
        "command_complete": command_succeeded(command),
        "completed_full_workload": False,
    }
    try:
        records = cargo_records(
            bounded_bytes(directory / "nextest.stdout", MAX_LOG_BYTES),
            Path(plan["source"]["path"]),
            target,
            require_complete=result["command_complete"],
        )
        result["cargo_records"] = {
            "path": "cargo-artifacts.json",
            "sha256": write_json(directory / "cargo-artifacts.json", records),
            "compiler_artifact_records": records["compiler_artifact_records"],
            "fresh_records": records["fresh_records"],
            "nonfresh_records": records["nonfresh_records"],
            "unit_multiset": records["unit_multiset"],
            "logical_unit_multiset": records["logical_unit_multiset"],
        }
        if result["command_complete"]:
            result["executable_artifacts"] = executable_artifacts(records, target)
    except (OSError, ValueError, KeyError, TypeError) as error:
        result["cargo_error"] = error_details(error)
    try:
        result["reported_times"] = reported_times(
            bounded_bytes(directory / "nextest.stderr", MAX_LOG_BYTES)
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        result["timing_error"] = error_details(error)
    try:
        junit = preserve_junit(plan, target)
        result["junit"] = junit
        if result["command_complete"]:
            if junit is None:
                raise AcceptanceError("Nextest did not write a JUnit report")
            validate_junit_invocation(
                junit["summary"], command, result["reported_times"]
            )
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError) as error:
        result["junit_error"] = error_details(error)
    if plan["expected_target"] == "malformed":
        try:
            contents = bounded_bytes(
                target / ".rustc_info.json", cache.MAX_CONFIG_BYTES
            )
            result["rustc_info_after"] = {
                "sha256": write_bytes(directory / "rustc-info-after.json", contents),
                "size": len(contents),
                "valid_json": isinstance(json_value(contents), dict),
                "differs_from_fixture": contents != MALFORMED_RUSTC_INFO,
            }
        except (OSError, ValueError, TypeError) as error:
            result["rustc_info_error"] = error_details(error)
    result["completed_full_workload"] = (
        result["command_complete"]
        and all(
            name not in result
            for name in ("cargo_error", "timing_error", "junit_error")
        )
        and (
            plan["expected_target"] != "malformed"
            or (
                result.get("rustc_info_after", {}).get("valid_json") is True
                and result["rustc_info_after"]["differs_from_fixture"]
            )
        )
    )
    write_json(directory / "workload-evidence.json", result)
    return result


def run_work(
    plan: dict[str, Any],
    manifest_path: Path,
    manifest_sha256: str,
    process_repository: Path,
    environment: dict[str, str],
    github_output: Path,
) -> int:
    require(plan["mode"] in {"seed", "full"}, "This case has no normal Cargo workload")
    state = manifest_state(plan, manifest_path, manifest_sha256, environment)
    directory = evidence_directory(plan)
    target = Path(state["identity"]["identity"]["paths"]["target"])
    source = Path(plan["source"]["path"])
    child_environment, selected_environment = runtime_environment(
        plan, state, environment
    )
    process_groups, owner = load_process_groups(process_repository, child_environment)
    mode = "fetch" if plan["mode"] == "seed" else "nextest"
    cargo = cache.common_outputs(state["identity"])["cargo"]
    command = (
        [cargo, "fetch", "--locked"]
        if mode == "fetch"
        else [cargo, *cache.WORKLOAD["cargo_arguments"], "--cargo-message-format=json"]
    )
    if mode == "nextest":
        report = target / "nextest/ci-linux/junit.xml"
        require(
            not report.exists() and not report.is_symlink(),
            "A pre-existing JUnit report would make completion ambiguous",
        )
        record_inventory(plan, "restored", target)
        if plan["save_allowed"] and not state["restore"]["target"]["exact"]:
            write_bytes(directory / "cargo-cache-marker", b"")
    phase(plan, "workload", "start")

    def verify() -> None:
        require(
            manifest_state(plan, manifest_path, manifest_sha256, child_environment)
            == state,
            "Workload identity changed before launch",
        )

    record = run_command(
        directory,
        mode,
        command,
        source,
        child_environment,
        process_groups,
        {
            "case": plan["case"],
            "source": plan["source"],
            "plan_sha256": hashlib.sha256(cache.json_bytes(plan)).hexdigest(),
            "restore_manifest_sha256": manifest_sha256,
            "runtime_environment": selected_environment,
            "process_owner": owner,
        },
        deadline_unix=plan["deadline_unix"],
        before_launch=verify,
    )
    complete = command_succeeded(record)
    try:
        verify()
        write_json(
            directory / "post-workload-identity.json",
            {"unchanged": True, "source": actual_source(plan, child_environment)},
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        complete = False
        write_json(
            directory / "post-workload-identity.json",
            {"unchanged": False, "error": error_details(error)},
        )
    if mode == "nextest":
        complete = (
            analyze_workload(plan, record, target)["completed_full_workload"]
            and complete
        )
        record_inventory(plan, "built", target)
    phase(plan, "workload", "end", outcome="success" if complete else "failure")
    cache.write_outputs(
        github_output,
        {
            "workload-complete": str(complete).lower(),
            "save-eligible": str(
                complete
                and all(expectations(plan, state).values())
                and plan["save_allowed"]
            ).lower(),
        },
    )
    return record["exit_code"] if record["exit_code"] else 0 if complete else 1


def run_pruner(
    plan: dict[str, Any],
    manifest_path: Path,
    manifest_sha256: str,
    process_repository: Path,
    environment: dict[str, str],
) -> int:
    state = manifest_state(plan, manifest_path, manifest_sha256, environment)
    require(
        plan["mode"] == "full"
        and plan["save_allowed"]
        and not state["restore"]["target"]["exact"]
        and all(expectations(plan, state).values()),
        "The compiled cache is not eligible for pruning",
    )
    directory = evidence_directory(plan)
    require(
        read_json(directory / "workload-evidence.json")["completed_full_workload"]
        is True
        and read_json(directory / "post-workload-identity.json")["unchanged"] is True,
        "The complete workload must precede pruning",
    )
    source = Path(plan["source"]["path"])
    target = Path(state["identity"]["identity"]["paths"]["target"])
    child_environment, selected_environment = runtime_environment(
        plan, state, environment
    )
    process_groups, owner = load_process_groups(process_repository, child_environment)
    python = cache.file_identity(Path(sys.executable))
    script = source / "scripts/prune_cargo_workspace_cache.py"
    script_identity = cache.file_identity(script, limit=cache.MAX_CONFIG_BYTES)
    marker = directory / "cargo-cache-marker"
    require(
        marker.is_file() and not marker.is_symlink(),
        "Missing pre-build freshness marker",
    )

    def verify() -> None:
        require(
            manifest_state(plan, manifest_path, manifest_sha256, child_environment)
            == state
            and cache.file_identity(Path(sys.executable)) == python
            and cache.file_identity(script, limit=cache.MAX_CONFIG_BYTES)
            == script_identity,
            "Pruner source or tools changed",
        )

    record = run_command(
        directory,
        "prune",
        [python["invocation"], str(script), str(target / cache.PROFILE), str(marker)],
        source,
        child_environment,
        process_groups,
        {
            "case": plan["case"],
            "source": plan["source"],
            "plan_sha256": hashlib.sha256(cache.json_bytes(plan)).hexdigest(),
            "restore_manifest_sha256": manifest_sha256,
            "runtime_environment": selected_environment,
            "process_owner": owner,
            "python": python,
            "script": script_identity,
            "marker_mtime_ns": marker.stat().st_mtime_ns,
        },
        deadline_unix=plan["deadline_unix"],
        before_launch=verify,
    )
    if command_succeeded(record):
        verify()
        record_inventory(plan, "pruned", target)
    return record["exit_code"]


def seed_save_plan(
    plan: dict[str, Any],
    manifest_path: Path,
    manifest_sha256: str,
    environment: dict[str, str],
    github_output: Path,
) -> None:
    state = manifest_state(plan, manifest_path, manifest_sha256, environment)
    directory = evidence_directory(plan)
    require(
        plan["mode"] == "seed"
        and plan["save_allowed"]
        and all(expectations(plan, state).values()),
        "The isolated downloads fixture is not eligible for publication",
    )
    require(
        command_succeeded(read_json(directory / "command-fetch.json"))
        and read_json(directory / "post-workload-identity.json")["unchanged"] is True,
        "A successful verified fetch must precede downloads publication",
    )
    decisions = cache.save_plan(state, environment, "success")
    require(
        decisions["save-downloads"] == "true", "The downloads fixture is already exact"
    )
    observation = {
        "kind": "isolated-downloads-fixture",
        "source_manifest_sha256": manifest_sha256,
        "downloads_key": decisions["downloads-key"],
        "downloads_paths": list(cache.DOWNLOAD_PATHS),
        "source": plan["source"],
        "normal_nextest_workload_ran": False,
    }
    write_json(directory / "seed-publication-plan.json", observation)
    cache.write_outputs(
        github_output,
        {key: decisions[key] for key in ("downloads-key", "downloads-paths")},
    )


def relative_components(value: Any) -> tuple[str, ...]:
    require(
        isinstance(value, str)
        and bool(value)
        and not any(
            ord(character) < 32 or ord(character) == 127 for character in value
        ),
        "Invalid evidence-relative path",
    )
    parts = tuple(value.split("/"))
    require(
        all(part not in {"", ".", ".."} for part in parts),
        "Invalid evidence-relative path",
    )
    return parts


def checked_payload_inventory(
    value: dict[str, Any], *, hash_contents: bool
) -> dict[str, Any]:
    require(
        isinstance(value, dict)
        and set(value)
        == {"entries", "files", "bytes", "symlinks", "content_tree_sha256"}
        and all(
            type(value[name]) is int and value[name] >= 0
            for name in ("files", "bytes", "symlinks")
        ),
        "Invalid compiled-cache inventory",
    )
    if not hash_contents:
        require(
            value["entries"] is None and value["content_tree_sha256"] is None,
            "Unexpected compiled-cache content inventory",
        )
        return value
    entries = value["entries"]
    require(
        isinstance(entries, list)
        and len(entries) <= MAX_PAYLOAD_ENTRIES
        and value["content_tree_sha256"] == cache.digest(entries),
        "Compiled-cache content digest differs",
    )
    paths = []
    files = size = symlinks = 0
    for item in entries:
        require(isinstance(item, dict), "Invalid compiled-cache inventory entry")
        parts = relative_components(item.get("path"))
        require(
            (parts == (".rustc_info.json",) or parts[0] == cache.PROFILE)
            and (len(parts) < 2 or parts[1] not in {"incremental", ".cargo-lock"})
            and type(item.get("mode")) is int
            and 0 <= item["mode"] <= 0o7777,
            "Compiled-cache entry is outside the declared payload",
        )
        paths.append(item["path"])
        kind = item.get("kind")
        if kind == "file":
            require(
                set(item) == {"path", "mode", "kind", "size", "sha256"}
                and type(item["size"]) is int
                and item["size"] >= 0
                and isinstance(item["sha256"], str)
                and cache.SHA256.fullmatch(item["sha256"]) is not None,
                "Invalid compiled-cache file identity",
            )
            files += 1
            size += item["size"]
        elif kind == "directory":
            require(
                set(item) == {"path", "mode", "kind"},
                "Invalid compiled-cache directory identity",
            )
        elif kind == "symlink":
            require(
                set(item) == {"path", "mode", "kind", "target"}
                and isinstance(item["target"], str)
                and bool(item["target"]),
                "Invalid compiled-cache symlink identity",
            )
            symlinks += 1
        else:
            raise AcceptanceError("Unsupported compiled-cache inventory entry")
    require(
        paths == sorted(set(paths))
        and value["files"] == files
        and value["bytes"] == size
        and value["symlinks"] == symlinks,
        "Compiled-cache inventory counts or paths differ",
    )
    return value


def changed_payload_paths(before: dict[str, Any], after: dict[str, Any]) -> list[str]:
    old = {item["path"]: item for item in before["entries"]}
    new = {item["path"]: item for item in after["entries"]}
    require(
        len(old) == len(before["entries"]) and len(new) == len(after["entries"]),
        "Duplicate payload inventory path",
    )
    return sorted(
        path for path in old.keys() | new.keys() if old.get(path) != new.get(path)
    )


def prepare_malformed_fixture(
    plan: dict[str, Any],
    manifest_path: Path,
    manifest_sha256: str,
    environment: dict[str, str],
    github_output: Path,
) -> None:
    require(
        plan["case"] == "malformed-cache-fixture" and plan["stage"] == STAGES[1],
        "Malformed-cache publication is outside the selected stage",
    )
    state = manifest_state(plan, manifest_path, manifest_sha256, environment)
    require(
        all(expectations(plan, state).values()) and not plan["save_allowed"],
        "The malformed fixture requires an exact read-only candidate restore",
    )
    directory = evidence_directory(plan)
    target = Path(state["identity"]["identity"]["paths"]["target"])
    metadata_path = target / ".rustc_info.json"
    original_stat = metadata_path.lstat()
    require(
        stat.S_ISREG(original_stat.st_mode) and not metadata_path.is_symlink(),
        "The original Cargo metadata is not an ordinary file",
    )
    original = bounded_bytes(metadata_path, cache.MAX_CONFIG_BYTES)
    require(
        isinstance(json_value(original), dict),
        "The original Cargo metadata is already malformed",
    )
    original_sha256 = write_bytes(directory / "rustc-info-original.json", original)
    before = payload_inventory(target, hash_contents=True)
    before_sha256 = write_json(directory / "fault-payload-before.json", before)
    require(
        bounded_bytes(metadata_path, cache.MAX_CONFIG_BYTES) == original,
        "Cargo metadata changed before fault preparation",
    )
    with tempfile.NamedTemporaryFile(
        prefix=".uv-cache-fixture-", dir=target, delete=False
    ) as stream:
        temporary = Path(stream.name)
        os.fchmod(stream.fileno(), stat.S_IMODE(original_stat.st_mode))
        stream.write(MALFORMED_RUSTC_INFO)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(metadata_path)
    after = payload_inventory(target, hash_contents=True)
    after_sha256 = write_json(directory / "fault-payload-after.json", after)
    changed = changed_payload_paths(before, after)
    require(
        changed == [".rustc_info.json"]
        and bounded_bytes(metadata_path, cache.MAX_CONFIG_BYTES)
        == MALFORMED_RUSTC_INFO,
        "Malformed fixture changed another compiled-cache path",
    )
    require(
        manifest_state(plan, manifest_path, manifest_sha256, environment) == state,
        "Source or tools changed during fault preparation",
    )
    source = plan["source"]
    identity = cache.make_identity(
        Path(plan["workspace"]),
        Path(source["path"]),
        source["repository"],
        source["commit"],
        False,
        environment,
        key_namespace=plan["fault_namespace"],
    )
    require(
        identity["identity"] == state["identity"]["identity"]
        and identity["policy"]
        == {"save_allowed": False, "key_namespace": plan["fault_namespace"]},
        "Fault namespace changed the source identity",
    )
    identity_sha256 = write_json(directory / "fault-identity.json", identity)
    write_json(
        directory / "fault-fixture.json",
        {
            "kind": "intentionally-malformed-target-fixture",
            "source_manifest_sha256": manifest_sha256,
            "source": source,
            "normal_nextest_workload_ran": False,
            "original_metadata_sha256": original_sha256,
            "malformed_metadata_sha256": hashlib.sha256(
                MALFORMED_RUSTC_INFO
            ).hexdigest(),
            "before_inventory_sha256": before_sha256,
            "after_inventory_sha256": after_sha256,
            "changed_paths": changed,
            "identity_sha256": identity_sha256,
            "target_key": identity["keys"]["target"],
            "target_paths": list(cache.TARGET_PATHS),
        },
    )
    cache.write_outputs(
        github_output,
        {
            "target-key": identity["keys"]["target"],
            "target-paths": "\n".join(cache.TARGET_PATHS),
            "fault-fixture-ready": "true",
        },
    )


def step_outcomes(values: list[str]) -> dict[str, str]:
    result: dict[str, str] = {}
    for value in values:
        name, separator, outcome = value.partition("=")
        require(
            bool(separator) and name in OUTCOME_NAMES and name not in result,
            "Invalid or duplicate workflow outcome",
        )
        outcome = outcome or "unknown"
        require(outcome in OUTCOMES, "Invalid workflow outcome")
        result[name] = outcome
    return {name: result.get(name, "unknown") for name in sorted(OUTCOME_NAMES)}


def evidence_inventory(directory: Path) -> list[dict[str, Any]]:
    entries = []
    total = 0
    for path in sorted(directory.rglob("*")):
        require(not path.is_symlink(), "Evidence contains a symbolic link")
        if path.is_dir() or path == directory / "result.json":
            continue
        reference = regular_file_reference(path, limit=MAX_EVIDENCE_FILE_BYTES)
        total += reference["size"]
        require(
            total <= MAX_EVIDENCE_BYTES,
            "Evidence exceeds its upload byte budget",
        )
        entries.append({"path": path.relative_to(directory).as_posix(), **reference})
        require(
            len(entries) <= MAX_EVIDENCE_FILES,
            "Evidence inventory exceeds its file limit",
        )
    return entries


def recorded_phase(plan: dict[str, Any], directory: Path, name: str) -> dict[str, Any]:
    start = read_json(directory / f"phase-{name}-start.json")
    end = read_json(directory / f"phase-{name}-end.json")
    require(
        start["case"] == end["case"] == plan["case"]
        and start["phase"] == end["phase"] == name
        and start["event"] == "start"
        and end["event"] == "end"
        and type(start["monotonic_ns"]) is int
        and type(end["monotonic_ns"]) is int
        and end["monotonic_ns"] >= start["monotonic_ns"]
        and end["elapsed_ns"] == end["monotonic_ns"] - start["monotonic_ns"]
        and end["outcome"] in OUTCOMES,
        "Invalid phase evidence",
    )
    return {
        "started_monotonic_ns": start["monotonic_ns"],
        "finished_monotonic_ns": end["monotonic_ns"],
        "elapsed_ns": end["elapsed_ns"],
        "outcome": end["outcome"],
    }


def expected_runtime(
    plan: dict[str, Any],
    state: dict[str, Any],
    start: dict[str, Any],
    setup: dict[str, Any],
) -> dict[str, Any]:
    return {
        **RUNTIME_ENVIRONMENT,
        "CARGO_NET_RETRY": "10",
        "CARGO_TERM_COLOR": "always",
        "RUSTUP_MAX_RETRIES": "10",
        "CARGO_UNSTABLE_MTIME_ON_USE": str(
            plan["mode"] == "full"
            and plan["save_allowed"]
            and not state["restore"]["target"]["exact"]
        ).lower(),
        "INSTA_PENDING_DIR": str(
            Path(plan["workspace"]) / "results" / plan["case"] / "pending-snapshots"
        ),
        "TMPDIR": start["temporary_directory"],
        "DBUS_SESSION_BUS_ADDRESS_sha256": setup["dbus_address_sha256"],
    }


def recorded_command(
    plan: dict[str, Any], directory: Path, state: dict[str, Any], name: str
) -> dict[str, Any]:
    record = read_json(directory / f"command-{name}.json")
    start = read_json(directory / "start.json")
    setup = read_json(directory / "setup-evidence.json")
    metadata = record["metadata"]
    target = Path(state["identity"]["identity"]["paths"]["target"])
    cargo = cache.common_outputs(state["identity"])["cargo"]
    if name == "fetch":
        command = [cargo, "fetch", "--locked"]
    elif name == "nextest":
        command = [
            cargo,
            *cache.WORKLOAD["cargo_arguments"],
            "--cargo-message-format=json",
        ]
    else:
        require(
            name == "prune"
            and {key: metadata["script"][key] for key in ("sha256", "size")}
            == setup["pruner"],
            "Pruner source evidence differs",
        )
        command = [
            metadata["python"]["invocation"],
            str(
                Path(plan["source"]["path"]) / "scripts/prune_cargo_workspace_cache.py"
            ),
            str(target / cache.PROFILE),
            str(
                Path(plan["workspace"])
                / "results"
                / plan["case"]
                / "cargo-cache-marker"
            ),
        ]
    require(
        record["format"] == FORMAT + "-command"
        and type(record["version"]) is int
        and record["version"] == VERSION
        and record["name"] == name
        and record["command"] == command
        and record["cwd"] == plan["source"]["path"]
        and record["deadline_unix"] == plan["deadline_unix"],
        "Command does not match its declared workload",
    )
    require(
        metadata["case"] == plan["case"]
        and metadata["source"] == plan["source"]
        and metadata["plan_sha256"] == file_reference(directory / "plan.json")["sha256"]
        and metadata["restore_manifest_sha256"]
        == file_reference(directory / "restore-manifest.json")["sha256"]
        and metadata["runtime_environment"]
        == expected_runtime(plan, state, start, setup)
        and metadata["process_owner"]
        == {
            **PROCESS_OWNER,
            "repository_path": str(Path(plan["workspace"]) / "process-owner"),
        },
        "Command provenance differs from its source or environment",
    )
    require(
        record["stdout"] == file_reference(directory / f"{name}.stdout")
        and record["stderr"] == file_reference(directory / f"{name}.stderr"),
        "Command output changed",
    )
    if command_succeeded(record):
        require(
            type(record["command_started_monotonic_ns"]) is int
            and type(record["command_finished_monotonic_ns"]) is int
            and record["command_elapsed_ns"]
            == record["command_finished_monotonic_ns"]
            - record["command_started_monotonic_ns"]
            >= 0
            and type(record["command_started_unix_ns"]) is int
            and type(record["command_finished_unix_ns"]) is int
            and record["command_finished_unix_ns"] >= record["command_started_unix_ns"],
            "Command timing evidence is invalid",
        )
    return record


def checked_fingerprints(directory: Path, name: str) -> dict[str, Any]:
    value = read_json(
        directory / f"fingerprints-{name}.json", MAX_FINGERPRINT_JSON_BYTES
    )
    entries = value["entries"]
    require(
        isinstance(entries, list)
        and type(value["files"]) is int
        and value["files"] == len(entries)
        and type(value["bytes"]) is int
        and 0 <= value["bytes"] <= MAX_FINGERPRINT_BYTES
        and value["sha256"] == cache.digest(entries),
        "Fingerprint inventory is incomplete",
    )
    total = 0
    configurations = []
    paths = []
    for item in entries:
        require(isinstance(item, dict), "Invalid fingerprint entry")
        parts = relative_components(item.get("path"))
        require(
            len(parts) > 2 and parts[:2] == (cache.PROFILE, ".fingerprint"),
            "Fingerprint entry is outside its declared directory",
        )
        paths.append(item["path"])
        contents = base64.b64decode(item["contents_base64"], validate=True)
        total += len(contents)
        require(
            type(item["size"]) is int
            and len(contents) == item["size"]
            and len(contents) <= cache.MAX_CONFIG_BYTES
            and hashlib.sha256(contents).hexdigest() == item["sha256"]
            and total <= MAX_FINGERPRINT_BYTES,
            "Fingerprint bytes differ",
        )
        expected: dict[str, Any] = {
            "path": item["path"],
            "size": len(contents),
            "sha256": hashlib.sha256(contents).hexdigest(),
            "contents_base64": base64.b64encode(contents).decode("ascii"),
        }
        if Path(parts[-1]).suffix == ".json":
            try:
                expected["configuration"] = json_value(contents)
            except (ValueError, UnicodeError):
                expected["configuration_error"] = "invalid-json"
        require(item == expected, "Fingerprint configuration differs")
        if "configuration" in expected:
            configurations.append(
                {"path": item["path"], "configuration": item["configuration"]}
            )
    require(
        total == value["bytes"] and paths == sorted(set(paths)),
        "Fingerprint byte count or paths differ",
    )
    return {
        "files": value["files"],
        "bytes": value["bytes"],
        "raw_inventory_sha256": value["sha256"],
        "configuration_inventory_sha256": cache.digest(configurations),
    }


def checked_workload(
    plan: dict[str, Any], directory: Path, state: dict[str, Any]
) -> dict[str, Any]:
    command = recorded_command(plan, directory, state, "nextest")
    value = read_json(directory / "workload-evidence.json")
    require(
        command_succeeded(command)
        and value["command_complete"] is True
        and value["completed_full_workload"] is True,
        "The full nextest operation did not complete",
    )
    target = Path(state["identity"]["identity"]["paths"]["target"])
    records = cargo_records(
        bounded_bytes(directory / "nextest.stdout", MAX_LOG_BYTES),
        Path(plan["source"]["path"]),
        target,
        require_complete=True,
    )
    require(
        read_json(directory / "cargo-artifacts.json") == records
        and file_reference(directory / "cargo-artifacts.json")["sha256"]
        == value["cargo_records"]["sha256"],
        "Cargo artifact evidence differs from stdout",
    )
    for name in (
        "compiler_artifact_records",
        "fresh_records",
        "nonfresh_records",
        "unit_multiset",
        "logical_unit_multiset",
    ):
        require(
            value["cargo_records"][name] == records[name],
            "Cargo artifact summary differs",
        )
    times = reported_times(bounded_bytes(directory / "nextest.stderr", MAX_LOG_BYTES))
    require(
        times == value["reported_times"], "Nextest timing summary differs from stderr"
    )
    junit = junit_summary(bounded_bytes(directory / "junit.xml", MAX_METADATA_BYTES))
    require(
        junit == value["junit"]["summary"]
        and file_reference(directory / "junit.xml")["sha256"]
        == value["junit"]["sha256"],
        "JUnit evidence differs",
    )
    validate_junit_invocation(junit, command, times)
    executable_paths = sorted(
        {
            str(Path(item["record"]["executable"]).relative_to(target))
            for item in records["records"]
            if item["record"].get("executable") is not None
        }
    )
    executables = value["executable_artifacts"]
    require(
        executable_paths == [item["path"] for item in executables]
        and all(
            cache.SHA256.fullmatch(item["sha256"]) is not None
            and type(item["size"]) is int
            and item["size"] > 0
            for item in executables
        ),
        "Post-command executable inventory differs",
    )
    if plan["expected_target"] == "malformed":
        contents = bounded_bytes(
            directory / "rustc-info-after.json", cache.MAX_CONFIG_BYTES
        )
        require(
            isinstance(json_value(contents), dict)
            and contents != MALFORMED_RUSTC_INFO
            and value["rustc_info_after"]
            == {
                "sha256": hashlib.sha256(contents).hexdigest(),
                "size": len(contents),
                "valid_json": True,
                "differs_from_fixture": True,
            },
            "Cargo did not replace the malformed metadata",
        )
    return {
        "command_elapsed_ns": command["command_elapsed_ns"],
        "reported_times": times,
        "cargo": {
            name: records[name]
            for name in (
                "compiler_artifact_records",
                "fresh_records",
                "nonfresh_records",
                "unit_multiset",
                "logical_unit_multiset",
            )
        },
        "junit": {
            key: junit[key]
            for key in (
                "run_uuid",
                "testcases",
                "setup_script_cases",
                "test_inventory_sha256",
            )
        },
        "fingerprints_restored": checked_fingerprints(directory, "restored"),
        "fingerprints_built": checked_fingerprints(directory, "built"),
        "payload_restored": checked_payload_inventory(
            read_json(directory / "payload-restored.json"), hash_contents=False
        ),
        "payload_built": checked_payload_inventory(
            read_json(directory / "payload-built.json"), hash_contents=False
        ),
    }


def checked_fault_fixture(
    plan: dict[str, Any], directory: Path, state: dict[str, Any]
) -> dict[str, Any]:
    value = read_json(directory / "fault-fixture.json")
    before = checked_payload_inventory(
        read_json(directory / "fault-payload-before.json", MAX_FINGERPRINT_JSON_BYTES),
        hash_contents=True,
    )
    after = checked_payload_inventory(
        read_json(directory / "fault-payload-after.json", MAX_FINGERPRINT_JSON_BYTES),
        hash_contents=True,
    )
    identity = read_json(directory / "fault-identity.json")
    require(
        value["kind"] == "intentionally-malformed-target-fixture"
        and value["normal_nextest_workload_ran"] is False
        and value["source"] == plan["source"]
        and value["source_manifest_sha256"]
        == file_reference(directory / "restore-manifest.json")["sha256"],
        "Fault fixture belongs to another source",
    )
    require(
        value["before_inventory_sha256"]
        == file_reference(directory / "fault-payload-before.json")["sha256"]
        and value["after_inventory_sha256"]
        == file_reference(directory / "fault-payload-after.json")["sha256"]
        and value["changed_paths"]
        == changed_payload_paths(before, after)
        == [".rustc_info.json"],
        "Fault fixture changed an unrelated payload path",
    )
    original = bounded_bytes(
        directory / "rustc-info-original.json", cache.MAX_CONFIG_BYTES
    )
    require(
        isinstance(json_value(original), dict)
        and hashlib.sha256(original).hexdigest() == value["original_metadata_sha256"]
        and value["malformed_metadata_sha256"]
        == hashlib.sha256(MALFORMED_RUSTC_INFO).hexdigest(),
        "Fault metadata evidence differs",
    )
    old_metadata = {item["path"]: item for item in before["entries"]}.get(
        ".rustc_info.json"
    )
    new_metadata = {item["path"]: item for item in after["entries"]}.get(
        ".rustc_info.json"
    )
    require(
        isinstance(old_metadata, dict)
        and isinstance(new_metadata, dict)
        and old_metadata["kind"] == new_metadata["kind"] == "file"
        and old_metadata["mode"] == new_metadata["mode"]
        and old_metadata["size"] == len(original)
        and old_metadata["sha256"] == value["original_metadata_sha256"]
        and new_metadata["size"] == len(MALFORMED_RUSTC_INFO)
        and new_metadata["sha256"] == value["malformed_metadata_sha256"],
        "Fault payload does not contain the retained metadata bytes",
    )
    require(
        identity
        == {
            **state["identity"],
            "policy": {"save_allowed": False, "key_namespace": plan["fault_namespace"]},
            "keys": cache.cache_keys(
                state["identity"]["identity"], plan["fault_namespace"]
            ),
        }
        and value["identity_sha256"]
        == file_reference(directory / "fault-identity.json")["sha256"]
        and value["target_key"] == identity["keys"]["target"]
        and value["target_paths"] == list(cache.TARGET_PATHS),
        "Fault publication identity differs",
    )
    return {
        "target_key": value["target_key"],
        "original_metadata_sha256": value["original_metadata_sha256"],
        "malformed_metadata_sha256": value["malformed_metadata_sha256"],
        "before_content_tree_sha256": before["content_tree_sha256"],
        "after_content_tree_sha256": after["content_tree_sha256"],
    }


def checked_phase_order(
    plan: dict[str, Any], directory: Path, state: dict[str, Any]
) -> list[str]:
    names = ["setup", "restore"]
    writable = (
        plan["mode"] == "full"
        and plan["save_allowed"]
        and not state["restore"]["target"]["exact"]
    )
    names.append("fault" if plan["mode"] == "fault-fixture" else "workload")
    if writable:
        names.append("prune")
    if writable or plan["mode"] in {"seed", "fault-fixture"}:
        names.append("save")
    require(
        {path.name for path in directory.glob("phase-*.json")}
        == {
            f"phase-{name}-{event}.json" for name in names for event in ("start", "end")
        },
        "The recorded acceptance phase set differs",
    )
    phases = [recorded_phase(plan, directory, name) for name in names]
    require(
        all(item["outcome"] == "success" for item in phases)
        and all(
            previous["finished_monotonic_ns"] <= current["started_monotonic_ns"]
            for previous, current in pairwise(phases)
        ),
        "Acceptance phases overlap or ran out of order",
    )
    return names


def evaluate_case(
    plan: dict[str, Any], directory: Path, outcomes: dict[str, str], job_status: str
) -> dict[str, Any]:
    checks = {
        "prior_job_success": job_status == "success",
        "setup": False,
        "source_unchanged": False,
        "restore": False,
        "workload_or_fixture": False,
        "prune": False,
        "save_policy": False,
        "phase_order": False,
    }
    errors: dict[str, Any] = {}
    summary: dict[str, Any] = {"phases": {}}
    state: dict[str, Any] | None = None

    def failed(name: str, error: BaseException) -> None:
        errors[name] = error_details(error)

    def successful_phase(name: str) -> dict[str, Any]:
        value = recorded_phase(plan, directory, name)
        summary["phases"][name] = value
        require(value["outcome"] == "success", "Acceptance phase did not succeed")
        return value

    try:
        start = read_json(directory / "start.json")
        setup = read_json(directory / "setup-evidence.json")
        require(
            start["format"] == FORMAT + "-start"
            and start["version"] == VERSION
            and start["plan"] == plan
            and setup["source"] == start["actual_source"]
            and outcomes["setup"] == "success"
            and all(
                outcomes[name] == "success"
                for name in OUTCOME_NAMES
                if name.startswith("setup-")
            ),
            "Setup does not match the accepted source",
        )
        require(
            set(setup["filesystems"]) == {"/btrfs", "/tmpfs", "/minix"}
            and all(
                setup["filesystems"][path]["type"] == expected
                for path, expected in (
                    ("/btrfs", "btrfs"),
                    ("/tmpfs", "tmpfs"),
                    ("/minix", "minix"),
                )
            )
            and setup["uv"]["version"].startswith("uv 0.12.10 ("),
            "Setup does not match the Linux test workload",
        )
        require(
            bool(setup["python_requests"]["values"])
            and set(setup["python_requests"]["values"])
            <= {item["version"] for item in setup["installed_pythons"]}
            and all(
                cache.SHA256.fullmatch(item["executable"]["sha256"]) is not None
                for item in setup["installed_pythons"]
            ),
            "Python fixture evidence is incomplete",
        )
        successful_phase("setup")
        checks["setup"] = True
        summary["python_fixtures"] = sorted(
            [item["version"], item["key"], item["executable"]["sha256"]]
            for item in setup["installed_pythons"]
        )
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        failed("setup", error)
    try:
        state = recorded_manifest(plan, read_json(directory / "restore-manifest.json"))
        start = read_json(directory / "start.json")
        final = read_json(directory / "source-final.json")
        identity = read_json(directory / "identity-final.json")
        expected_source = state["identity"]["identity"]["source"]
        require(
            start["actual_source"] == expected_source
            and final["status"] == "success"
            and final["source"] == expected_source
            and identity["unchanged"] is True
            and identity["source"] == expected_source
            and identity["restore_manifest_sha256"]
            == file_reference(directory / "restore-manifest.json")["sha256"],
            "The source or cache identity changed",
        )
        checks["source_unchanged"] = True
        summary["keys"] = state["identity"]["keys"]
        summary["declared_compatibility"] = {
            "source_inputs": expected_source["inputs"],
            "platform": state["identity"]["identity"]["platform"],
            "environment": state["identity"]["identity"]["configuration"][
                "environment"
            ],
            "cargo_configuration": [
                {key: item[key] for key in ("role", "sha256")}
                for item in state["identity"]["identity"]["configuration"]["cargo"]
            ],
            "tools": cache.semantic_tools(state["identity"]["identity"]["tools"]),
        }
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        failed("source_unchanged", error)
    if state is not None:
        try:
            observation = read_json(directory / "restore-observation.json")
            expected = expectations(plan, state)
            if plan["expected_target"] == "malformed":
                expected["malformed_metadata_restored"] = (
                    bounded_bytes(
                        directory / "restored-rustc-info.json", cache.MAX_CONFIG_BYTES
                    )
                    == MALFORMED_RUSTC_INFO
                )
            require(
                observation["service_observation_origin"] == "official-cache-actions"
                and observation["source_manifest_sha256"]
                == file_reference(directory / "restore-manifest.json")["sha256"]
                and observation["restore"] == state["restore"]
                and observation["expectations"] == expected
                and all(expected.values())
                and outcomes["observation"] == "success",
                "The official restore did not meet the case's cache contract",
            )
            if plan["mode"] == "seed":
                require(
                    all(
                        outcomes[name] == "success"
                        for name in ("seed-prepare", "seed-downloads", "seed-observe")
                    )
                    and outcomes["restore"] == "skipped",
                    "The downloads-only restore path was not used",
                )
            else:
                require(
                    outcomes["restore"] == "success"
                    and all(
                        outcomes[name] == "skipped"
                        for name in ("seed-prepare", "seed-downloads", "seed-observe")
                    ),
                    "The normal restore wrapper did not succeed",
                )
            successful_phase("restore")
            checks["restore"] = True
            summary["restore"] = state["restore"]
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            failed("restore", error)
        try:
            if plan["mode"] == "fault-fixture":
                require(
                    outcomes["workload"] == "skipped"
                    and outcomes["fault-prepare"] == "success"
                    and not (directory / "command-nextest.json").exists()
                    and not (directory / "command-fetch.json").exists(),
                    "The fault producer ran an undeclared workload",
                )
                summary["fault_fixture"] = checked_fault_fixture(plan, directory, state)
                successful_phase("fault")
            else:
                require(
                    outcomes["workload"] == "success"
                    and outcomes["fault-prepare"] == "skipped"
                    and read_json(directory / "post-workload-identity.json")[
                        "unchanged"
                    ]
                    is True,
                    "The workload or its identity check failed",
                )
                name = "fetch" if plan["mode"] == "seed" else "nextest"
                command = recorded_command(plan, directory, state, name)
                require(
                    command_succeeded(command), "The workload process did not complete"
                )
                timing = successful_phase("workload")
                require(
                    timing["started_monotonic_ns"]
                    <= command["command_started_monotonic_ns"]
                    <= command["command_finished_monotonic_ns"]
                    <= timing["finished_monotonic_ns"],
                    "Command is outside its recorded workload phase",
                )
                summary["workload"] = (
                    {"command_elapsed_ns": command["command_elapsed_ns"]}
                    if name == "fetch"
                    else checked_workload(plan, directory, state)
                )
            checks["workload_or_fixture"] = True
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            ET.ParseError,
        ) as error:
            failed("workload_or_fixture", error)
        writable = (
            plan["mode"] == "full"
            and plan["save_allowed"]
            and not state["restore"]["target"]["exact"]
        )
        try:
            if writable:
                require(
                    outcomes["prune"] == "success"
                    and checks["workload_or_fixture"]
                    and command_succeeded(
                        recorded_command(plan, directory, state, "prune")
                    ),
                    "The source-aware pruner did not complete",
                )
                successful_phase("prune")
                summary["fingerprints_pruned"] = checked_fingerprints(
                    directory, "pruned"
                )
                summary["payload_pruned"] = checked_payload_inventory(
                    read_json(directory / "payload-pruned.json"), hash_contents=False
                )
            else:
                require(
                    outcomes["prune"] == "skipped"
                    and not (directory / "command-prune.json").exists(),
                    "A read-only case ran the pruner",
                )
            checks["prune"] = True
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            failed("prune", error)
        try:
            if plan["mode"] == "seed":
                publication = read_json(directory / "seed-publication-plan.json")
                require(
                    checks["restore"]
                    and checks["workload_or_fixture"]
                    and outcomes["seed-save-plan"] == outcomes["seed-save"] == "success"
                    and outcomes["save"] == outcomes["fault-save"] == "skipped"
                    and publication
                    == {
                        "kind": "isolated-downloads-fixture",
                        "source_manifest_sha256": file_reference(
                            directory / "restore-manifest.json"
                        )["sha256"],
                        "downloads_key": state["identity"]["keys"]["downloads"],
                        "downloads_paths": list(cache.DOWNLOAD_PATHS),
                        "source": plan["source"],
                        "normal_nextest_workload_ran": False,
                    },
                    "The downloads fixture publication differs from its plan",
                )
                successful_phase("save")
            elif plan["mode"] == "fault-fixture":
                require(
                    checks["restore"]
                    and checks["workload_or_fixture"]
                    and outcomes["fault-save"] == "success"
                    and outcomes["save"]
                    == outcomes["seed-save-plan"]
                    == outcomes["seed-save"]
                    == "skipped",
                    "The isolated malformed fixture was not published as planned",
                )
                successful_phase("save")
            elif writable:
                require(
                    checks["restore"]
                    and checks["workload_or_fixture"]
                    and checks["prune"]
                    and outcomes["save"] == "success"
                    and outcomes["save-downloads"] == "skipped"
                    and outcomes["save-target"] == "success"
                    and outcomes["seed-save-plan"]
                    == outcomes["seed-save"]
                    == outcomes["fault-save"]
                    == "skipped",
                    "The normal save wrapper did not follow its original permission",
                )
                successful_phase("save")
            else:
                require(
                    outcomes["save"]
                    == outcomes["seed-save-plan"]
                    == outcomes["seed-save"]
                    == outcomes["fault-save"]
                    == "skipped"
                    and outcomes["save-downloads"] in {"skipped", "unknown"}
                    and outcomes["save-target"] in {"skipped", "unknown"}
                    and not (directory / "phase-save-start.json").exists()
                    and not (directory / "phase-save-end.json").exists(),
                    "A read-only case attempted cache publication",
                )
            checks["save_policy"] = True
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            failed("save_policy", error)
        try:
            summary["phase_sequence"] = checked_phase_order(plan, directory, state)
            checks["phase_order"] = True
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            failed("phase_order", error)
    return {
        "checks": checks,
        "errors": errors,
        "summary": summary,
        "complete": all(checks.values()),
    }


def finish_case(
    plan: dict[str, Any],
    manifest_path: Path | None,
    manifest_sha256: str,
    environment: dict[str, str],
    outcomes: dict[str, str],
    job_status: str,
    github_output: Path,
) -> int:
    require(
        job_status in {"success", "failure", "cancelled"},
        "Invalid enclosing job status",
    )
    directory = evidence_directory(plan)
    write_json(
        directory / "workflow-outcomes.json",
        {"job_status": job_status, "steps": outcomes},
    )
    try:
        write_json(
            directory / "source-final.json",
            {"status": "success", "source": actual_source(plan, environment)},
        )
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        write_json(
            directory / "source-final.json",
            {"status": "failure", "error": error_details(error)},
        )
    try:
        if manifest_path is None or not manifest_sha256:
            raise AcceptanceError("The restore did not produce an identity manifest")
        state = manifest_state(plan, manifest_path, manifest_sha256, environment)
        write_json(
            directory / "identity-final.json",
            {
                "unchanged": True,
                "source": state["identity"]["identity"]["source"],
                "restore_manifest_sha256": manifest_sha256,
            },
        )
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        write_json(
            directory / "identity-final.json",
            {"unchanged": False, "error": error_details(error)},
        )
    evaluation = evaluate_case(plan, directory, outcomes, job_status)
    try:
        inventory = evidence_inventory(directory)
        safe = True
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        inventory = []
        safe = False
        evaluation["checks"]["evidence_inventory"] = False
        evaluation["errors"]["evidence_inventory"] = error_details(error)
        evaluation["complete"] = False
    result: dict[str, Any] = {
        "format": FORMAT + "-case",
        "version": VERSION,
        "case": plan["case"],
        "plan_sha256": file_reference(directory / "plan.json")["sha256"],
        "job_status": job_status,
        "outcomes": outcomes,
        **evaluation,
        "evidence": inventory,
        "finished_at": timestamp(),
    }
    checksum = write_json(directory / "result.json", result)
    cache.write_outputs(
        github_output,
        {
            "artifact-safe": str(safe).lower(),
            "case-complete": str(result["complete"]).lower(),
            "result-sha256": checksum,
        },
    )
    print(
        json.dumps(
            {
                "case": plan["case"],
                "complete": result["complete"],
                "failed_checks": [
                    name for name, value in result["checks"].items() if not value
                ],
                "result_sha256": checksum,
            },
            sort_keys=True,
        ),
        flush=True,
    )
    return 0 if result["complete"] else 1


def recorded_plan(
    plan: dict[str, Any],
    case: str,
    controller: dict[str, Any],
    github: dict[str, str],
    stage: str,
) -> None:
    require(
        plan["format"] == FORMAT + "-plan"
        and type(plan["version"]) is int
        and plan["version"] == VERSION
        and plan["case"] == case
        and plan["stage"] == stage,
        "Artifact belongs to another acceptance case",
    )
    commit, directory, mode, save_allowed, expectation = CASE_DATA[case]
    workspace = Path(plan["workspace"])
    namespace = cache.validate_key_namespace(
        f"acceptance-{github['GITHUB_RUN_ID']}-{github['GITHUB_RUN_ATTEMPT']}"
    )
    require(
        workspace.is_absolute()
        and plan["source"]
        == {
            "repository": REPOSITORY,
            "commit": commit,
            "tree": TREES[commit],
            "relative_directory": directory,
            "path": str(workspace / directory),
        }
        and plan["mode"] == mode
        and type(plan["save_allowed"]) is bool
        and plan["save_allowed"] == save_allowed
        and plan["expected_target"] == expectation
        and plan["namespace"]
        == (namespace + "-fault" if case == "malformed-cache-consumer" else namespace)
        and plan["fault_namespace"] == namespace + "-fault"
        and plan["process_owner"] == PROCESS_OWNER
        and type(plan["job_started_unix"]) is int
        and type(plan["deadline_unix"]) is int
        and plan["deadline_unix"] - plan["job_started_unix"] == JOB_BUDGET_SECONDS,
        "Artifact source, namespace, or workload differs",
    )
    require(
        {key: value for key, value in plan["controller"].items() if key != "path"}
        == {key: value for key, value in controller.items() if key != "path"}
        and plan["controller"]["path"] == str(workspace / "controller"),
        "Artifact was produced by another controller",
    )
    require(
        all(
            plan["github"].get(key) == github[key]
            for key in (
                "GITHUB_REPOSITORY",
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_WORKFLOW_REF",
                "GITHUB_WORKFLOW_SHA",
            )
        )
        and plan["github"]["GITHUB_WORKFLOW_SHA"] == github["GITHUB_SHA"],
        "Artifact belongs to another workflow run",
    )


def service_artifacts(
    values: list[str], selected: tuple[str, ...]
) -> dict[str, dict[str, Any] | None]:
    result: dict[str, dict[str, Any] | None] = {}
    for value in values:
        case, separator, identity = value.partition("=")
        require(
            bool(separator) and case in CASE_DATA and case not in result,
            "Invalid artifact association",
        )
        parts = identity.split(":")
        require(len(parts) == 4, "Invalid service artifact identity")
        number, checksum, result_checksum, download_outcome = parts
        require(download_outcome in OUTCOMES, "Invalid artifact download outcome")
        if not number and not checksum and not result_checksum:
            require(
                download_outcome in {"skipped", "unknown"},
                "A missing artifact has an unexpected download outcome",
            )
            result[case] = None
            continue
        require(
            re.fullmatch(r"[1-9][0-9]{0,19}", number) is not None
            and cache.SHA256.fullmatch(checksum) is not None
            and cache.SHA256.fullmatch(result_checksum) is not None
            and case in selected,
            "Invalid or unexpected service artifact",
        )
        result[case] = {
            "id": int(number),
            "sha256": checksum,
            "result_sha256": result_checksum,
            "download_outcome": download_outcome,
        }
    identifiers = [item["id"] for item in result.values() if item is not None]
    require(
        len(identifiers) == len(set(identifiers)),
        "A service artifact is associated with multiple cases",
    )
    return {case: result.get(case) for case in selected}


def malformed_cache_comparisons(cases: dict[str, Any]) -> dict[str, bool]:
    normal = cases["candidate-exact"]["summary"]
    producer = cases["malformed-cache-fixture"]["summary"]
    consumer = cases["malformed-cache-consumer"]["summary"]
    return {
        "normal_candidate_restored": producer["restore"]["target"]["matched"]
        == normal["keys"]["target"],
        "isolated_fixture_restored": producer["fault_fixture"]["target_key"]
        == consumer["keys"]["target"]
        == consumer["restore"]["target"]["matched"],
        "declared_compatibility_equal": producer["declared_compatibility"]
        == consumer["declared_compatibility"]
        == normal["declared_compatibility"],
        "logical_cargo_unit_inventory_equal": consumer["workload"]["cargo"][
            "logical_unit_multiset"
        ]
        == normal["workload"]["cargo"]["logical_unit_multiset"],
        "test_inventory_equal": consumer["workload"]["junit"]["test_inventory_sha256"]
        == normal["workload"]["junit"]["test_inventory_sha256"],
        "distinct_nextest_invocation": consumer["workload"]["junit"]["run_uuid"]
        not in {
            cases[case]["summary"]["workload"]["junit"]["run_uuid"]
            for case in SOURCE_CASES
            if CASE_DATA[case][2] == "full"
        },
        "python_fixture_identity_equal": producer["python_fixtures"]
        == consumer["python_fixtures"]
        == normal["python_fixtures"],
    }


def aggregate(
    directory: Path,
    output: Path,
    environment: dict[str, str],
    artifact_values: list[str],
) -> int:
    github = github_identity(environment)
    controller = controller_identity(environment)
    stage = environment.get("UV_RUST_CACHE_ACCEPTANCE_STAGE", "")
    require(stage in STAGES, "Unknown acceptance stage")
    selected = SOURCE_CASES + FAULT_CASES if stage == STAGES[1] else SOURCE_CASES
    artifacts = service_artifacts(artifact_values, selected)
    prefix = f"ci-rust-cache-acceptance-{github['GITHUB_RUN_ID']}-{github['GITHUB_RUN_ATTEMPT']}-"
    expected_directories = {prefix + case for case in selected}
    observed_directories = (
        {path.name for path in directory.iterdir()} if directory.is_dir() else set()
    )
    cases: dict[str, Any] = {}
    for case in selected:
        root = directory / (prefix + case)
        artifact = artifacts[case]
        record: dict[str, Any] = {
            "service_artifact": artifact,
            "complete": False,
        }
        try:
            if artifact is None:
                raise AcceptanceError("Expected case artifact is missing")
            require(
                artifact["download_outcome"] == "success",
                "The official artifact download did not succeed",
            )
            require(
                root.is_dir() and not root.is_symlink(),
                "Expected case artifact is missing",
            )
            value = read_json(root / "result.json")
            plan = read_json(root / "plan.json")
            require(
                file_reference(root / "result.json")["sha256"]
                == artifact["result_sha256"],
                "The uploaded case result changed",
            )
            recorded_plan(plan, case, controller, github, stage)
            require(
                value["format"] == FORMAT + "-case"
                and type(value["version"]) is int
                and value["version"] == VERSION
                and value["case"] == case
                and value["plan_sha256"] == file_reference(root / "plan.json")["sha256"]
                and value["evidence"] == evidence_inventory(root),
                "Case evidence inventory differs",
            )
            outcomes = step_outcomes(
                [name + "=" + outcome for name, outcome in value["outcomes"].items()]
            )
            require(
                read_json(root / "workflow-outcomes.json")
                == {"job_status": value["job_status"], "steps": outcomes},
                "Workflow outcome evidence differs",
            )
            evaluation = evaluate_case(plan, root, outcomes, value["job_status"])
            require(
                all(
                    value[key] == evaluation[key]
                    for key in ("checks", "errors", "summary", "complete")
                ),
                "Case completion does not follow from its retained evidence",
            )
            record.update(
                **evaluation,
                result_sha256=file_reference(root / "result.json")["sha256"],
            )
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            ET.ParseError,
        ) as error:
            record["error"] = error_details(error)
        cases[case] = record
    comparisons: dict[str, Any] = {}
    source_complete = all(cases[case]["complete"] for case in SOURCE_CASES)
    if source_complete:
        summaries = [cases[case]["summary"] for case in SOURCE_CASES]
        full = [
            cases[case]["summary"]
            for case in SOURCE_CASES
            if CASE_DATA[case][2] == "full"
        ]
        comparisons = {
            "declared_compatibility_equal": all(
                item["declared_compatibility"] == summaries[0]["declared_compatibility"]
                for item in summaries
            ),
            "downloads_key_equal": len(
                {item["keys"]["downloads"] for item in summaries}
            )
            == 1,
            "target_prefix_equal": len(
                {item["keys"]["target_restore"] for item in summaries}
            )
            == 1,
            "logical_cargo_unit_inventory_equal": all(
                item["workload"]["cargo"]["logical_unit_multiset"]
                == full[0]["workload"]["cargo"]["logical_unit_multiset"]
                for item in full
            ),
            "test_inventory_equal": len(
                {item["workload"]["junit"]["test_inventory_sha256"] for item in full}
            )
            == 1,
            "distinct_nextest_invocations": len(
                {item["workload"]["junit"]["run_uuid"] for item in full}
            )
            == len(full),
            "python_fixture_identity_equal": all(
                item["python_fixtures"] == summaries[0]["python_fixtures"]
                for item in summaries
            ),
        }
    exact_case_set = observed_directories == expected_directories
    fault_comparisons: dict[str, bool] = {}
    fault_complete = stage == STAGES[1] and all(
        cases[case]["complete"] for case in FAULT_CASES
    )
    if source_complete and fault_complete:
        fault_comparisons = malformed_cache_comparisons(cases)
    complete = (
        exact_case_set
        and source_complete
        and all(comparisons.values())
        and all(cases[case]["complete"] for case in selected)
        and all(fault_comparisons.values())
    )
    value = {
        "format": FORMAT + "-aggregate",
        "version": VERSION,
        "github": github,
        "controller": controller,
        "stage": stage,
        "expected_cases": list(selected),
        "observed_artifact_directories": sorted(observed_directories),
        "exact_case_set": exact_case_set,
        "cases": cases,
        "source_comparisons": comparisons,
        "malformed_cache_comparisons": fault_comparisons,
        "source_stage_complete": source_complete and all(comparisons.values()),
        "malformed_cache_stage_complete": (
            source_complete
            and all(comparisons.values())
            and fault_complete
            and all(fault_comparisons.values())
            if stage == STAGES[1]
            else None
        ),
        "selected_stage_complete": complete,
        "production_adoption_qualified": False,
        "speedup_claimed": False,
        "finished_at": timestamp(),
    }
    checksum = write_json(output, value)
    print(
        json.dumps(
            {
                "selected_stage_complete": complete,
                "aggregate_sha256": checksum,
                "failed_cases": [
                    case for case, item in cases.items() if not item["complete"]
                ],
            },
            sort_keys=True,
        ),
        flush=True,
    )
    return 0 if complete else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("plan")
    prepare.add_argument("--case", choices=tuple(CASE_DATA), required=True)
    prepare.add_argument("--output", type=Path, required=True)
    prepare.add_argument("--github-output", type=Path, required=True)
    prepare.add_argument("--github-env", type=Path, required=True)
    aggregate_parser = commands.add_parser("aggregate")
    aggregate_parser.add_argument("--directory", type=Path, required=True)
    aggregate_parser.add_argument("--output", type=Path, required=True)
    aggregate_parser.add_argument("--artifact", action="append", default=[])
    for name in (
        "begin",
        "setup",
        "phase",
        "observe",
        "run",
        "prune",
        "seed-save-plan",
        "malformed-fixture",
        "finish",
    ):
        command = commands.add_parser(name)
        command.add_argument("--plan", type=Path, required=True)
        command.add_argument("--plan-sha256", required=True)
        if name == "begin":
            command.add_argument("--github-env", type=Path, required=True)
        if name == "phase":
            command.add_argument(
                "--name",
                choices=("setup", "restore", "workload", "prune", "save", "fault"),
                required=True,
            )
            command.add_argument("--event", choices=("start", "end"), required=True)
            command.add_argument("--outcome", choices=tuple(sorted(OUTCOMES)))
        if name in {"observe", "run", "prune", "seed-save-plan", "malformed-fixture"}:
            command.add_argument("--manifest", type=Path, required=True)
            command.add_argument("--manifest-sha256", required=True)
        if name in {"run", "prune"}:
            command.add_argument("--process-repository", type=Path, required=True)
        if name in {"observe", "run", "seed-save-plan", "malformed-fixture", "finish"}:
            command.add_argument("--github-output", type=Path, required=True)
        if name == "finish":
            command.add_argument("--manifest", default="")
            command.add_argument("--manifest-sha256", default="")
            command.add_argument(
                "--job-status",
                choices=("success", "failure", "cancelled"),
                required=True,
            )
            command.add_argument("--outcome", action="append", default=[])
    arguments = parser.parse_args()
    environment = dict(os.environ)
    if arguments.command == "plan":
        plan = plan_value(arguments.case, environment)
        output = cache.safe_path(arguments.output).resolve(strict=False)
        require(
            not output.is_relative_to(Path(plan["workspace"])),
            "The acceptance plan must be outside the Actions workspace",
        )
        checksum = write_json(output, plan)
        cache.write_outputs(
            arguments.github_output,
            {
                "plan": str(output),
                "plan-sha256": checksum,
                "source-directory": plan["source"]["path"],
                "source-relative-directory": plan["source"]["relative_directory"],
                "source-commit": plan["source"]["commit"],
                "mode": plan["mode"],
                "namespace": plan["namespace"],
                "fault-namespace": plan["fault_namespace"],
                "save-if": str(plan["save_allowed"]).lower(),
            },
        )
        write_environment(
            arguments.github_env,
            {
                "UV_RUST_CACHE_PLAN": str(output),
                "UV_RUST_CACHE_PLAN_SHA256": checksum,
                "UV_RUST_CACHE_SOURCE": plan["source"]["path"],
                "UV_RUST_CACHE_SOURCE_COMMIT": plan["source"]["commit"],
                "UV_RUST_CACHE_MODE": plan["mode"],
                "UV_RUST_CACHE_NAMESPACE": plan["namespace"],
                "UV_RUST_CACHE_FAULT_NAMESPACE": plan["fault_namespace"],
                "UV_RUST_CACHE_SAVE_ALLOWED": str(plan["save_allowed"]).lower(),
            },
        )
        return 0
    if arguments.command == "aggregate":
        return aggregate(
            arguments.directory, arguments.output, environment, arguments.artifact
        )
    plan = load_plan(arguments.plan, arguments.plan_sha256, environment)
    if arguments.command == "begin":
        begin(plan, environment, arguments.github_env)
    elif arguments.command == "setup":
        setup_evidence(plan, environment)
    elif arguments.command == "phase":
        require(
            (arguments.event == "end") == (arguments.outcome is not None),
            "Only a phase end has an outcome",
        )
        phase(plan, arguments.name, arguments.event, outcome=arguments.outcome)
    elif arguments.command == "observe":
        observe(
            plan,
            arguments.manifest,
            arguments.manifest_sha256,
            environment,
            arguments.github_output,
        )
    elif arguments.command == "run":
        return run_work(
            plan,
            arguments.manifest,
            arguments.manifest_sha256,
            arguments.process_repository,
            environment,
            arguments.github_output,
        )
    elif arguments.command == "prune":
        return run_pruner(
            plan,
            arguments.manifest,
            arguments.manifest_sha256,
            arguments.process_repository,
            environment,
        )
    elif arguments.command == "seed-save-plan":
        seed_save_plan(
            plan,
            arguments.manifest,
            arguments.manifest_sha256,
            environment,
            arguments.github_output,
        )
    elif arguments.command == "malformed-fixture":
        prepare_malformed_fixture(
            plan,
            arguments.manifest,
            arguments.manifest_sha256,
            environment,
            arguments.github_output,
        )
    else:
        return finish_case(
            plan,
            Path(arguments.manifest) if arguments.manifest else None,
            arguments.manifest_sha256,
            environment,
            step_outcomes(arguments.outcome),
            arguments.job_status,
            arguments.github_output,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AcceptanceError, cache.CacheError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from None
    except (OSError, KeyError, TypeError, AttributeError, ValueError, ET.ParseError):
        print(
            "error: Could not inspect the acceptance source or evidence",
            file=sys.stderr,
        )
        raise SystemExit(1) from None
