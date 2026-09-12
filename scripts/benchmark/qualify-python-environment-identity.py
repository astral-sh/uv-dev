"""Qualify immutable environments across a real, same-path CPython upgrade."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import platform
import re
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

VERSIONS = ("3.13.1", "3.13.2")
QUALIFIER_COMMIT = "1146b3b6b99a1649f97fe541c846738a99ce4706"
QUALIFIER_SHA256 = "3370082386ea21b9dec3bd6ee7d6fa2471fde187b767e77d02d000469342f522"
CONSTRAINT_HASHES = {
    "ruff": "9a0555cd3d4f4537ec1e9b7738667491b190991f0b8fc8f7474b6ee63d0b11ac",
    "black": "6aef51651405b6eac28c6d69ff187e70895076eaf661f4b3b95ff7a9d164d6a9",
}
ARCHIVE_HASHES = {
    "3.13.1": "650f1d3242667c64959391105525469e0fe1502a6aab9f5db3b0bfefe7dcbabd",
    "3.13.2": "9002e620e4113e7439b2de0db5ff9b2dc914cde4fba2f10134cce3a5cdebac81",
}
PYTHON_RELATIVE_PATH = Path("bin/python3.13")
BASE_PROBE = """\
import json
import os
import platform
import sys
import sysconfig
print(json.dumps({
    "executable": sys.executable,
    "canonical_executable": os.path.realpath(sys.executable),
    "prefix": sys.prefix,
    "base_prefix": sys.base_prefix,
    "base_executable": getattr(sys, "_base_executable", sys.executable),
    "python_version": list(sys.version_info[:3]),
    "implementation": platform.python_implementation(),
    "soabi": sysconfig.get_config_var("SOABI"),
}))
"""


def verify_sha256(path: Path, expected: str) -> None:
    with path.open("rb") as file:
        actual = hashlib.file_digest(file, "sha256").hexdigest()
    if actual != expected:
        raise ValueError(f"Unexpected input digest for {path}: {actual}")


def load_qualifier(path: Path):
    verify_sha256(path, QUALIFIER_SHA256)
    spec = importlib.util.spec_from_file_location("tool_environment_qualifier", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"Cannot load the existing qualification harness at {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    previous_dont_write_bytecode = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous_dont_write_bytecode
    return module


def file_identity(path: Path, qualifier) -> dict:
    metadata = path.stat()
    return {
        "path": str(path),
        "canonical_path": str(path.resolve(strict=True)),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
        "size": metadata.st_size,
        "mtime_ns": metadata.st_mtime_ns,
        "ctime_ns": metadata.st_ctime_ns,
        "sha256": qualifier.sha256(path),
    }


def replace_python(archive: Path, version: str, installation: Path, qualifier, report):
    if qualifier.sha256(archive) != ARCHIVE_HASHES[version]:
        raise ValueError(f"Unexpected CPython archive digest: {archive}")
    previous = installation.with_name("previous-python")
    if previous.exists() or previous.is_symlink() or installation.is_symlink():
        raise ValueError(f"A previous fixture replacement remains at {previous}")
    with tempfile.TemporaryDirectory(
        prefix="incoming-python-", dir=installation.parent
    ) as temporary:
        with tarfile.open(archive, "r:gz") as source:
            source.extractall(temporary, filter="data")
        incoming = Path(temporary) / "python"
        executable = incoming / PYTHON_RELATIVE_PATH
        if not executable.is_file() or executable.is_symlink():
            raise ValueError(
                "The archive does not contain the expected real interpreter"
            )
        if installation.exists():
            installation.rename(previous)
        try:
            incoming.rename(installation)
        except BaseException:
            if previous.exists() and not installation.exists():
                previous.rename(installation)
            raise
        if previous.exists():
            shutil.rmtree(previous)

    executable = installation / PYTHON_RELATIVE_PATH
    environment = {
        key: value
        for key, value in os.environ.items()
        if key
        not in {
            "PYTHONHOME",
            "PYTHONPATH",
            "PYTHONEXECUTABLE",
            "__PYVENV_LAUNCHER__",
            "PYTHONPLATLIBDIR",
            "VIRTUAL_ENV",
            "CONDA_PREFIX",
        }
    }
    environment.update(PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1")
    result = qualifier.run_command(
        [str(executable), "-I", "-c", BASE_PROBE], environment, installation.parent, 60
    )
    probe = json.loads(result["stdout"])
    if probe["python_version"] != [int(part) for part in version.split(".")]:
        raise AssertionError(
            f"The replacement does not execute CPython {version}: {probe}"
        )
    if probe["implementation"] != "CPython":
        raise AssertionError(f"The replacement is not CPython: {probe}")
    if probe["canonical_executable"] != str(executable.resolve(strict=True)):
        raise AssertionError(f"Unexpected canonical executable: {probe}")
    if Path(probe["base_prefix"]).resolve(strict=True) != installation:
        raise AssertionError(
            f"The interpreter escaped its task-owned installation: {probe}"
        )
    receipt = {
        "version": version,
        "archive": str(archive),
        "archive_sha256": ARCHIVE_HASHES[version],
        "probe": probe,
        "command": result,
        "executable": file_identity(executable, qualifier),
        "dynamic_library": file_identity(
            installation / "lib/libpython3.13.dylib", qualifier
        ),
    }
    report["python_replacements"].append(receipt)
    return receipt


def configuration(path: Path) -> dict[str, str]:
    values = {}
    for line in path.read_text().splitlines():
        key, separator, value = line.partition("=")
        if separator:
            values[key.strip()] = value.strip()
    return values


def check_environment_publication(
    name: str,
    delta: dict,
    work: dict,
    *,
    creates: bool,
    environment_alias: str,
    relative_root: str,
    graph_size: int,
) -> None:
    expected = {
        "created_entries": [environment_alias] if creates else [],
        "retargeted_entries": [],
        "rewritten_entries": [],
        "removed_entries": [],
        "new_archives": [relative_root] if creates else [],
        "removed_archives": [],
    }
    expected_installed = graph_size if creates else 0
    if delta != expected or work["installed"] != expected_installed:
        raise AssertionError(
            f"Unexpected environment publication: {name}: "
            f"{delta}, {work}; expected {expected}, {expected_installed} installed"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--base-revision", required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--candidate-revision", required=True)
    parser.add_argument("--before-archive", type=Path, required=True)
    parser.add_argument("--after-archive", type=Path, required=True)
    parser.add_argument(
        "--qualifier",
        type=Path,
        required=True,
        help=f"qualify-tool-environment-reuse.py from {QUALIFIER_COMMIT}",
    )
    parser.add_argument(
        "--constraints-directory",
        type=Path,
        required=True,
        help="The pinned tool-locks directory next to the reused qualifier",
    )
    parser.add_argument(
        "--package-cache",
        type=Path,
        required=True,
        help="A prepared read-only Ruff and Click package cache",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    for revision in (args.base_revision, args.candidate_revision):
        if re.fullmatch(r"[0-9a-f]{40}", revision) is None:
            parser.error("Both revisions must be exact commit SHAs")
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("These pinned archives are for native macOS arm64")
    for name in (
        "base",
        "candidate",
        "before_archive",
        "after_archive",
        "qualifier",
        "constraints_directory",
        "package_cache",
    ):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    for name, expected in CONSTRAINT_HASHES.items():
        verify_sha256(args.constraints_directory / f"{name}.txt", expected)
    verify_sha256(args.before_archive, ARCHIVE_HASHES[VERSIONS[0]])
    verify_sha256(args.after_archive, ARCHIVE_HASHES[VERSIONS[1]])
    qualifier = load_qualifier(args.qualifier)
    args.output.mkdir(parents=True, exist_ok=False)
    args.output = args.output.resolve(strict=True)
    installation = args.output / "python"
    executable = installation / PYTHON_RELATIVE_PATH
    reused_args = argparse.Namespace(
        python=executable,
        constraints_directory=args.constraints_directory,
        preview_feature=[],
    )
    report = {
        "recorded_at_utc": datetime.datetime.now(datetime.UTC).isoformat(),
        "schema": "uv-python-environment-identity-qualification-v1",
        "driver_sha256": qualifier.sha256(Path(__file__)),
        "reused_qualifier": str(args.qualifier),
        "reused_qualifier_sha256": qualifier.sha256(args.qualifier),
        "reused_qualifier_commit": QUALIFIER_COMMIT,
        "base": qualifier.binary_info(args.base) | {"source": args.base_revision},
        "candidate": qualifier.binary_info(args.candidate)
        | {"source": args.candidate_revision},
        "platform": platform.platform(),
        "canonical_executable": str(executable),
        "package_cache": str(args.package_cache),
        "input_cache": qualifier.cache_inventory(args.package_cache),
        "constraints": {
            name: qualifier.sha256(args.constraints_directory / f"{name}.txt")
            for name in ("ruff", "black")
        },
        "python_replacements": [],
        "graphs": {},
        "cases": [],
        "comparisons": {},
        "measurement_scope": "Physical environment identity, cache publication, installation work, and actual interpreter execution. Elapsed times are diagnostic, not a performance comparison.",
        "success": False,
    }
    tool = qualifier.Tool(
        "ruff", "0.11.13", "ruff", args.constraints_directory / "ruff.txt"
    )
    qualification = qualifier.Qualification(reused_args, report)
    expected_graphs = {}
    unchanged_graph = None
    try:
        for label, binary, policy in (
            ("base", args.base, "path-only"),
            ("candidate", args.candidate, "full-version"),
        ):
            directory = args.output / label
            directory.mkdir()
            cache = directory / "cache"
            qualifier.copy_package_cache(args.package_cache, cache)
            environment = qualification.environment(directory)
            for name in ("PYTHONEXECUTABLE", "__PYVENV_LAUNCHER__", "PYTHONPLATLIBDIR"):
                environment.pop(name, None)
            environment.update(
                UV_NO_SYSTEM_CONFIG="1",
                UV_PYTHON_SEARCH_PATH="",
                UV_PYTHON_INSTALL_DIR=str(directory / "managed"),
            )
            if qualifier.snapshot(cache)["entries"]:
                raise AssertionError(
                    "The copied input cache already has an environment alias"
                )
            roots = {}
            entries = {}
            selected = {}
            for phase, version, archive in (
                ("before", VERSIONS[0], args.before_archive),
                ("after", VERSIONS[1], args.after_archive),
            ):
                selected[phase] = replace_python(
                    archive, version, installation, qualifier, report
                )
                if selected[phase]["executable"]["canonical_path"] != str(executable):
                    raise AssertionError(
                        "The fixture changed its canonical executable path"
                    )
                for changed in (False, True):
                    resolution = "changed" if changed else "unchanged"
                    for repeat in (False, True):
                        name = f"{label}/{phase}/{resolution}/{'warm' if repeat else 'first'}"
                        before = qualifier.snapshot(cache)
                        scenario = "changed-resolution" if changed else "cold"
                        command = qualification.command(binary, cache, tool, scenario)
                        result = qualifier.run_command(
                            command, environment, directory, 180
                        )
                        probe = json.loads(result["stdout"])
                        graph = probe.pop("graph")
                        graph_hash = qualification.check_graph(
                            tool, graph, changed=changed
                        )
                        expected = expected_graphs.setdefault(resolution, graph_hash)
                        if graph_hash != expected:
                            raise AssertionError(
                                f"The frozen dependency graph changed: {name}"
                            )
                        if changed:
                            without_click = [
                                item for item in graph if item["name"] != "click"
                            ]
                            if without_click != unchanged_graph or not any(
                                item["name"] == "click" and item["version"] == "8.2.1"
                                for item in graph
                            ):
                                raise AssertionError(
                                    "The changed-resolution control did not add only Click"
                                )
                        elif unchanged_graph is None:
                            unchanged_graph = graph
                        elif graph != unchanged_graph:
                            raise AssertionError(
                                "The unchanged-resolution control selected another graph"
                            )
                        if probe["python_version"] != [
                            int(part) for part in version.split(".")
                        ]:
                            raise AssertionError(
                                f"The cached environment ran the wrong Python: {probe}"
                            )
                        if (
                            Path(probe["base_executable"]).resolve(strict=True)
                            != executable
                        ):
                            raise AssertionError(
                                f"The cached environment used another base: {probe}"
                            )
                        environment_root = Path(probe["prefix"]).resolve(strict=True)
                        if not environment_root.is_relative_to(cache):
                            raise AssertionError(
                                f"The command escaped its cached environment: {probe}"
                            )
                        version_result = qualifier.run_command(
                            [
                                str(Path(probe["scripts"]) / tool.executable),
                                "--version",
                            ],
                            environment,
                            directory,
                            180,
                        )
                        if tool.version not in version_result["stdout"]:
                            raise AssertionError(
                                "The installed Ruff entry point did not execute"
                            )
                        compatibility = qualifier.run_command(
                            [
                                str(binary),
                                "--no-config",
                                "--no-progress",
                                "--cache-dir",
                                str(cache),
                                "--offline",
                                "pip",
                                "check",
                                "--python",
                                probe["executable"],
                            ],
                            environment,
                            directory,
                            180,
                        )
                        after = qualifier.snapshot(cache)
                        delta = qualifier.changes(before, after)
                        relative_root = environment_root.relative_to(cache).as_posix()
                        aliases = [
                            entry
                            for entry, value in after["entries"].items()
                            if value["target"] == relative_root
                        ]
                        if len(aliases) != 1:
                            raise AssertionError(
                                f"Expected one published environment alias: {aliases}"
                            )
                        work = qualifier.install_work(result["stderr"])
                        creates = not repeat and (
                            phase == "before" or policy == "full-version"
                        )
                        cfg = configuration(environment_root / "pyvenv.cfg")
                        case = {
                            "name": name,
                            "participant": label,
                            "policy": policy,
                            "phase": phase,
                            "resolution": resolution,
                            "repeat": repeat,
                            "python_version": version,
                            "graph_sha256": graph_hash,
                            "environment": probe | {"relative_root": relative_root},
                            "environment_alias": aliases[0],
                            "pyvenv_cfg": cfg,
                            "command": result,
                            "ruff_version_command": version_result,
                            "compatibility_check": compatibility,
                            "before": before,
                            "after": after,
                            "changes": delta,
                            "install_work": work,
                            "expected_new_environment": creates,
                            "success": False,
                        }
                        report["cases"].append(case)
                        check_environment_publication(
                            name,
                            delta,
                            work,
                            creates=creates,
                            environment_alias=aliases[0],
                            relative_root=relative_root,
                            graph_size=len(graph),
                        )
                        expected_cfg_version = (
                            version
                            if creates or policy == "full-version"
                            else VERSIONS[0]
                        )
                        if cfg.get("version_info") != expected_cfg_version:
                            raise AssertionError(
                                f"Unexpected creating-interpreter metadata: {name}: {cfg}"
                            )
                        key = (phase, resolution)
                        if repeat and roots[key] != relative_root:
                            raise AssertionError(
                                f"The repeated request selected another environment: {name}"
                            )
                        roots[key] = relative_root
                        entries[key] = aliases[0]
                        case["success"] = True
            for resolution in ("unchanged", "changed"):
                old_root = roots[("before", resolution)]
                new_root = roots[("after", resolution)]
                if (old_root == new_root) != (policy == "path-only"):
                    raise AssertionError(
                        f"Unexpected environment identity across replacement: {label}/{resolution}"
                    )
                old_entry = entries[("before", resolution)].split("/")
                new_entry = entries[("after", resolution)].split("/")
                if old_entry[2] != new_entry[2]:
                    raise AssertionError(
                        "Interpreter replacement changed the frozen resolution hash"
                    )
                if (old_entry[1] == new_entry[1]) != (policy == "path-only"):
                    raise AssertionError(
                        "Unexpected interpreter shard identity across replacement"
                    )
            if roots[("before", "unchanged")] == roots[("before", "changed")]:
                raise AssertionError("Different resolutions shared one environment")
            report["comparisons"][label] = {
                "same_canonical_executable": selected["before"]["executable"][
                    "canonical_path"
                ]
                == selected["after"]["executable"]["canonical_path"],
                "different_real_executable_bytes": selected["before"]["executable"][
                    "sha256"
                ]
                != selected["after"]["executable"]["sha256"],
                "different_real_library_bytes": selected["before"]["dynamic_library"][
                    "sha256"
                ]
                != selected["after"]["dynamic_library"]["sha256"],
                "unchanged_resolution_reused_environment": roots[
                    ("before", "unchanged")
                ]
                == roots[("after", "unchanged")],
                "changed_resolution_reused_environment": roots[("before", "changed")]
                == roots[("after", "changed")],
                "published_environment_entries": len(
                    qualifier.snapshot(cache)["entries"]
                ),
            }
        if qualifier.cache_inventory(args.package_cache) != report["input_cache"]:
            raise AssertionError("The qualification modified the shared package cache")
        report["success"] = True
    except BaseException as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
        print(
            json.dumps(
                {
                    "success": report["success"],
                    "cases": len(report["cases"]),
                    "comparisons": report["comparisons"],
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
