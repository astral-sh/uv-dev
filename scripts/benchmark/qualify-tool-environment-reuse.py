"""Qualify reuse and publication of immutable, real-world tool environments."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import stat
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

PYTHON_VERSION = [3, 12, 11]
EXCLUDE_NEWER = "2025-06-12T00:00:00Z"
CHANGED_REQUIREMENT = "click==8.2.1"
SCENARIOS = ("cold", "warm", "latest", "refresh", "refresh-package")
POLICIES = ("reuse", "refresh-rebuild")
PROBE = """\
import hashlib
import importlib.metadata
import json
import platform
import re
import sys
import sysconfig

graph = []
for distribution in importlib.metadata.distributions():
    name = distribution.metadata["Name"]
    metadata = distribution.read_text("METADATA")
    if metadata is None:
        metadata = distribution.read_text("PKG-INFO")
    if name is None or metadata is None:
        raise RuntimeError("Installed distribution has incomplete metadata")
    graph.append({
        "name": re.sub(r"[-_.]+", "-", name).lower(),
        "version": distribution.version,
        "metadata_sha256": hashlib.sha256(metadata.encode()).hexdigest(),
    })
graph.sort(key=lambda item: (item["name"], item["version"]))
print(json.dumps({
    "prefix": sys.prefix,
    "base_prefix": sys.base_prefix,
    "executable": sys.executable,
    "base_executable": getattr(sys, "_base_executable", sys.executable),
    "scripts": sysconfig.get_path("scripts"),
    "python_version": list(sys.version_info[:3]),
    "implementation": platform.python_implementation(),
    "tool_version": importlib.metadata.version(sys.argv[1]),
    "graph": graph,
}))
"""


def sha256(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def canonical_name(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def graph_digest(graph: list[dict]) -> str:
    return hashlib.sha256(
        json.dumps(graph, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def file_identity(path: Path, *, follow_symlinks: bool = True) -> dict[str, int]:
    metadata = path.stat(follow_symlinks=follow_symlinks)
    return {"device": metadata.st_dev, "inode": metadata.st_ino}


def cache_inventory(directory: Path) -> dict:
    """Hash an input cache without following its relocatable archive links."""
    digest = hashlib.sha256()
    files = links = directories = size = 0
    for current, names, filenames in os.walk(directory, followlinks=False):
        names.sort()
        filenames.sort()
        for name in sorted([*names, *filenames]):
            path = Path(current) / name
            relative = path.relative_to(directory).as_posix()
            metadata = path.lstat()
            if path.is_symlink():
                target = os.readlink(path)
                if Path(target).is_absolute():
                    raise ValueError(f"Non-relocatable cache link: {path}")
                if not (path.parent / target).resolve().is_relative_to(directory):
                    raise ValueError(f"Cache link escapes its input: {path}")
                record = ["link", relative, target]
                links += 1
            elif stat.S_ISDIR(metadata.st_mode):
                record = ["directory", relative, stat.S_IMODE(metadata.st_mode)]
                directories += 1
            elif stat.S_ISREG(metadata.st_mode):
                record = [
                    "file",
                    relative,
                    stat.S_IMODE(metadata.st_mode),
                    sha256(path),
                ]
                files += 1
                size += metadata.st_size
            else:
                raise ValueError(f"Unsupported cache input: {path}")
            digest.update(json.dumps(record, separators=(",", ":")).encode() + b"\n")
    return {
        "sha256": digest.hexdigest(),
        "files": files,
        "links": links,
        "directories": directories,
        "file_bytes": size,
    }


def copy_package_cache(source: Path, destination: Path) -> None:
    # A cold environment lookup still uses the real, already-prepared package cache.
    # Omitting only the environment aliases leaves wheel and index cache state intact.
    def ignore(directory: str, names: list[str]) -> set[str]:
        if Path(directory) == source:
            return {"environments-v2"}.intersection(names)
        return set()

    shutil.copytree(source, destination, symlinks=True, ignore=ignore)


def is_environment(path: Path) -> bool:
    return (path / "pyvenv.cfg").is_file() and any(
        (path / executable).is_file()
        for executable in ("bin/python", "Scripts/python.exe")
    )


def environment_archives(cache: Path) -> dict:
    archives = {}
    for bucket in sorted(cache.glob("archive-v*")):
        if not bucket.is_dir():
            continue
        for path in sorted(bucket.iterdir()):
            if path.is_dir() and is_environment(path):
                archives[path.relative_to(cache).as_posix()] = {
                    "identity": file_identity(path),
                    "pyvenv_cfg_sha256": sha256(path / "pyvenv.cfg"),
                }
    return archives


def environment_entries(cache: Path) -> dict:
    entries = {}
    bucket = cache / "environments-v2"
    if not bucket.is_dir():
        return entries
    for shard in sorted(bucket.iterdir()):
        if not shard.is_dir() or shard.is_symlink():
            continue
        for entry in sorted(shard.iterdir()):
            if entry.is_symlink():
                target = entry.resolve(strict=True)
            elif entry.is_file() and entry.stat().st_size < 256:
                link = entry.read_text()
                # Windows cache links contain the archive bucket and identifier.
                if re.fullmatch(r"archive-v[0-9]+/[^/\\]+", link) is None:
                    continue
                target = (cache / link).resolve(strict=True)
            else:
                continue
            if not target.is_relative_to(cache) or not is_environment(target):
                continue
            entries[entry.relative_to(cache).as_posix()] = {
                "identity": file_identity(entry, follow_symlinks=False),
                "target": target.relative_to(cache).as_posix(),
                "target_identity": file_identity(target),
                "pyvenv_cfg_sha256": sha256(target / "pyvenv.cfg"),
            }
    return entries


def snapshot(cache: Path) -> dict:
    return {
        "entries": environment_entries(cache),
        "archives": environment_archives(cache),
    }


def changes(before: dict, after: dict) -> dict:
    old_entries = before["entries"]
    new_entries = after["entries"]
    shared = old_entries.keys() & new_entries.keys()
    return {
        "created_entries": sorted(new_entries.keys() - old_entries.keys()),
        "retargeted_entries": sorted(
            key
            for key in shared
            if old_entries[key]["target"] != new_entries[key]["target"]
        ),
        "rewritten_entries": sorted(
            key
            for key in shared
            if old_entries[key]["target"] == new_entries[key]["target"]
            and old_entries[key] != new_entries[key]
        ),
        "removed_entries": sorted(old_entries.keys() - new_entries.keys()),
        "new_archives": sorted(after["archives"].keys() - before["archives"].keys()),
        "removed_archives": sorted(
            before["archives"].keys() - after["archives"].keys()
        ),
    }


@dataclass(frozen=True)
class Tool:
    name: str
    version: str
    executable: str
    constraints: Path

    def requirement(self) -> str:
        return f"{self.name}=={self.version}"


def constraint_versions(paths: list[Path]) -> dict[str, set[str]]:
    versions: dict[str, set[str]] = {}
    for path in paths:
        for line in path.read_text().splitlines():
            match = re.match(r"^([A-Za-z0-9_.-]+)(?:\[[^]]+\])?==([^ ;\\]+)", line)
            if match is not None:
                name, version = match.groups()
                versions.setdefault(canonical_name(name), set()).add(version)
    return versions


def install_work(stderr: str) -> dict[str, int]:
    counts = {
        name.lower(): 0 for name in ("Resolved", "Prepared", "Installed", "Audited")
    }
    for action, count in re.findall(
        r"(?m)^(Resolved|Prepared|Installed|Audited) ([0-9]+) packages? ", stderr
    ):
        counts[action.lower()] += int(count)
    return counts


def run_command(
    command: list[str], environment: dict, directory: Path, timeout: float
) -> dict:
    started = time.perf_counter()
    result = subprocess.run(
        command,
        cwd=directory,
        env=environment,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    output = {
        "command": command,
        "exit_code": result.returncode,
        "elapsed_seconds": time.perf_counter() - started,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }
    if result.returncode != 0:
        raise RuntimeError(f"Command failed: {output}")
    return output


class Qualification:
    def __init__(self, args: argparse.Namespace, report: dict) -> None:
        self.args = args
        self.report = report
        self.expected_graphs: dict[str, str] = {}

    def environment(self, directory: Path) -> dict[str, str]:
        environment = {
            name: value
            for name, value in os.environ.items()
            if not name.startswith("UV_")
            and name
            not in {
                "VIRTUAL_ENV",
                "CONDA_PREFIX",
                "PYTHONHOME",
                "PYTHONPATH",
                "RUST_LOG",
            }
        }
        environment.update(
            UV_TOOL_DIR=str(directory / "tools"),
            UV_TOOL_BIN_DIR=str(directory / "bin"),
            UV_PYTHON_DOWNLOADS="never",
            PYTHONNOUSERSITE="1",
            PYTHONDONTWRITEBYTECODE="1",
        )
        return environment

    def command(
        self, binary: Path, cache: Path, tool: Tool, scenario: str
    ) -> list[str]:
        command = [
            str(binary),
            "--no-config",
            "--no-progress",
            "--color",
            "never",
            "--cache-dir",
            str(cache),
        ]
        # Explicit refresh flags conflict with offline mode. Their package graphs are still
        # constrained to the prepared versions, but registry revalidation is part of the command.
        if scenario not in {"refresh", "refresh-package"}:
            command.append("--offline")
        for feature in self.args.preview_feature:
            command.extend(["--preview-features", feature])
        command.extend(
            [
                "tool",
                "run",
                "--isolated",
                "--no-build",
                "--python",
                str(self.args.python),
                "--constraints",
                str(tool.constraints),
            ]
        )
        if scenario == "refresh":
            command.append("--refresh")
        elif scenario == "refresh-package":
            command.extend(["--refresh-package", tool.name])
        if scenario == "changed-resolution":
            command.extend(
                [
                    "--with",
                    CHANGED_REQUIREMENT,
                    "--constraints",
                    str(self.args.constraints_directory / "black.txt"),
                ]
            )
        if scenario == "latest":
            command.extend(
                [
                    "--exclude-newer-package",
                    f"{tool.name}={EXCLUDE_NEWER}",
                    "--from",
                    f"{tool.name}@latest",
                ]
            )
        else:
            command.extend(["--from", tool.requirement()])
        command.extend(["python", "-c", PROBE, tool.name])
        return command

    def check_graph(self, tool: Tool, graph: list[dict], *, changed: bool) -> str:
        constraints = [tool.constraints]
        if changed:
            constraints.append(self.args.constraints_directory / "black.txt")
        allowed = constraint_versions(constraints)
        for distribution in graph:
            if distribution["version"] not in allowed.get(distribution["name"], set()):
                raise AssertionError(
                    f"Distribution is outside the frozen graph: {distribution}"
                )
        digest = graph_digest(graph)
        self.report["graphs"].setdefault(digest, graph)
        return digest

    def matrix(self, binary: Path, label: str, policy: str, tool: Tool) -> None:
        source = self.args.package_cache_root / tool.name
        with tempfile.TemporaryDirectory(
            prefix=f"tool-environment-{tool.name}-{label}-",
            dir=self.args.work_directory,
        ) as temporary:
            directory = Path(temporary)
            cache = directory / "cache"
            copy_package_cache(source, cache)
            environment = self.environment(directory)
            previous_root = None
            baseline_graph = None
            checked_roots = set()
            scenarios = [*SCENARIOS]
            if tool.name == "ruff":
                scenarios.append("changed-resolution")
            for scenario in scenarios:
                before = snapshot(cache)
                if scenario == "cold" and before["entries"]:
                    raise AssertionError(
                        "Cold fixture already contains an environment alias"
                    )
                result = run_command(
                    self.command(binary, cache, tool, scenario),
                    environment,
                    directory,
                    self.args.timeout,
                )
                probe = json.loads(result.pop("stdout"))
                graph = probe.pop("graph")
                if probe["python_version"] != PYTHON_VERSION:
                    raise AssertionError(f"Unexpected Python version: {probe}")
                if probe["tool_version"] != tool.version:
                    raise AssertionError(f"Unexpected tool version: {probe}")
                root = Path(probe["prefix"]).resolve(strict=True)
                if not root.is_relative_to(cache) or not is_environment(root):
                    raise AssertionError(
                        f"Tool did not use its cached environment: {probe}"
                    )
                graph_hash = self.check_graph(
                    tool, graph, changed=scenario == "changed-resolution"
                )
                if baseline_graph is None:
                    baseline_graph = graph
                    expected = self.expected_graphs.setdefault(tool.name, graph_hash)
                    if graph_hash != expected:
                        raise AssertionError(
                            "The compared binaries resolved different tool graphs"
                        )
                elif scenario == "changed-resolution":
                    without_extra = [item for item in graph if item["name"] != "click"]
                    if without_extra != baseline_graph or not any(
                        item["name"] == "click" and item["version"] == "8.2.1"
                        for item in graph
                    ):
                        raise AssertionError(
                            "The changed-resolution control did not add only Click"
                        )
                elif graph != baseline_graph:
                    raise AssertionError(
                        "An unchanged request resolved a different graph"
                    )

                executable = Path(probe["scripts"]) / tool.executable
                if os.name == "nt":
                    executable = executable.with_suffix(".exe")
                version = run_command(
                    [str(executable), "--version"],
                    environment,
                    directory,
                    self.args.timeout,
                )
                if tool.version not in version["stdout"]:
                    raise AssertionError(f"Unexpected executable version: {version}")
                compatibility = None
                if root not in checked_roots:
                    compatibility = run_command(
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
                        self.args.timeout,
                    )
                    checked_roots.add(root)
                after = snapshot(cache)
                delta = changes(before, after)
                relative_root = root.relative_to(cache).as_posix()
                if not any(
                    entry["target"] == relative_root
                    for entry in after["entries"].values()
                ):
                    raise AssertionError(
                        "The executed environment has no published cache alias"
                    )
                work = install_work(result["stderr"])
                creates = scenario in {"cold", "changed-resolution"} or (
                    policy == "refresh-rebuild" and scenario in {"latest", "refresh"}
                )
                publications = len(delta["created_entries"]) + len(
                    delta["retargeted_entries"]
                )
                if creates:
                    if (
                        root == previous_root
                        or publications != 1
                        or delta["new_archives"] != [relative_root]
                        or work["installed"] == 0
                    ):
                        raise AssertionError(
                            f"Expected one new complete environment: {delta}, {work}"
                        )
                elif root != previous_root or any(delta.values()) or work["installed"]:
                    raise AssertionError(
                        f"Cached environment was reconstructed: {delta}, {work}"
                    )
                if delta["removed_entries"] or delta["removed_archives"]:
                    raise AssertionError(
                        f"Environment reuse removed unrelated cache state: {delta}"
                    )
                self.report["cases"].append(
                    {
                        "name": f"{tool.name}/{label}/{scenario}",
                        "tool": tool.name,
                        "participant": label,
                        "scenario": scenario,
                        "network_mode": (
                            "registry-revalidation"
                            if scenario in {"refresh", "refresh-package"}
                            else "offline"
                        ),
                        "policy": policy,
                        "expected_publications": 1 if creates else 0,
                        "environment": probe
                        | {"relative_root": relative_root, "graph_sha256": graph_hash},
                        "command": result,
                        "tool_version_command": version,
                        "compatibility_check": compatibility,
                        "before": before,
                        "after": after,
                        "changes": delta,
                        "install_work": work,
                        "success": True,
                    }
                )
                previous_root = root


def binary_info(path: Path) -> dict:
    return {
        "path": str(path),
        "sha256": sha256(path),
        "version": subprocess.check_output([str(path), "--version"], text=True).strip(),
    }


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument(
        "--manifest", type=Path, default=Path(__file__).with_name("tools.json")
    )
    parser.add_argument(
        "--constraints-directory",
        type=Path,
        default=Path(__file__).with_name("tool-locks"),
    )
    parser.add_argument(
        "--package-cache-root", type=Path, default=root / ".cache/bench-tool-caches"
    )
    parser.add_argument(
        "--work-directory", type=Path, default=root / ".cache/bench-tool-qualification"
    )
    parser.add_argument("--base-policy", choices=POLICIES, default="reuse")
    parser.add_argument("--candidate-policy", choices=POLICIES, default="reuse")
    parser.add_argument("--preview-feature", action="append", default=[])
    parser.add_argument("--tool", action="append", default=[])
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    for name in (
        "base",
        "candidate",
        "python",
        "manifest",
        "constraints_directory",
        "package_cache_root",
    ):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    fixtures = [
        item for item in json.loads(args.manifest.read_text()) if item.get("cached-run")
    ]
    if unknown := set(args.tool).difference(item["name"] for item in fixtures):
        parser.error(f"unknown cached-tool fixtures: {', '.join(sorted(unknown))}")
    tools = [
        Tool(
            item["name"],
            item["version"],
            item["executable"],
            args.constraints_directory / f"{item['name']}.txt",
        )
        for item in fixtures
        if not args.tool or item["name"] in args.tool
    ]
    report = {
        "schema": "uv-tool-environment-qualification-v1",
        "harness_sha256": sha256(Path(__file__)),
        "platform": platform.platform(),
        "base": binary_info(args.base),
        "candidate": binary_info(args.candidate),
        "base_policy": args.base_policy,
        "candidate_policy": args.candidate_policy,
        "python": {
            "path": str(args.python),
            "sha256": sha256(args.python),
            "version": subprocess.check_output(
                [str(args.python), "--version"], text=True
            ).strip(),
        },
        "manifest": {"path": str(args.manifest), "sha256": sha256(args.manifest)},
        "constraints": {tool.name: sha256(tool.constraints) for tool in tools},
        "changed_resolution_requirement": CHANGED_REQUIREMENT,
        "changed_resolution_constraints_sha256": sha256(
            args.constraints_directory / "black.txt"
        ),
        "preview_features": args.preview_feature,
        "package_cache_root": str(args.package_cache_root),
        "input_caches": {},
        "measurement_scope": "Structural environment identity, publication, and installation work. CLI elapsed times are diagnostic, not an optimized performance comparison.",
        "graphs": {},
        "cases": [],
    }
    try:
        qualification = Qualification(args, report)
        for tool in tools:
            source = args.package_cache_root / tool.name
            if not source.is_dir():
                raise ValueError(
                    f"Missing tool package cache: {source}; run prepare-tools.py"
                )
            before = cache_inventory(source)
            report["input_caches"][tool.name] = before
            for label, binary, policy in [
                ("base", args.base, args.base_policy),
                ("candidate", args.candidate, args.candidate_policy),
            ]:
                qualification.matrix(binary, label, policy, tool)
            if cache_inventory(source) != before:
                raise AssertionError(
                    f"Qualification modified its input cache: {source}"
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
