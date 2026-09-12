"""Qualify optional installed metadata recovery and conservative reinstall behavior."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import platform
import re
import subprocess
import tarfile
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

NAME = "installed-sidecar-fixture"
MODULE = NAME.replace("-", "_")
VERSION = "1.0.0"
WHEEL_NAME = "click"
WHEEL_VERSION = "8.4.2"
WHEEL_SHA256 = "e6f9f66136c816745b9d65817da91d61d957fb16e02e4dcd0552553c5a197b76"
CANARY = "https://user:sidecar-secret@example.invalid/a?sig=sidecar-signature"
SECRETS = ("sidecar-secret", "sidecar-signature")
POLICIES = ("strict", "ignore-json-errors", "invalidate-malformed")
SIDECARS = ("uv_cache.json", "uv_build.json")
PROBE = """\
import hashlib
import importlib
import importlib.metadata
import json
import sys
from pathlib import Path

distribution = importlib.metadata.distribution(sys.argv[1])
metadata = [file for file in distribution.files or [] if str(file).endswith('.dist-info/METADATA')]
if len(metadata) != 1:
    raise RuntimeError('Expected one recorded METADATA file')
module = importlib.import_module(sys.argv[2])
path = Path(module.__file__)
print(json.dumps({
    'version': distribution.version,
    'dist_info': str(Path(distribution.locate_file(metadata[0])).parent),
    'module_path': str(path),
    'module_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
    'value': getattr(module, 'VALUE', None),
    'python_version': list(sys.version_info[:3]),
}))
"""
PAYLOAD_PROBE = """\
import hashlib
import importlib
import json
import sys
from pathlib import Path

module = importlib.import_module(sys.argv[1])
path = Path(module.__file__)
print(json.dumps({
    'module_path': str(path),
    'module_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
    'value': getattr(module, 'VALUE', None),
}))
"""


def sha256(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def atomic_write(path: Path, contents: bytes) -> None:
    # Installed wheel files can be hardlinked into a package cache. Replace the directory entry
    # instead of modifying a shared inode when constructing an invalid-metadata fixture.
    temporary = path.with_name(path.name + ".qualification-tmp")
    with temporary.open("xb") as file:
        file.write(contents)
    os.replace(temporary, path)


def state(path: Path) -> dict:
    if not path.exists():
        return {"kind": "missing"}
    if path.is_dir():
        return {"kind": "directory"}
    contents = path.read_bytes()
    result = {
        "kind": "file",
        "bytes": len(contents),
        "sha256": hashlib.sha256(contents).hexdigest(),
    }
    try:
        value = json.loads(contents)
    except (UnicodeError, ValueError):
        result["json_type"] = "invalid"
    else:
        result["json_type"] = type(value).__name__
        if isinstance(value, dict):
            settings = value.get("config_settings")
            if isinstance(settings, dict):
                flavor = settings.get("flavor")
                if isinstance(flavor, str) and flavor in {"alpha", "beta", "default"}:
                    result["fixture_flavor"] = flavor
    return result


def install_work(stderr: str) -> dict[str, int]:
    work = {
        action.lower(): 0
        for action in ("Resolved", "Prepared", "Uninstalled", "Installed", "Audited")
    }
    for action, count in re.findall(
        r"(?m)^(Resolved|Prepared|Uninstalled|Installed|Audited) ([0-9]+) packages? ",
        stderr,
    ):
        work[action.lower()] += int(count)
    return work


def leaks_values(result: dict) -> bool:
    return any(
        secret in result[stream]
        for secret in SECRETS
        for stream in ("stdout", "stderr")
    )


def run(command: list[str], directory: Path, environment: dict, timeout: float) -> dict:
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
    return {
        "command": command,
        "exit_code": result.returncode,
        "elapsed_seconds": time.perf_counter() - started,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def require_success(result: dict) -> None:
    if result["exit_code"] != 0:
        raise AssertionError(f"Fixture command failed: {result}")


def check_outcome(result: dict, expected: str) -> None:
    if expected == "success":
        require_success(result)
    elif expected == "hash-mismatch":
        if result["exit_code"] == 0 or "Hash mismatch" not in result["stderr"]:
            raise AssertionError(f"Expected an artifact hash mismatch: {result}")
    elif expected == "reject":
        if result["exit_code"] == 0:
            raise AssertionError(f"Expected the command to fail: {result}")
    else:
        raise ValueError(f"Unknown expected outcome: {expected}")


def check_inspection_output(result: dict, command: str) -> None:
    if command == "list":
        expected = [{"name": NAME, "version": VERSION}]
        if json.loads(result["stdout"]) != expected:
            raise AssertionError(
                f"Installed package is missing from the list: {result}"
            )
    elif command == "show":
        if not result["stdout"].startswith(f"Name: {NAME}\nVersion: {VERSION}\n"):
            raise AssertionError(
                f"Installed package is missing from the report: {result}"
            )
    elif command == "tree" and result["stdout"].strip() != f"{NAME} v{VERSION}":
        raise AssertionError(f"Installed package is missing from the tree: {result}")


def make_source_archive(directory: Path, backend: Path) -> Path:
    prefix = f"{MODULE}-{VERSION}"
    metadata = (
        f"Metadata-Version: 2.3\nName: {NAME}\nVersion: {VERSION}\n"
        "Requires-Python: >=3.9\n\n"
    )
    files = {
        "PKG-INFO": metadata.encode(),
        "backend.py": backend.read_bytes(),
        "pyproject.toml": (
            '[build-system]\nrequires = []\nbuild-backend = "backend"\n'
            'backend-path = ["."]\n\n[project]\n'
            f'name = "{NAME}"\nversion = "{VERSION}"\nrequires-python = ">=3.9"\n'
        ).encode(),
    }
    data = io.BytesIO()
    with (
        gzip.GzipFile(filename="", mode="wb", fileobj=data, mtime=0) as compressed,
        tarfile.open(
            fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT
        ) as archive,
    ):
        for name, contents in sorted(files.items()):
            member = tarfile.TarInfo(f"{prefix}/{name}")
            member.mode = 0o644
            member.size = len(contents)
            archive.addfile(member, io.BytesIO(contents))
    path = directory / f"{prefix}.tar.gz"
    contents = data.getvalue()
    if path.exists():
        if path.read_bytes() != contents:
            raise ValueError(f"Existing source fixture has different contents: {path}")
    else:
        with path.open("xb") as file:
            file.write(contents)
    return path


@dataclass(frozen=True)
class Fixture:
    name: str
    version: str
    module: str
    archive: Path
    digest: str
    source: bool
    registry: bool = False


@dataclass(frozen=True)
class FreshnessCase:
    name: str
    fixture: str
    mutation: str = "none"
    flavor: str | None = None
    wrong_hash: bool = False
    reinstall: bool = False
    upgrade: bool = False
    outcome: str = "success"
    value: str | None = None
    installed: int = 0
    built: int = 0
    strict_outcome: str | None = None
    fallback_outcome: str | None = None
    fallback_value: str | None = None
    fallback_installed: int | None = None
    fallback_built: int | None = None


FRESHNESS_CASES = (
    FreshnessCase("wheel-fresh", "wheel"),
    FreshnessCase("wheel-missing-cache", "wheel", "cache-missing", installed=1),
    FreshnessCase("wheel-malformed-cache", "wheel", "cache-syntax", installed=1),
    FreshnessCase("wheel-valid-wrong-hash", "wheel", wrong_hash=True),
    FreshnessCase(
        "wheel-missing-cache-wrong-hash",
        "wheel",
        "cache-missing",
        wrong_hash=True,
        outcome="hash-mismatch",
    ),
    FreshnessCase(
        "wheel-malformed-cache-wrong-hash",
        "wheel",
        "cache-syntax",
        wrong_hash=True,
        outcome="hash-mismatch",
    ),
    FreshnessCase(
        "wheel-malformed-cache-reinstall",
        "wheel",
        "cache-syntax",
        reinstall=True,
        installed=1,
    ),
    FreshnessCase(
        "wheel-malformed-cache-reinstall-wrong-hash",
        "wheel",
        "cache-syntax",
        wrong_hash=True,
        reinstall=True,
        outcome="hash-mismatch",
    ),
    FreshnessCase("source-fresh", "source", flavor="alpha", value="alpha"),
    FreshnessCase(
        "source-changed-settings",
        "source",
        flavor="beta",
        value="beta",
        installed=1,
        built=1,
    ),
    FreshnessCase(
        "source-missing-build-settings",
        "source",
        "build-missing",
        flavor="beta",
        value="alpha",
    ),
    FreshnessCase(
        "source-malformed-build-settings",
        "source",
        "build-syntax",
        flavor="beta",
        value="beta",
        installed=1,
        built=1,
        fallback_value="alpha",
        fallback_installed=0,
        fallback_built=0,
    ),
    FreshnessCase(
        "source-malformed-build-default-settings",
        "source",
        "build-syntax",
        value="default",
        installed=1,
        built=1,
        fallback_value="alpha",
        fallback_installed=0,
        fallback_built=0,
    ),
    FreshnessCase(
        "source-missing-cache",
        "source",
        "cache-missing",
        flavor="alpha",
        value="alpha",
        installed=1,
    ),
    FreshnessCase(
        "source-malformed-cache",
        "source",
        "cache-syntax",
        flavor="alpha",
        value="alpha",
        installed=1,
    ),
    FreshnessCase(
        "source-valid-settings-wrong-hash",
        "source",
        flavor="beta",
        wrong_hash=True,
        outcome="hash-mismatch",
        value="alpha",
    ),
    FreshnessCase(
        "source-missing-build-settings-wrong-hash",
        "source",
        "build-missing",
        flavor="beta",
        wrong_hash=True,
        value="alpha",
    ),
    FreshnessCase(
        "source-malformed-build-settings-wrong-hash",
        "source",
        "build-syntax",
        flavor="beta",
        wrong_hash=True,
        outcome="hash-mismatch",
        value="alpha",
        fallback_outcome="success",
    ),
    FreshnessCase(
        "source-malformed-build-reinstall-wrong-hash",
        "source",
        "build-syntax",
        flavor="beta",
        wrong_hash=True,
        reinstall=True,
        outcome="hash-mismatch",
        value="alpha",
        strict_outcome="hash-mismatch",
    ),
    FreshnessCase(
        "source-malformed-build-reinstall",
        "source",
        "build-syntax",
        flavor="beta",
        reinstall=True,
        value="beta",
        installed=1,
        built=1,
    ),
    FreshnessCase(
        "source-both-missing-settings",
        "source",
        "both-missing",
        flavor="beta",
        value="beta",
        installed=1,
        built=1,
    ),
    FreshnessCase(
        "source-both-malformed-settings",
        "source",
        "both-syntax",
        flavor="beta",
        value="beta",
        installed=1,
        built=1,
    ),
    FreshnessCase(
        "registry-source-fresh", "registry-source", flavor="alpha", value="alpha"
    ),
    FreshnessCase(
        "registry-source-valid-changed-settings",
        "registry-source",
        flavor="beta",
        value="alpha",
    ),
    FreshnessCase(
        "registry-source-missing-build-settings",
        "registry-source",
        "build-missing",
        flavor="beta",
        value="alpha",
    ),
    FreshnessCase(
        "registry-source-missing-cache",
        "registry-source",
        "cache-missing",
        flavor="alpha",
        value="alpha",
    ),
    FreshnessCase(
        "registry-source-malformed-build-settings",
        "registry-source",
        "build-syntax",
        flavor="beta",
        value="beta",
        installed=1,
        built=1,
        fallback_value="alpha",
        fallback_installed=0,
        fallback_built=0,
    ),
    FreshnessCase(
        "registry-source-malformed-build-settings-wrong-hash",
        "registry-source",
        "build-syntax",
        flavor="beta",
        wrong_hash=True,
        outcome="hash-mismatch",
        value="alpha",
        fallback_outcome="success",
    ),
    FreshnessCase(
        "registry-source-malformed-cache",
        "registry-source",
        "cache-syntax",
        flavor="alpha",
        value="alpha",
        installed=1,
        fallback_installed=0,
    ),
    FreshnessCase(
        "registry-source-malformed-cache-wrong-hash",
        "registry-source",
        "cache-syntax",
        flavor="alpha",
        wrong_hash=True,
        outcome="hash-mismatch",
        value="alpha",
        fallback_outcome="success",
    ),
    FreshnessCase(
        "registry-source-malformed-build-upgrade",
        "registry-source",
        "build-syntax",
        flavor="beta",
        upgrade=True,
        value="beta",
        installed=1,
        built=1,
        fallback_value="alpha",
        fallback_installed=0,
        fallback_built=0,
    ),
    FreshnessCase(
        "registry-source-malformed-build-upgrade-wrong-hash",
        "registry-source",
        "build-syntax",
        flavor="beta",
        wrong_hash=True,
        upgrade=True,
        outcome="hash-mismatch",
        value="alpha",
        fallback_outcome="success",
    ),
)


class Environment:
    def __init__(self, args: argparse.Namespace, root: Path, fixture: Fixture) -> None:
        self.args = args
        self.root = root
        self.fixture = fixture
        self.cache = root / "cache"
        self.venv = root / "venv"
        self.python = self.venv / (
            "Scripts/python.exe" if os.name == "nt" else "bin/python"
        )
        self.events_path = root / "build-events.jsonl"
        self.environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("UV_")
            and key
            not in {
                "VIRTUAL_ENV",
                "CONDA_PREFIX",
                "PYTHONPATH",
                "PYTHONHOME",
                "RUST_LOG",
                "SIDECAR_QUALIFICATION_LOG",
            }
        }
        self.environment.update(
            UV_PYTHON_DOWNLOADS="never",
            PYTHONNOUSERSITE="1",
            PYTHONDONTWRITEBYTECODE="1",
            RUST_LOG="warn",
            SIDECAR_QUALIFICATION_LOG=str(self.events_path),
        )
        self.prefix = [
            str(args.uv),
            "--no-config",
            "--no-progress",
            "--color",
            "never",
            "--offline",
            "--cache-dir",
            str(self.cache),
        ]
        self.setup = []
        self.dist_info: Path | None = None

    def command(self, *arguments: str) -> dict:
        return run(
            [*self.prefix, *arguments], self.root, self.environment, self.args.timeout
        )

    def install(
        self,
        *,
        flavor: str | None = None,
        wrong_hash: bool = False,
        reinstall: bool = False,
        upgrade: bool = False,
        no_installer_metadata: bool = False,
    ) -> dict:
        digest = "0" * 64 if wrong_hash else self.fixture.digest
        requirements = self.root / ("wrong.txt" if wrong_hash else "requirements.txt")
        requirement = (
            f"{self.fixture.name}=={self.fixture.version}"
            if self.fixture.registry
            else f"{self.fixture.name} @ {self.fixture.archive.as_uri()}"
        )
        requirements.write_text(f"{requirement} --hash=sha256:{digest}\n")
        arguments = [
            "pip",
            "install",
            "--python",
            str(self.python),
            "--no-index",
            "--no-deps",
            "--require-hashes",
            "--requirements",
            str(requirements),
        ]
        if self.fixture.registry:
            arguments.extend(["--find-links", str(self.fixture.archive.parent)])
        if flavor is not None:
            arguments.extend(["--config-setting", f"flavor={flavor}"])
        if reinstall:
            arguments.append("--reinstall")
        if upgrade:
            arguments.append("--upgrade")
        if no_installer_metadata:
            arguments.append("--no-installer-metadata")
        return self.command(*arguments)

    def initialize(self, *, no_installer_metadata: bool = False) -> dict:
        created = self.command(
            "venv", "--python", str(self.args.python), str(self.venv)
        )
        require_success(created)
        installed = self.install(
            flavor="alpha" if self.fixture.source else None,
            no_installer_metadata=no_installer_metadata,
        )
        require_success(installed)
        self.setup.extend([created, installed])
        probed = run(
            [str(self.python), "-c", PROBE, self.fixture.name, self.fixture.module],
            self.root,
            self.environment,
            self.args.timeout,
        )
        require_success(probed)
        probe = json.loads(probed["stdout"])
        if probe["version"] != self.fixture.version or probe["python_version"] != [
            3,
            12,
            11,
        ]:
            raise AssertionError(f"Unexpected installed fixture: {probe}")
        self.dist_info = Path(probe["dist_info"])
        if not self.dist_info.is_relative_to(self.venv):
            raise AssertionError(
                f"Fixture was installed outside its environment: {probe}"
            )
        if self.fixture.registry and (self.dist_info / "direct_url.json").exists():
            raise AssertionError("Registry fixture was recorded as a direct URL")
        if self.fixture.source and not no_installer_metadata:
            record = (self.dist_info / "RECORD").read_text()
            for name in SIDECARS:
                if not (self.dist_info / name).is_file() or f"/{name}," not in record:
                    raise AssertionError(f"Source build did not record {name}")
        if no_installer_metadata and any(
            (self.dist_info / name).exists() for name in SIDECARS
        ):
            raise AssertionError(
                "Installer metadata was written despite the explicit opt-out"
            )
        return probe

    def payload(self) -> dict:
        result = run(
            [str(self.python), "-c", PAYLOAD_PROBE, self.fixture.module],
            self.root,
            self.environment,
            self.args.timeout,
        )
        require_success(result)
        return json.loads(result["stdout"])

    def snapshot(self) -> dict:
        if self.dist_info is None:
            raise RuntimeError("The environment has not been initialized")
        return {
            name: state(self.dist_info / name)
            for name in (*SIDECARS, "METADATA", "RECORD", "direct_url.json")
        }

    def events(self) -> list[dict]:
        if not self.events_path.exists():
            return []
        return [json.loads(line) for line in self.events_path.read_text().splitlines()]

    def mutate(self, mutation: str) -> dict | None:
        if self.dist_info is None:
            raise RuntimeError("The environment has not been initialized")
        if mutation == "none":
            return None
        if mutation == "core-missing":
            (self.dist_info / "METADATA").unlink()
            return None
        if mutation == "core-invalid":
            atomic_write(
                self.dist_info / "METADATA",
                f"Metadata-Version: 2.3\nVersion: {self.fixture.version}\n\n".encode(),
            )
            return None
        target, operation = mutation.split("-", 1)
        names = SIDECARS if target == "both" else (f"uv_{target}.json",)
        io_probe = None
        for name in names:
            path = self.dist_info / name
            if operation == "missing":
                path.unlink(missing_ok=True)
            elif operation in {"syntax", "data"}:
                contents = (
                    b"{" if operation == "syntax" else json.dumps(CANARY).encode()
                )
                atomic_write(path, contents)
            elif operation == "io":
                path.unlink()
                path.mkdir()
                try:
                    descriptor = os.open(path, os.O_RDONLY)
                except OSError as error:
                    io_probe = {"stage": "open", "errno": error.errno}
                else:
                    try:
                        os.read(descriptor, 1)
                    except OSError as error:
                        io_probe = {"stage": "read", "errno": error.errno}
                    finally:
                        os.close(descriptor)
                if io_probe is None:
                    raise AssertionError(
                        "The directory fixture did not cause an I/O error"
                    )
            else:
                raise ValueError(f"Unknown mutation: {mutation}")
        return io_probe


class Qualification:
    def __init__(
        self, args: argparse.Namespace, report: dict, fixtures: dict[str, Fixture]
    ):
        self.args = args
        self.report = report
        self.fixtures = fixtures

    def selected(self, name: str) -> bool:
        return not self.args.case or name in self.args.case

    def record(self, case: dict) -> None:
        self.report["cases"].append(case | {"success": True})

    def inspection(self, mutation: str) -> None:
        name = f"inspect-{mutation}"
        if not self.selected(name):
            return
        with tempfile.TemporaryDirectory(
            prefix=name + "-", dir=self.args.work_directory
        ) as temporary:
            environment = Environment(
                self.args, Path(temporary), self.fixtures["source"]
            )
            probe = environment.initialize(no_installer_metadata=mutation == "opt-out")
            io_probe = environment.mutate("none" if mutation == "opt-out" else mutation)
            before = environment.snapshot()
            payload_before = environment.payload()
            malformed = mutation.endswith(("syntax", "data"))
            expected = "success"
            if (malformed and self.args.expect == "strict") or (
                mutation.endswith("-io")
                and (
                    self.args.expect != "ignore-json-errors"
                    or io_probe is None
                    or io_probe["stage"] == "open"
                )
            ):
                expected = "reject"
            commands = []
            for arguments in [
                [
                    "pip",
                    "list",
                    "--format",
                    "json",
                    "--python",
                    str(environment.python),
                ],
                ["pip", "show", "--python", str(environment.python), NAME],
                ["pip", "tree", "--python", str(environment.python), "--package", NAME],
            ]:
                result = environment.command(*arguments)
                check_outcome(result, expected)
                if expected == "success":
                    check_inspection_output(result, arguments[1])
                elif result["stdout"]:
                    raise AssertionError(
                        f"Rejected inspection emitted a partial report: {result}"
                    )
                leaked = leaks_values(result)
                if mutation.endswith("-data") and leaked != (
                    self.args.expect != "invalidate-malformed"
                ):
                    raise AssertionError(
                        f"Unexpected diagnostic value handling: {result}"
                    )
                commands.append(result | {"leaked_values": leaked})
            after = environment.snapshot()
            if before != after or payload_before != environment.payload():
                raise AssertionError("Inspection modified the installed distribution")
            self.record(
                {
                    "name": name,
                    "fixture": "source",
                    "mutation": mutation,
                    "expected_outcome": expected,
                    "io_probe": io_probe,
                    "initial_probe": probe,
                    "setup": environment.setup,
                    "before": before,
                    "after": after,
                    "commands": commands,
                }
            )

    def required_metadata(self, mutation: str) -> None:
        name = f"required-{mutation}"
        if not self.selected(name):
            return
        with tempfile.TemporaryDirectory(
            prefix=name + "-", dir=self.args.work_directory
        ) as temporary:
            environment = Environment(
                self.args, Path(temporary), self.fixtures["source"]
            )
            probe = environment.initialize()
            environment.mutate(mutation)
            before = environment.snapshot()
            listed = environment.command(
                "pip", "list", "--format", "json", "--python", str(environment.python)
            )
            require_success(listed)
            check_inspection_output(listed, "list")
            tree = environment.command(
                "pip", "tree", "--python", str(environment.python), "--package", NAME
            )
            check_outcome(tree, "reject")
            if tree["stdout"]:
                raise AssertionError(
                    f"Required-metadata failure emitted a partial tree: {tree}"
                )
            if "METADATA" not in tree["stderr"]:
                raise AssertionError(
                    f"Required metadata failure lost its file context: {tree}"
                )
            after = environment.snapshot()
            if before != after:
                raise AssertionError(
                    "Required-metadata inspection modified the distribution"
                )
            self.record(
                {
                    "name": name,
                    "fixture": "source",
                    "mutation": mutation,
                    "expected_outcome": "required-reader-rejects",
                    "initial_probe": probe,
                    "setup": environment.setup,
                    "before": before,
                    "after": after,
                    "commands": [listed, tree],
                }
            )

    def freshness(self, case: FreshnessCase) -> None:
        if not self.selected(case.name):
            return
        with tempfile.TemporaryDirectory(
            prefix=case.name + "-", dir=self.args.work_directory
        ) as temporary:
            environment = Environment(
                self.args, Path(temporary), self.fixtures[case.fixture]
            )
            probe = environment.initialize()
            environment.mutate(case.mutation)
            before = environment.snapshot()
            payload_before = environment.payload()
            events_before = environment.events()
            result = environment.install(
                flavor=case.flavor,
                wrong_hash=case.wrong_hash,
                reinstall=case.reinstall,
                upgrade=case.upgrade,
            )
            outcome = case.outcome
            value = case.value
            installed = case.installed
            built = case.built
            if self.args.expect == "strict" and case.mutation.endswith(
                ("syntax", "data")
            ):
                outcome, value, installed, built = (
                    case.strict_outcome or "reject",
                    payload_before["value"],
                    0,
                    0,
                )
            elif self.args.expect == "ignore-json-errors":
                outcome = case.fallback_outcome or outcome
                value = (
                    case.fallback_value if case.fallback_value is not None else value
                )
                installed = (
                    case.fallback_installed
                    if case.fallback_installed is not None
                    else installed
                )
                built = (
                    case.fallback_built if case.fallback_built is not None else built
                )
            check_outcome(result, outcome)
            work = install_work(result["stderr"])
            payload_after = environment.payload()
            events = environment.events()[len(events_before) :]
            completed_builds = [event for event in events if event["phase"] == "end"]
            if work["installed"] != installed or len(completed_builds) != built:
                raise AssertionError(
                    f"Unexpected installation/build work: {case.name}, {work}, {events}"
                )
            if payload_after["value"] != value:
                raise AssertionError(
                    f"Unexpected installed build flavor: {case.name}, {payload_after}"
                )
            if outcome != "success" and payload_before != payload_after:
                raise AssertionError(
                    "A rejected operation changed the installed payload"
                )
            after = environment.snapshot()
            if not installed and before != after:
                raise AssertionError(
                    "An operation without installation changed installed metadata"
                )
            if installed:
                for sidecar in (
                    SIDECARS if environment.fixture.source else ("uv_cache.json",)
                ):
                    if after[sidecar].get("json_type") != "dict":
                        raise AssertionError(
                            f"Reinstallation did not repair {sidecar}: {after}"
                        )
            self.record(
                {
                    "name": case.name,
                    "fixture": case.fixture,
                    "mutation": case.mutation,
                    "requested_flavor": case.flavor,
                    "wrong_hash": case.wrong_hash,
                    "reinstall": case.reinstall,
                    "upgrade": case.upgrade,
                    "expected_outcome": outcome,
                    "expected_value": value,
                    "expected_installed": installed,
                    "expected_builds": built,
                    "initial_probe": probe,
                    "setup": environment.setup,
                    "before": before,
                    "after": after,
                    "payload_before": payload_before,
                    "payload_after": payload_after,
                    "command": result,
                    "install_work": work,
                    "build_events": events,
                    "completed_builds": len(completed_builds),
                }
            )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--wheel", type=Path, required=True)
    parser.add_argument("--expect", choices=POLICIES, required=True)
    parser.add_argument(
        "--work-directory",
        type=Path,
        default=root / ".cache/installed-sidecar-qualification",
    )
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    for name in ("uv", "python", "wheel"):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    if sha256(args.wheel) != WHEEL_SHA256:
        parser.error("wheel must be the pinned click-8.4.2-py3-none-any.whl")
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    fixture_directory = args.work_directory / "fixtures"
    fixture_directory.mkdir(exist_ok=True)
    backend = Path(__file__).with_name("fixtures") / "installed-sidecar-backend.py"
    archive = make_source_archive(fixture_directory, backend)
    fixtures = {
        "source": Fixture(NAME, VERSION, MODULE, archive, sha256(archive), True),
        "registry-source": Fixture(
            NAME, VERSION, MODULE, archive, sha256(archive), True, registry=True
        ),
        "wheel": Fixture(
            WHEEL_NAME, WHEEL_VERSION, WHEEL_NAME, args.wheel, WHEEL_SHA256, False
        ),
    }
    inspections = (
        "none",
        "opt-out",
        "cache-missing",
        "build-missing",
        "cache-syntax",
        "build-syntax",
        "cache-data",
        "build-data",
        "both-syntax",
        "cache-io",
        "build-io",
    )
    known_cases = {f"inspect-{mutation}" for mutation in inspections}
    known_cases.update({"required-core-missing", "required-core-invalid"})
    known_cases.update(case.name for case in FRESHNESS_CASES)
    if unknown := set(args.case).difference(known_cases):
        parser.error(f"unknown cases: {', '.join(sorted(unknown))}")
    report = {
        "schema": "uv-installed-sidecar-qualification-v1",
        "harness_sha256": sha256(Path(__file__)),
        "backend_sha256": sha256(backend),
        "platform": platform.platform(),
        "binary": {
            "path": str(args.uv),
            "sha256": sha256(args.uv),
            "version": subprocess.check_output(
                [str(args.uv), "--version"], text=True
            ).strip(),
        },
        "python": {
            "path": str(args.python),
            "sha256": sha256(args.python),
            "version": subprocess.check_output(
                [str(args.python), "--version"], text=True
            ).strip(),
        },
        "fixtures": {
            name: {"path": str(fixture.archive), "sha256": fixture.digest}
            for name, fixture in fixtures.items()
        },
        "expected_policy": args.expect,
        "measurement_scope": "Real installed distributions, diagnostics, direct-URL and registry source build settings, and artifact hash enforcement; no timing comparison.",
        "cases": [],
    }
    try:
        qualification = Qualification(args, report, fixtures)
        for mutation in inspections:
            qualification.inspection(mutation)
        for mutation in ("core-missing", "core-invalid"):
            qualification.required_metadata(mutation)
        for case in FRESHNESS_CASES:
            qualification.freshness(case)
        for fixture in fixtures.values():
            if sha256(fixture.archive) != fixture.digest:
                raise AssertionError(
                    f"Qualification modified an input artifact: {fixture.archive}"
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
