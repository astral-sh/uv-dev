# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Identify the Linux nextest cache without depending on another action's logs."""

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
import sys
import tomllib
import uuid
from collections.abc import Mapping
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

FORMAT = "uv-rust-cache"
VERSION = 1
CACHE_ACTION = "actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
HOST = "x86_64-unknown-linux-gnu"
PROFILE = "fast-build-nightly"
NEXTEST_VERSION = "0.9.143"
MAX_JSON_BYTES = 2 * 1024 * 1024
MAX_CONFIG_BYTES = 1024 * 1024
OID = re.compile(r"[0-9a-f]{40}")
SHA256 = re.compile(r"[0-9a-f]{64}")
KEY_NAMESPACE = re.compile(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*")
REPOSITORY = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+")
DOWNLOAD_PATHS = (
    "~/.cargo/registry/index",
    "~/.cargo/registry/cache",
    "~/.cargo/registry/src",
    "~/.cargo/git/db",
    "~/.cargo/git/checkouts",
)
TARGET_PATHS = (
    "target/.rustc_info.json",
    f"target/{PROFILE}",
    f"!target/{PROFILE}/incremental",
    f"!target/{PROFILE}/incremental/**",
    f"!target/{PROFILE}/.cargo-lock",
)
WORKLOAD = {
    "id": "linux-nextest-fast-build-nightly-v1",
    "host": HOST,
    "profile": PROFILE,
    "nextest_version": NEXTEST_VERSION,
    "cargo_arguments": [
        "nextest",
        "run",
        "--cargo-profile",
        PROFILE,
        "-Z",
        "panic-abort-tests",
        "-Z",
        "checksum-freshness",
        "--features",
        "test-python-patch,test-universal,native-auth,secret-service",
        "--workspace",
        "--profile",
        "ci-linux",
    ],
    "download_paths": list(DOWNLOAD_PATHS),
    "target_paths": list(TARGET_PATHS),
}
REQUIRED_ENVIRONMENT = {
    "CARGO_INCREMENTAL": "0",
    "RUSTC_BOOTSTRAP": "1",
    "UV_LOCKED": "1",
}
TOOL_SELECTORS = {
    "AR",
    "CC",
    "CXX",
    "LD",
    "RANLIB",
    "RUSTC",
    "RUSTDOC",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTDOC",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
}
COMPILATION_VARIABLES = {
    *REQUIRED_ENVIRONMENT,
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CPPFLAGS",
    "CFLAGS",
    "CXXFLAGS",
    "LDFLAGS",
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
}
COMPILATION_PREFIXES = (
    "AWS_LC_",
    "CARGO_PROFILE_",
    "CARGO_TARGET_",
    "CMAKE_",
    "OPENSSL_",
    "PKG_CONFIG_",
)
CREDENTIAL_NAME = re.compile(r"(?:TOKEN|PASSWORD|SECRET|CREDENTIAL|AUTH)")


class CacheError(ValueError):
    """A cache identity or state does not satisfy the supported contract."""


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()


def json_bytes(value: Any) -> bytes:
    return (
        json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n"
    ).encode()


def digest(value: Any) -> str:
    return hashlib.sha256(canonical_json(value)).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CacheError(message)


def safe_path(value: str | Path) -> Path:
    value = str(value)
    require(
        not any(ord(character) < 32 or ord(character) == 127 for character in value),
        "Invalid path",
    )
    return Path(value).absolute()


def file_identity(path: Path, *, limit: int | None = None) -> dict[str, Any]:
    invocation = safe_path(path)
    resolved = invocation.resolve(strict=True)
    with resolved.open("rb") as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode), "Expected a regular file")
        require(limit is None or before.st_size <= limit, "File exceeds its limit")
        if limit is None:
            checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        else:
            contents = stream.read(limit + 1)
            require(len(contents) <= limit, "File exceeds its limit")
            checksum = hashlib.sha256(contents).hexdigest()
        after = os.fstat(stream.fileno())

    def state(value: os.stat_result) -> tuple[int, int, int, int, int]:
        return (
            value.st_dev,
            value.st_ino,
            value.st_size,
            value.st_mtime_ns,
            value.st_ctime_ns,
        )

    require(
        state(before) == state(after)
        and invocation.resolve(strict=True) == resolved
        and state(resolved.stat()) == state(after),
        "File changed while its identity was recorded",
    )
    return {
        "invocation": str(invocation),
        "resolved": str(resolved),
        "sha256": checksum,
        "size": before.st_size,
    }


def read_configuration(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    identity = file_identity(path, limit=MAX_CONFIG_BYTES)
    with path.open("rb") as stream:
        contents = stream.read(MAX_CONFIG_BYTES + 1)
    require(
        len(contents) <= MAX_CONFIG_BYTES
        and hashlib.sha256(contents).hexdigest() == identity["sha256"],
        "Cargo configuration changed while it was read",
    )
    value = tomllib.loads(contents.decode("utf-8"))
    require(
        file_identity(path, limit=MAX_CONFIG_BYTES) == identity,
        "Cargo configuration changed while it was read",
    )
    return identity, value


def executable_identity(name: str, environment: Mapping[str, str]) -> dict[str, Any]:
    invocation = shutil.which(name, path=environment.get("PATH"))
    if invocation is None:
        raise CacheError(f"Required executable is unavailable: {name}")
    return file_identity(Path(invocation))


def verify_executable(
    name: str, expected: dict[str, Any], environment: Mapping[str, str]
) -> None:
    require(
        executable_identity(name, environment) == expected,
        f"Executable changed while its identity was recorded: {name}",
    )


def command(
    arguments: list[str],
    source: Path,
    environment: Mapping[str, str],
    description: str,
) -> str:
    try:
        result = subprocess.run(
            arguments,
            cwd=source,
            env=environment,
            capture_output=True,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CacheError(f"Could not {description}") from error
    require(result.returncode == 0, f"Could not {description}")
    require(len(result.stdout) <= MAX_JSON_BYTES, "Command output exceeds its limit")
    try:
        return result.stdout.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise CacheError(f"Invalid output while attempting to {description}") from error


def git_command(
    source: Path, git: dict[str, Any], environment: Mapping[str, str], *arguments: str
) -> str:
    return command(
        [git["invocation"], "-c", "core.fsmonitor=false", *arguments],
        source,
        {**environment, "GIT_OPTIONAL_LOCKS": "0"},
        "inspect Git source",
    )


def repository_name(remote: str) -> str:
    if remote.startswith("git@github.com:"):
        name = remote.removeprefix("git@github.com:")
    else:
        parsed = urlsplit(remote)
        require(
            parsed.scheme == "https"
            and parsed.netloc == "github.com"
            and not parsed.query
            and not parsed.fragment,
            "Expected a credential-free GitHub origin",
        )
        name = parsed.path.removeprefix("/")
    name = name.removesuffix(".git")
    require(REPOSITORY.fullmatch(name) is not None, "Invalid GitHub repository")
    return name.lower()


def input_kind(name: str) -> str | None:
    path = Path(name)
    if path.name in {"Cargo.toml", "Cargo.lock"}:
        return "dependency"
    if path.name in {"rust-toolchain", "rust-toolchain.toml"} or (
        path.parent.name == ".cargo" and path.name in {"config", "config.toml"}
    ):
        return "configuration"
    return None


def require_clean_source(
    source: Path, git: dict[str, Any], environment: Mapping[str, str]
) -> None:
    require(
        not git_command(
            source,
            git,
            environment,
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ),
        "The source checkout is dirty",
    )
    # A normal status check can hide local changes behind assume-unchanged or sparse entries.
    entries = git_command(source, git, environment, "ls-files", "-v", "-z")
    require(
        all(entry.startswith("H ") for entry in entries.split("\0") if entry),
        "The source checkout uses unsupported index flags",
    )


def source_identity(
    source: Path,
    repository: str,
    commit: str,
    git: dict[str, Any],
    environment: Mapping[str, str],
) -> dict[str, Any]:
    require(OID.fullmatch(commit) is not None, "Expected a full source commit")
    require(REPOSITORY.fullmatch(repository) is not None, "Invalid source repository")
    require(
        Path(git_command(source, git, environment, "rev-parse", "--show-toplevel"))
        == source,
        "Expected the source directory to be the Git checkout root",
    )
    require(
        git_command(source, git, environment, "rev-parse", "HEAD") == commit,
        "The source checkout does not match the requested commit",
    )
    require(
        repository_name(
            git_command(source, git, environment, "remote", "get-url", "origin")
        )
        == repository.lower(),
        "The source checkout does not match the requested repository",
    )
    require_clean_source(source, git, environment)
    tree = git_command(source, git, environment, "rev-parse", "HEAD^{tree}")
    require(OID.fullmatch(tree) is not None, "Invalid source tree")
    files = []
    entries = git_command(
        source, git, environment, "ls-tree", "-rz", "--full-tree", "HEAD"
    )
    for entry in entries.split("\0"):
        if not entry:
            continue
        metadata, separator, name = entry.partition("\t")
        require(bool(separator), "Invalid Git tree entry")
        require(not metadata.startswith("160000 "), "Source submodules are unsupported")
        if kind := input_kind(name):
            mode, object_kind, blob = metadata.split(" ")
            require(
                mode in {"100644", "100755"}
                and object_kind == "blob"
                and OID.fullmatch(blob) is not None,
                "Unsupported source configuration entry",
            )
            path = source / name
            require(
                path.resolve(strict=True).is_relative_to(source)
                and not path.is_symlink(),
                "Source configuration escapes its checkout",
            )
            identity = file_identity(path, limit=MAX_CONFIG_BYTES)
            files.append(
                {
                    "path": name,
                    "kind": kind,
                    "git_blob": blob,
                    "sha256": identity["sha256"],
                    "size": identity["size"],
                }
            )
    require(
        {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
        <= {item["path"] for item in files},
        "Missing required Cargo source inputs",
    )
    require(
        git_command(source, git, environment, "rev-parse", "HEAD") == commit,
        "The source changed while its identity was recorded",
    )
    require_clean_source(source, git, environment)
    return {
        "repository": repository.lower(),
        "commit": commit,
        "tree": tree,
        "path": str(source),
        "inputs": sorted(files, key=lambda item: item["path"]),
    }


def version_fields(output: str, name: str) -> dict[str, str]:
    lines = output.splitlines()
    require(bool(lines) and lines[0].startswith(name + " "), f"Invalid {name} version")
    fields = {}
    for line in lines[1:]:
        key, separator, value = line.partition(": ")
        if separator and key in {
            "release",
            "host",
            "commit-hash",
            "commit-date",
            "LLVM version",
        }:
            require(key not in fields, f"Duplicate {name} version field")
            fields[key] = value
    require(
        {"release", "host", "commit-hash"} <= fields.keys()
        and OID.fullmatch(fields["commit-hash"]) is not None,
        f"Incomplete {name} version",
    )
    return fields


def rust_toolchain(source: Path, environment: Mapping[str, str]) -> dict[str, Any]:
    environment = {**environment, "RUSTUP_AUTO_INSTALL": "0"}
    rustup = executable_identity("rustup", environment)
    proxies = {
        name: executable_identity(name, environment) for name in ("cargo", "rustc")
    }
    require(
        all(proxy["sha256"] == rustup["sha256"] for proxy in proxies.values()),
        "The Linux cache requires Rustup-managed Cargo and rustc",
    )
    active = command(
        [rustup["invocation"], "show", "active-toolchain"],
        source,
        environment,
        "identify the Rust toolchain",
    ).split(" ", 1)[0]
    _, configuration = read_configuration(source / "rust-toolchain.toml")
    channel = configuration["toolchain"]["channel"]
    require(
        isinstance(channel, str)
        and re.fullmatch(r"\d+\.\d+\.\d+", channel) is not None
        and active == f"{channel}-{HOST}",
        "The active Rust toolchain does not match the source's pinned Linux toolchain",
    )
    result: dict[str, Any] = {"rustup": rustup, "active": active}
    for name, arguments in (("cargo", ["-Vv"]), ("rustc", ["-vV"])):
        proxy = proxies[name]
        native_path = command(
            [rustup["invocation"], "which", name], source, environment, f"locate {name}"
        )
        require(Path(native_path).is_absolute(), f"Invalid native {name} path")
        native = file_identity(Path(native_path))
        require(
            Path(native["invocation"]).name == name, f"Invalid native {name} basename"
        )
        verify_executable(name, proxy, environment)
        version = version_fields(
            command(
                [proxy["invocation"], *arguments],
                source,
                environment,
                f"identify {name}",
            ),
            name,
        )
        require(
            version
            == version_fields(
                command(
                    [native["invocation"], *arguments],
                    source,
                    environment,
                    f"identify native {name}",
                ),
                name,
            )
            and version["host"] == HOST
            and version["release"] == channel,
            f"The selected {name} does not match its Rustup binary",
        )
        verify_executable(name, proxy, environment)
        require(file_identity(Path(native_path)) == native, f"Native {name} changed")
        require(
            command(
                [rustup["invocation"], "which", name],
                source,
                environment,
                f"recheck {name}",
            )
            == native_path,
            f"The selected {name} changed",
        )
        result[name] = {"invocation": proxy, "native": native, "version": version}
    verify_executable("rustup", rustup, environment)
    return result


def verify_cargo_selection(
    source: Path, environment: Mapping[str, str], toolchain: dict[str, Any]
) -> None:
    rustup = toolchain["rustup"]
    cargo = toolchain["cargo"]
    verify_executable("rustup", rustup, environment)
    verify_executable("cargo", cargo["invocation"], environment)
    require(
        command(
            [rustup["invocation"], "show", "active-toolchain"],
            source,
            environment,
            "recheck the Rust toolchain",
        ).split(" ", 1)[0]
        == toolchain["active"]
        and command(
            [rustup["invocation"], "which", "cargo"],
            source,
            environment,
            "recheck Cargo's native executable",
        )
        == cargo["native"]["invocation"]
        and file_identity(Path(cargo["native"]["invocation"])) == cargo["native"],
        "The selected native Cargo changed",
    )


def nextest_identity(
    source: Path, environment: Mapping[str, str], toolchain: dict[str, Any]
) -> dict[str, Any]:
    environment = {**environment, "RUSTUP_AUTO_INSTALL": "0"}
    invocation = toolchain["cargo"]["invocation"]

    def selected() -> str:
        listing = command(
            [invocation["invocation"], "--list", "--verbose"],
            source,
            environment,
            "identify Cargo's nextest command",
        )
        candidates = [
            match.group(1)
            for line in listing.splitlines()
            if (match := re.fullmatch(r"\s+nextest\s+(.+)", line))
        ]
        require(
            len(candidates) == 1
            and Path(candidates[0]).is_absolute()
            and Path(candidates[0]).name == "cargo-nextest",
            "Cargo does not select a supported nextest executable",
        )
        return candidates[0]

    verify_cargo_selection(source, environment, toolchain)
    path = selected()
    identity = file_identity(Path(path))
    version = version_fields(
        command(
            [invocation["invocation"], "nextest", "--version"],
            source,
            environment,
            "identify nextest",
        ),
        "cargo-nextest",
    )
    require(
        version
        == version_fields(
            command(
                [identity["invocation"], "nextest", "--version"],
                source,
                environment,
                "identify native nextest",
            ),
            "cargo-nextest",
        )
        and version["release"] == NEXTEST_VERSION
        and version["host"] == HOST,
        "The nextest executable does not match the pinned workload",
    )
    verify_cargo_selection(source, environment, toolchain)
    require(
        selected() == path and file_identity(Path(path)) == identity,
        "The selected nextest changed",
    )
    return {"executable": identity, "version": version}


def native_tool(
    source: Path, environment: Mapping[str, str], name: str
) -> dict[str, Any]:
    identity = executable_identity(name, environment)
    lines = command(
        [identity["invocation"], "--version"], source, environment, f"identify {name}"
    ).splitlines()
    require(bool(lines), f"Invalid {name} version")
    version = lines[0]
    require(
        0 < len(version) <= 512
        and all(32 <= ord(character) < 127 for character in version),
        f"Invalid {name} version",
    )
    verify_executable(name, identity, environment)
    return {"executable": identity, "version": version}


def tools_identity(source: Path, environment: Mapping[str, str]) -> dict[str, Any]:
    tools = rust_toolchain(source, environment)
    tools["nextest"] = nextest_identity(source, environment, tools)
    tools["native"] = {
        name: native_tool(source, environment, name)
        for name in ("cc", "c++", "ar", "ld", "mold", "pkg-config")
    }
    require(
        tools["native"]["ld"]["executable"]["sha256"]
        == tools["native"]["mold"]["executable"]["sha256"],
        "The Linux cache requires mold to be the selected ld",
    )
    return tools


def semantic_tools(tools: dict[str, Any]) -> dict[str, Any]:
    return {
        "cargo": {
            "sha256": tools["cargo"]["native"]["sha256"],
            "version": tools["cargo"]["version"],
        },
        "rustc": {
            "sha256": tools["rustc"]["native"]["sha256"],
            "version": tools["rustc"]["version"],
        },
        "nextest": {
            "sha256": tools["nextest"]["executable"]["sha256"],
            "version": tools["nextest"]["version"],
        },
        "native": {
            name: {"sha256": item["executable"]["sha256"], "version": item["version"]}
            for name, item in tools["native"].items()
        },
    }


def platform_identity(environment: Mapping[str, str]) -> dict[str, Any]:
    release = platform.freedesktop_os_release()
    libc = platform.libc_ver()
    require(
        platform.system() == "Linux"
        and platform.machine() == "x86_64"
        and release.get("ID") == "ubuntu"
        and release.get("VERSION_ID") == "24.04"
        and libc[0] == "glibc",
        "The cache workload supports only Ubuntu 24.04 x86_64",
    )
    return {
        "system": "Linux",
        "architecture": "x86_64",
        "distribution": "ubuntu",
        "distribution_version": "24.04",
        "libc": list(libc),
        "image": {
            key: environment[key]
            for key in ("ImageOS", "ImageVersion")
            if key in environment
        },
    }


def validate_tool_selectors(environment: Mapping[str, str]) -> None:
    for name, value in environment.items():
        if not value:
            continue
        require(
            name not in TOOL_SELECTORS
            and not re.fullmatch(r"CARGO_TARGET_.+_(?:LINKER|RUNNER)", name)
            and not re.fullmatch(
                r"(?:HOST_|TARGET_)?(?:CC|CXX|AR|LD|RANLIB)(?:_.+)?", name
            ),
            "The compilation environment selects an unsupported tool override",
        )


def environment_identity(environment: Mapping[str, str]) -> dict[str, Any]:
    validate_tool_selectors(environment)
    require(
        all(
            environment.get(name) == value
            for name, value in REQUIRED_ENVIRONMENT.items()
        ),
        "The compilation environment does not match the cache workload",
    )
    selected = {
        name: value
        for name, value in environment.items()
        if value
        and not CREDENTIAL_NAME.search(name)
        and name != "CARGO_TARGET_DIR"
        and (name in COMPILATION_VARIABLES or name.startswith(COMPILATION_PREFIXES))
    }
    registries = {
        name: value
        for name, value in environment.items()
        if value
        and not CREDENTIAL_NAME.search(name)
        and (
            name.startswith(("CARGO_REGISTRIES_", "CARGO_SOURCE_"))
            or name in {"CARGO_REGISTRY_DEFAULT", "CARGO_NET_GIT_FETCH_WITH_CLI"}
        )
    }
    return {
        "compilation": {"names": sorted(selected), "sha256": digest(selected)},
        "registries": {"names": sorted(registries), "sha256": digest(registries)},
    }


def cargo_config_paths(source: Path, cargo_home: Path) -> list[tuple[str, Path]]:
    paths: list[tuple[str, Path]] = []
    for distance, directory in enumerate((source, *source.parents)):
        for name in ("config", "config.toml"):
            path = directory / ".cargo" / name
            if path.exists() or path.is_symlink():
                paths.append((f"ancestor-{distance}/{name}", path))
    for name in ("config", "config.toml"):
        path = cargo_home / name
        if path.exists() or path.is_symlink():
            paths.append((f"cargo-home/{name}", path))
    return paths


def cargo_configuration(source: Path, cargo_home: Path) -> list[dict[str, Any]]:
    result = []
    for role, path in cargo_config_paths(source, cargo_home):
        require(not path.is_symlink(), "Symlinked Cargo configuration is unsupported")
        identity, config = read_configuration(path)
        build = config.get("build", {})
        require(
            isinstance(build, dict)
            and not any(
                name in build
                for name in (
                    "rustc",
                    "rustdoc",
                    "rustc-wrapper",
                    "rustc-workspace-wrapper",
                    "target",
                    "target-dir",
                    "build-dir",
                )
            ),
            "Cargo configuration selects an unsupported tool or output directory",
        )
        targets = config.get("target", {})
        require(
            isinstance(targets, dict)
            and all(
                isinstance(value, dict)
                and not value.get("linker")
                and not value.get("runner")
                for value in targets.values()
            ),
            "Cargo configuration selects an unsupported target tool",
        )
        configured_environment = config.get("env", {})
        require(
            isinstance(configured_environment, dict),
            "Invalid Cargo environment configuration",
        )
        require(
            not {
                "PATH",
                "CARGO_BUILD_TARGET",
                "CARGO_BUILD_TARGET_DIR",
                "CARGO_BUILD_BUILD_DIR",
                "CARGO_TARGET_DIR",
            }
            & configured_environment.keys(),
            "Cargo configuration overrides a tool or output path",
        )
        validate_tool_selectors(
            {name: str(value) for name, value in configured_environment.items()}
        )
        result.append(
            {
                "role": role,
                "path": str(path),
                "sha256": identity["sha256"],
                "size": identity["size"],
            }
        )
    return result


def paths_identity(
    workspace: Path, source: Path, environment: Mapping[str, str]
) -> dict[str, Any]:
    workspace = workspace.resolve(strict=True)
    source = source.resolve(strict=True)
    require(
        source.is_relative_to(workspace),
        "The source must be inside the Actions workspace",
    )
    require(
        not environment.get("GITHUB_WORKSPACE")
        or safe_path(environment["GITHUB_WORKSPACE"]).resolve(strict=True) == workspace,
        "The requested workspace does not match the Actions workspace",
    )
    cargo_home = safe_path(environment.get("HOME", str(Path.home()))) / ".cargo"
    require(
        not environment.get("CARGO_HOME")
        or safe_path(environment["CARGO_HOME"]).resolve() == cargo_home.resolve(),
        "The cache workload requires the default Cargo home",
    )
    target = workspace / "target"
    actual_target = Path(environment.get("CARGO_TARGET_DIR", "target"))
    if not actual_target.is_absolute():
        actual_target = source / actual_target
    require(
        actual_target.resolve() == target.resolve(),
        "The Cargo target directory does not match the cache layout",
    )
    require(
        not source.is_relative_to(target),
        "The source is inside the cache target directory",
    )
    for path in (
        target,
        target / PROFILE,
        cargo_home,
        *(cargo_home / item for item in ("registry", "git")),
    ):
        require(not path.is_symlink(), "A cache root is a symbolic link")
    require(
        not any(
            environment.get(name)
            for name in (
                "CARGO_BUILD_TARGET",
                "CARGO_BUILD_TARGET_DIR",
                "CARGO_BUILD_BUILD_DIR",
            )
        ),
        "The cache workload does not support an explicit Cargo target or build directory",
    )
    return {
        "workspace": str(workspace),
        "source": str(source),
        "cargo_home": str(cargo_home),
        "target": str(target),
        "download_paths": list(DOWNLOAD_PATHS),
        "target_paths": list(TARGET_PATHS),
    }


def validate_key_namespace(value: str) -> str:
    require(
        type(value) is str
        and len(value) <= 64
        and (not value or KEY_NAMESPACE.fullmatch(value) is not None),
        "Invalid cache key namespace",
    )
    return value


def cache_keys(identity: dict[str, Any], key_namespace: str = "") -> dict[str, str]:
    key_namespace = validate_key_namespace(key_namespace)
    dependencies = [
        {key: item[key] for key in ("path", "sha256")}
        for item in identity["source"]["inputs"]
        if item["kind"] == "dependency"
    ]
    configurations = [
        {key: item[key] for key in ("path", "sha256")}
        for item in identity["source"]["inputs"]
        if item["kind"] == "configuration"
    ]
    cargo_configs = [
        {key: item[key] for key in ("role", "sha256")}
        for item in identity["configuration"]["cargo"]
    ]
    download_compatibility = digest(
        {
            "version": VERSION,
            "platform": identity["platform"],
            "paths": list(DOWNLOAD_PATHS),
            "cargo": cargo_configs,
            "registries": identity["configuration"]["environment"]["registries"],
        }
    )
    compatibility = digest(
        {
            "version": VERSION,
            "workload": WORKLOAD,
            "platform": identity["platform"],
            "dependencies": dependencies,
            "configuration": configurations,
            "cargo": cargo_configs,
            "environment": identity["configuration"]["environment"],
            "tools": semantic_tools(identity["tools"]),
        }
    )
    namespace_component = (
        f"ns-{hashlib.sha256(key_namespace.encode('ascii')).hexdigest()}-"
        if key_namespace
        else ""
    )
    downloads_prefix = (
        f"uv-rust-downloads-v1-{namespace_component}{download_compatibility}-"
    )
    target_prefix = f"uv-rust-target-v1-{namespace_component}{compatibility}-"
    return {
        "downloads": downloads_prefix + digest(dependencies),
        "downloads_restore": downloads_prefix,
        "target": target_prefix + identity["source"]["commit"],
        "target_restore": target_prefix,
    }


def implementation_identity(
    git: dict[str, Any], environment: Mapping[str, str]
) -> dict[str, Any]:
    script = Path(__file__).resolve()
    root = Path(
        git_command(script.parent, git, environment, "rev-parse", "--show-toplevel")
    )
    require_clean_source(root, git, environment)
    return {
        "commit": git_command(root, git, environment, "rev-parse", "HEAD"),
        "tree": git_command(root, git, environment, "rev-parse", "HEAD^{tree}"),
        "script_sha256": file_identity(script)["sha256"],
        "cache_action": CACHE_ACTION,
    }


def make_identity(
    workspace: Path,
    source: Path,
    repository: str,
    commit: str,
    save_allowed: bool,
    environment: Mapping[str, str],
    *,
    key_namespace: str = "",
) -> dict[str, Any]:
    key_namespace = validate_key_namespace(key_namespace)
    environment = dict(environment)
    workspace = safe_path(workspace).resolve(strict=True)
    source = safe_path(source).resolve(strict=True)
    paths = paths_identity(workspace, source, environment)
    git = executable_identity("git", environment)
    original_source = source_identity(source, repository, commit, git, environment)
    identity = {
        "source": original_source,
        "platform": platform_identity(environment),
        "configuration": {
            "environment": environment_identity(environment),
            "cargo": cargo_configuration(source, Path(paths["cargo_home"])),
        },
        "tools": tools_identity(source, environment),
        "paths": paths,
        "implementation": implementation_identity(git, environment),
        "git": git,
        "github": {
            key: environment[key]
            for key in (
                "GITHUB_REPOSITORY",
                "GITHUB_SHA",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "GITHUB_JOB",
                "GITHUB_WORKFLOW_REF",
                "GITHUB_WORKFLOW_SHA",
            )
            if key in environment
        },
    }
    require(
        source_identity(source, repository, commit, git, environment)
        == original_source,
        "The source changed during preparation",
    )
    verify_executable("git", git, environment)
    policy: dict[str, bool | str] = {"save_allowed": save_allowed}
    if key_namespace:
        policy["key_namespace"] = key_namespace
    return {
        "format": FORMAT,
        "version": VERSION,
        "workload": WORKLOAD,
        "identity": identity,
        "policy": policy,
        "keys": cache_keys(identity, key_namespace),
    }


def read_json(path: Path, expected_sha256: str) -> dict[str, Any]:
    require(SHA256.fullmatch(expected_sha256) is not None, "Invalid manifest digest")
    with path.open("rb") as stream:
        require(
            stat.S_ISREG(os.fstat(stream.fileno()).st_mode),
            "Expected a regular manifest file",
        )
        data = stream.read(MAX_JSON_BYTES + 1)
    require(
        len(data) <= MAX_JSON_BYTES
        and hashlib.sha256(data).hexdigest() == expected_sha256,
        "Manifest digest or size mismatch",
    )

    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result = {}
        for key, value in pairs:
            require(key not in result, "Duplicate manifest field")
            result[key] = value
        return result

    def nonfinite(_value: str) -> None:
        raise CacheError("Non-finite manifest number")

    value = json.loads(data, object_pairs_hook=unique, parse_constant=nonfinite)
    require(isinstance(value, dict), "Expected a manifest object")
    return value


def write_json(path: Path, value: dict[str, Any]) -> str:
    data = json_bytes(value)
    require(len(data) <= MAX_JSON_BYTES, "Manifest exceeds its size limit")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)
    return hashlib.sha256(data).hexdigest()


def manifest_output(path: Path, manifest: dict[str, Any]) -> Path:
    output = safe_path(path).resolve(strict=False)
    require(
        not output.is_relative_to(Path(manifest["identity"]["paths"]["workspace"])),
        "The manifest must be outside the Actions workspace",
    )
    return output


def verify_identity(manifest: dict[str, Any], environment: Mapping[str, str]) -> None:
    require(
        set(manifest) == {"format", "version", "workload", "identity", "policy", "keys"}
        and manifest["format"] == FORMAT
        and type(manifest["version"]) is int
        and manifest["version"] == VERSION
        and manifest["workload"] == WORKLOAD
        and isinstance(manifest["policy"], dict)
        and set(manifest["policy"])
        in ({"save_allowed"}, {"save_allowed", "key_namespace"})
        and type(manifest["policy"].get("save_allowed")) is bool
        and (
            "key_namespace" not in manifest["policy"]
            or (
                type(manifest["policy"]["key_namespace"]) is str
                and bool(manifest["policy"]["key_namespace"])
            )
        ),
        "Unsupported cache manifest",
    )
    key_namespace = validate_key_namespace(manifest["policy"].get("key_namespace", ""))
    identity = manifest["identity"]
    current = make_identity(
        Path(identity["paths"]["workspace"]),
        Path(identity["source"]["path"]),
        identity["source"]["repository"],
        identity["source"]["commit"],
        manifest["policy"]["save_allowed"],
        environment,
        key_namespace=key_namespace,
    )
    require(
        canonical_json(current) == canonical_json(manifest),
        "The cache source, configuration, tools, or paths changed",
    )


def restore_observation(
    keys: dict[str, str], kind: str, primary: str, matched: str, hit: str
) -> dict[str, Any]:
    requested = keys[kind]
    prefix = keys[kind + "_restore"]
    suffix = OID if kind == "target" else SHA256
    require(
        primary in {"", requested}, "The cache action used an unexpected primary key"
    )
    require(hit in {"", "true", "false"}, "Invalid exact-hit output")
    require(
        not matched
        or (
            matched.startswith(prefix)
            and suffix.fullmatch(matched.removeprefix(prefix)) is not None
        ),
        "The cache action matched an incompatible key",
    )
    require(
        (hit == "true") == (matched == requested), "Contradictory cache-hit outputs"
    )
    require(bool(primary) or not matched, "A matched cache has no primary key")
    return {
        "requested": requested,
        "primary": primary,
        "matched": matched or None,
        "exact": hit == "true",
    }


def write_outputs(path: Path, values: Mapping[str, str]) -> None:
    with path.open("a", encoding="utf-8") as stream:
        for name, value in values.items():
            require(
                re.fullmatch(r"[a-z][a-z0-9-]*", name) is not None,
                "Invalid output name",
            )
            delimiter = "uv-cache-" + uuid.uuid4().hex
            require(delimiter not in value.splitlines(), "Invalid output delimiter")
            stream.write(f"{name}<<{delimiter}\n{value}\n{delimiter}\n")


def common_outputs(manifest: dict[str, Any]) -> dict[str, str]:
    return {
        "downloads-key": manifest["keys"]["downloads"],
        "downloads-restore-key": manifest["keys"]["downloads_restore"],
        "downloads-paths": "\n".join(DOWNLOAD_PATHS),
        "target-key": manifest["keys"]["target"],
        "target-restore-key": manifest["keys"]["target_restore"],
        "target-paths": "\n".join(TARGET_PATHS),
        "cargo": manifest["identity"]["tools"]["cargo"]["invocation"]["invocation"],
        "cargo-target-dir": manifest["identity"]["paths"]["target"],
    }


def restored_manifest(
    manifest: dict[str, Any], observations: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    return {
        "format": FORMAT + "-restored",
        "version": VERSION,
        "identity": manifest,
        "identity_sha256": hashlib.sha256(json_bytes(manifest)).hexdigest(),
        "restore": observations,
    }


def save_plan(
    state: dict[str, Any], environment: Mapping[str, str], job_status: str
) -> dict[str, str]:
    require(
        set(state) == {"format", "version", "identity", "identity_sha256", "restore"}
        and state["format"] == FORMAT + "-restored"
        and type(state["version"]) is int
        and state["version"] == VERSION,
        "Unsupported restored-cache manifest",
    )
    manifest = state["identity"]
    require(
        isinstance(manifest, dict)
        and hashlib.sha256(json_bytes(manifest)).hexdigest()
        == state["identity_sha256"],
        "Identity digest mismatch",
    )
    verify_identity(manifest, environment)
    require(
        isinstance(state["restore"], dict)
        and set(state["restore"]) == {"downloads", "target"},
        "Invalid restore observations",
    )
    observations = {}
    for kind in ("downloads", "target"):
        item = state["restore"][kind]
        require(
            isinstance(item, dict)
            and set(item) == {"requested", "primary", "matched", "exact"}
            and isinstance(item["requested"], str)
            and isinstance(item["primary"], str)
            and (item["matched"] is None or isinstance(item["matched"], str))
            and type(item["exact"]) is bool,
            "Invalid restore observation",
        )
        observations[kind] = restore_observation(
            manifest["keys"],
            kind,
            item["primary"],
            item["matched"] or "",
            str(item["exact"]).lower(),
        )
        require(observations[kind] == item, "Invalid restore observation")
    require(job_status in {"success", "failure", "cancelled"}, "Invalid job status")
    allowed = manifest["policy"]["save_allowed"] and job_status == "success"
    return {
        **common_outputs(manifest),
        "save-downloads": str(
            allowed and not observations["downloads"]["exact"]
        ).lower(),
        "save-target": str(allowed and not observations["target"]["exact"]).lower(),
    }


def boolean(value: str) -> bool:
    require(value in {"true", "false"}, "Expected true or false")
    return value == "true"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser("prepare")
    prepare.add_argument("--workspace", type=Path, required=True)
    prepare.add_argument("--source", type=Path, required=True)
    prepare.add_argument("--repository", required=True)
    prepare.add_argument("--commit", required=True)
    prepare.add_argument("--save-if", type=boolean, required=True)
    prepare.add_argument("--key-namespace", default="")
    prepare.add_argument("--output", type=Path, required=True)
    prepare.add_argument("--github-output", type=Path, required=True)
    observe = subparsers.add_parser("observe")
    observe.add_argument("--identity", type=Path, required=True)
    observe.add_argument("--identity-sha256", required=True)
    observe.add_argument("--output", type=Path, required=True)
    observe.add_argument("--github-output", type=Path, required=True)
    for kind in ("downloads", "target"):
        for field in ("primary", "matched", "hit"):
            observe.add_argument(f"--{kind}-{field}", default="")
    verify = subparsers.add_parser("verify")
    verify.add_argument("--manifest", type=Path, required=True)
    verify.add_argument("--manifest-sha256", required=True)
    verify.add_argument("--github-output", type=Path, required=True)
    verify.add_argument(
        "--job-status", choices=("success", "failure", "cancelled"), default="success"
    )
    arguments = parser.parse_args()
    environment = dict(os.environ)
    if arguments.command == "prepare":
        manifest = make_identity(
            arguments.workspace,
            arguments.source,
            arguments.repository,
            arguments.commit,
            arguments.save_if,
            environment,
            key_namespace=arguments.key_namespace,
        )
        output = manifest_output(arguments.output, manifest)
        checksum = write_json(output, manifest)
        write_outputs(
            arguments.github_output,
            {
                **common_outputs(manifest),
                "identity": str(output),
                "identity-sha256": checksum,
            },
        )
    elif arguments.command == "observe":
        manifest = read_json(arguments.identity, arguments.identity_sha256)
        verify_identity(manifest, environment)
        observations = {
            kind: restore_observation(
                manifest["keys"],
                kind,
                getattr(arguments, kind + "_primary"),
                getattr(arguments, kind + "_matched"),
                getattr(arguments, kind + "_hit"),
            )
            for kind in ("downloads", "target")
        }
        state = restored_manifest(manifest, observations)
        output = manifest_output(arguments.output, manifest)
        checksum = write_json(output, state)
        write_outputs(
            arguments.github_output,
            {
                **common_outputs(manifest),
                "manifest": str(output),
                "manifest-sha256": checksum,
                "cache-hit": str(observations["target"]["exact"]).lower(),
                "downloads-cache-hit": str(observations["downloads"]["exact"]).lower(),
                "cache-matched-key": observations["target"]["matched"] or "",
            },
        )
    else:
        state = read_json(arguments.manifest, arguments.manifest_sha256)
        write_outputs(
            arguments.github_output,
            save_plan(state, environment, arguments.job_status),
        )


if __name__ == "__main__":
    try:
        main()
    except CacheError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from None
    except (
        OSError,
        KeyError,
        TypeError,
        AttributeError,
        ValueError,
    ):
        print(
            "error: Could not inspect the cache identity or manifest", file=sys.stderr
        )
        raise SystemExit(1) from None
