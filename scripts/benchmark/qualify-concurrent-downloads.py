"""Check remote-wheel cache coordination against immutable loopback fixtures."""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Self
from urllib.parse import quote

FILENAMES = (
    "click-8.4.2-py3-none-any.whl",
    "flask-3.1.2-py3-none-any.whl",
    "jupyterlab-4.4.7-py3-none-any.whl",
    "sympy-1.14.0-py3-none-any.whl",
)
SCENARIOS = (
    "independent",
    "shared",
    "metadata-pep658",
    "metadata-range",
    "interrupted",
)


def sha256(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


@dataclass(frozen=True)
class Wheel:
    path: Path
    name: str
    version: str
    digest: str
    metadata_path: str
    metadata: bytes

    def url(self, server: FixtureServer) -> str:
        return f"{server.url}/files/{quote(self.path.name)}#sha256={self.digest}"

    def count_key(self, kind: str) -> str:
        return f"GET /files/{self.path.name} [{kind}]"

    def verify(self, target: Path, *, all_files: bool = False) -> None:
        if (target / self.metadata_path).read_bytes() != self.metadata:
            raise AssertionError(f"Installed metadata differs for {self.path.name}")
        if not all_files:
            return
        record_path = self.metadata_path.removesuffix("METADATA") + "RECORD"
        with zipfile.ZipFile(self.path) as archive:
            for member in archive.infolist():
                if member.is_dir() or member.filename == record_path:
                    continue
                # Scheme-specific files and generated RECORD entries have installer-defined paths.
                if member.filename.split("/", 1)[0].endswith(".data"):
                    continue
                if (target / member.filename).read_bytes() != archive.read(member):
                    raise AssertionError(
                        f"Installed wheel member differs: {member.filename}"
                    )


def load_wheels(directory: Path, manifest: Path) -> tuple[list[dict], list[Wheel]]:
    entries = {item["filename"]: item for item in json.loads(manifest.read_text())}
    selected = []
    wheels = []
    for filename in FILENAMES:
        item = entries[filename]
        path = directory / filename
        if not path.is_file() or sha256(path) != item["sha256"]:
            raise ValueError(
                f"Missing or invalid fixture {path}; run prepare-fixtures.py --fixture {filename}"
            )
        with zipfile.ZipFile(path) as archive:
            metadata_paths = [
                name
                for name in archive.namelist()
                if name.endswith(".dist-info/METADATA")
            ]
            if len(metadata_paths) != 1:
                raise ValueError(f"Expected one METADATA file in {filename}")
            metadata_path = metadata_paths[0]
            metadata = archive.read(metadata_path)
        headers = email.parser.BytesParser().parsebytes(metadata, headersonly=True)
        selected.append(item)
        wheels.append(
            Wheel(
                path,
                headers["Name"],
                headers["Version"],
                item["sha256"],
                metadata_path,
                metadata,
            )
        )
    return selected, wheels


class FixtureServer:
    def __init__(
        self,
        python: Path,
        directory: Path,
        manifest: Path,
        delay_ms: float,
        body_delay_ms: float = 0,
    ) -> None:
        self.process = subprocess.Popen(
            [
                str(python),
                str(Path(__file__).with_name("serve-fixtures.py")),
                "--directory",
                str(directory),
                "--manifest",
                str(manifest),
                "--delay-ms",
                str(delay_ms),
                "--body-delay-ms",
                str(body_delay_ms),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert self.process.stdout is not None
        self.url = self.process.stdout.readline().strip()
        if not self.url.startswith("http://127.0.0.1:"):
            if self.process.poll() is None:
                self.process.terminate()
            _, stderr = self.process.communicate(timeout=5)
            raise RuntimeError(f"Fixture server failed to start: {stderr}")
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.process.terminate()
        try:
            _, stderr = self.process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            _, stderr = self.process.communicate()
        if stderr:
            print(stderr, file=sys.stderr, end="")

    def request(self, path: str, *, method: str = "GET") -> dict[str, int]:
        request = urllib.request.Request(self.url + path, method=method)
        with self.opener.open(request, timeout=5) as response:
            return json.load(response)

    def stats(self, *, idle: bool = True) -> dict[str, int]:
        deadline = time.monotonic() + 10
        while True:
            counts = self.request("/stats")
            if not idle or not any(
                value for key, value in counts.items() if key.endswith(" [active]")
            ):
                return counts
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Fixture requests did not finish: {counts}")
            time.sleep(0.01)

    def reset(self) -> None:
        self.stats()
        self.request("/reset", method="POST")


class Qualification:
    def __init__(self, args: argparse.Namespace, directory: Path) -> None:
        self.args = args
        self.directory = directory
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
            }
        }
        self.environment.update(
            UV_PYTHON_DOWNLOADS="never",
            UV_CONCURRENT_BUILDS="1",
            UV_CONCURRENT_DOWNLOADS=str(args.downloads),
            NO_PROXY="127.0.0.1,localhost",
            no_proxy="127.0.0.1,localhost",
        )

    def command(
        self,
        cache: Path,
        target: Path,
        requirements: list[str],
        *,
        index: str | None = None,
        dry_run: bool = False,
        offline: bool = False,
    ) -> list[str]:
        command = [
            str(self.args.uv),
            "--no-config",
            "--no-progress",
            "--cache-dir",
            str(cache),
        ]
        for feature in self.args.preview_feature:
            command.extend(["--preview-features", feature])
        if offline:
            command.append("--offline")
        command.extend(
            [
                "pip",
                "install",
                "--python",
                str(self.args.python),
                "--link-mode",
                "hardlink",
                "--target",
                str(target),
            ]
        )
        if dry_run:
            # Click's sole dependency is Windows-only. Resolving for Linux reads its metadata
            # without introducing another artifact into the replay fixture set.
            command.extend(["--dry-run", "--python-platform", "linux"])
        else:
            command.append("--no-deps")
        if index is None:
            command.extend(["--no-index", "--require-hashes"])
        else:
            command.extend(["--index-url", index])
        command.extend(requirements)
        if self.args.fd_limit is not None:
            command = [
                "sh",
                "-c",
                'ulimit -S -n "$1" && ulimit -H -n "$1" && shift && exec "$@"',
                "sh",
                str(self.args.fd_limit),
                *command,
            ]
        return command

    def spawn(self, command: list[str]) -> subprocess.Popen[bytes]:
        return subprocess.Popen(
            command,
            env=self.environment,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

    def run(self, commands: list[list[str]]) -> float:
        started = time.perf_counter()
        children = []
        try:
            for command in commands:
                children.append(self.spawn(command))
            for child in children:
                _, stderr = child.communicate(timeout=self.args.timeout)
                if child.returncode != 0:
                    raise RuntimeError(
                        f"uv exited with {child.returncode}: {stderr.decode(errors='replace')}"
                    )
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.communicate()
        return time.perf_counter() - started

    def offline_install(
        self, server: FixtureServer, cache: Path, target: Path, wheels: list[Wheel]
    ) -> None:
        before = server.stats()
        self.run(
            [
                self.command(
                    cache, target, [wheel.url(server) for wheel in wheels], offline=True
                )
            ]
        )
        after = server.stats()
        if after != before:
            raise AssertionError(
                f"Offline install sent HTTP requests: {before} -> {after}"
            )
        for wheel in wheels:
            wheel.verify(target, all_files=True)

    def independent(self, server: FixtureServer, wheels: list[Wheel]) -> dict:
        directory = self.directory / "independent"
        cache = directory / "cache"
        target = directory / "installed"
        server.reset()
        elapsed = self.run(
            [self.command(cache, target, [wheel.url(server) for wheel in wheels])]
        )
        counts = server.stats()
        for wheel in wheels:
            if counts.get(wheel.count_key("body"), 0) != 1:
                raise AssertionError(
                    f"Expected one full body for {wheel.path.name}: {counts}"
                )
            wheel.verify(target)
        if counts.get("GET file bodies [max-active]", 0) < 2:
            raise AssertionError(f"Independent wheel bodies were serialized: {counts}")
        self.offline_install(server, cache, directory / "offline", wheels)
        return {"name": "independent", "elapsed_seconds": elapsed, "counts": counts}

    def shared(self, server: FixtureServer, wheel: Wheel) -> dict:
        directory = self.directory / "shared"
        cache = directory / "cache"
        targets = [
            directory / f"installed-{index}" for index in range(self.args.processes)
        ]
        server.reset()
        elapsed = self.run(
            [self.command(cache, target, [wheel.url(server)]) for target in targets]
        )
        counts = server.stats()
        if counts.get(wheel.count_key("body"), 0) != 1:
            raise AssertionError(
                f"Concurrent installs downloaded duplicate full bodies: {counts}"
            )
        if counts.get(wheel.count_key("body-bytes"), 0) != wheel.path.stat().st_size:
            raise AssertionError(f"Unexpected full-body byte count: {counts}")
        for target in targets:
            wheel.verify(target, all_files=True)
        self.offline_install(server, cache, directory / "offline", [wheel])
        return {"name": "shared", "elapsed_seconds": elapsed, "counts": counts}

    def metadata(self, server: FixtureServer, wheel: Wheel, *, index: bool) -> dict:
        name = "metadata-pep658" if index else "metadata-range"
        directory = self.directory / name
        requirements = (
            [f"{wheel.name}=={wheel.version}"] if index else [wheel.url(server)]
        )
        index_url = server.url + "/simple" if index else None
        key = (
            f"GET /files/{wheel.path.name}.metadata"
            if index
            else wheel.count_key("range")
        )
        observations = {}
        for label, processes in [("single", 1), ("concurrent", self.args.processes)]:
            cache = directory / label / "cache"
            server.reset()
            elapsed = self.run(
                [
                    self.command(
                        cache,
                        directory / label / f"installed-{number}",
                        requirements,
                        index=index_url,
                        dry_run=True,
                    )
                    for number in range(processes)
                ]
            )
            counts = server.stats()
            if counts.get(key, 0) == 0 or counts.get(wheel.count_key("body"), 0) != 0:
                raise AssertionError(
                    f"Expected metadata-only requests for {name}: {counts}"
                )
            observations[label] = {"elapsed_seconds": elapsed, "counts": counts}
        if self.args.expect_coalesced_metadata:
            single = observations["single"]["counts"].get(key, 0)
            concurrent = observations["concurrent"]["counts"].get(key, 0)
            if concurrent != single:
                raise AssertionError(
                    f"Metadata requests were not coalesced: {observations}"
                )
        server.reset()
        warm_elapsed = [
            self.run(
                [
                    self.command(
                        directory / "concurrent" / "cache",
                        directory / f"warm-{number}",
                        requirements,
                        index=index_url,
                        dry_run=True,
                    )
                ]
            )
            for number in range(self.args.warm_iterations)
        ]
        warm_counts = server.stats()
        if warm_counts.get(key, 0) or warm_counts.get(wheel.count_key("body"), 0):
            raise AssertionError(
                f"Fresh cached metadata made another artifact request: {warm_counts}"
            )
        return {
            "name": name,
            **observations,
            "warm_elapsed_seconds": warm_elapsed,
            "warm_counts": warm_counts,
        }

    def interrupted(self, server: FixtureServer, wheel: Wheel) -> dict:
        directory = self.directory / "interrupted"
        cache = directory / "cache"
        server.reset()
        leader = self.spawn(
            self.command(cache, directory / "leader", [wheel.url(server)])
        )
        try:
            deadline = time.monotonic() + self.args.timeout
            while True:
                counts = server.stats(idle=False)
                sent = counts.get(wheel.count_key("body-bytes"), 0)
                if 0 < sent < wheel.path.stat().st_size:
                    break
                if leader.poll() is not None or time.monotonic() >= deadline:
                    raise AssertionError(
                        f"No partially written wheel body observed: {counts}"
                    )
                time.sleep(0.005)
            leader.terminate()
            leader.communicate(timeout=10)
            if leader.returncode == 0:
                raise AssertionError(
                    "Interrupted install unexpectedly finished successfully"
                )
        finally:
            if leader.poll() is None:
                leader.kill()
            leader.communicate()
        interrupted_counts = server.stats()
        if (
            not 0
            < interrupted_counts.get(wheel.count_key("body-bytes"), 0)
            < wheel.path.stat().st_size
        ):
            raise AssertionError(
                f"The leader was not interrupted during its body: {interrupted_counts}"
            )
        targets = [directory / f"recovered-{number}" for number in range(2)]
        elapsed = self.run(
            [self.command(cache, target, [wheel.url(server)]) for target in targets]
        )
        for target in targets:
            wheel.verify(target, all_files=True)
        self.offline_install(server, cache, directory / "offline", [wheel])
        return {
            "name": "interrupted",
            "leader_exit_code": leader.returncode,
            "interrupted_counts": interrupted_counts,
            "recovery_elapsed_seconds": elapsed,
            "counts": server.stats(),
        }


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument(
        "--directory", type=Path, default=root / ".cache/bench-fixtures"
    )
    parser.add_argument(
        "--manifest", type=Path, default=Path(__file__).with_name("fixtures.json")
    )
    parser.add_argument(
        "--work-directory", type=Path, default=root / ".cache/bench-qualification"
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--processes", type=int, default=4)
    parser.add_argument("--downloads", type=int, default=4)
    parser.add_argument("--fd-limit", type=int)
    parser.add_argument("--delay-ms", type=float, default=100)
    parser.add_argument("--body-delay-ms", type=float, default=25)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--warm-iterations", type=int, default=5)
    parser.add_argument("--preview-feature", action="append", default=[])
    parser.add_argument("--expect-coalesced-metadata", action="store_true")
    parser.add_argument("--scenario", choices=SCENARIOS, action="append", default=[])
    args = parser.parse_args()
    if args.processes < 2 or args.downloads < 1 or args.warm_iterations < 1:
        parser.error(
            "at least two processes, one download, and one warm iteration are required"
        )
    if args.delay_ms <= 0 or args.body_delay_ms <= 0 or args.timeout <= 0:
        parser.error("delays and timeout must be positive")
    if args.fd_limit is not None and (os.name != "posix" or args.fd_limit < 16):
        parser.error("--fd-limit requires POSIX and a limit of at least 16")
    args.uv = args.uv.resolve(strict=True)
    args.python = args.python.resolve(strict=True)
    args.directory = args.directory.resolve(strict=True)
    args.work_directory.mkdir(parents=True, exist_ok=True)
    args.work_directory = args.work_directory.resolve(strict=True)
    selected, wheels = load_wheels(args.directory, args.manifest)
    scenarios = [
        name for name in SCENARIOS if not args.scenario or name in args.scenario
    ]
    results = []
    report = {
        "success": False,
        "platform": sys.platform,
        "harness_sha256": sha256(Path(__file__)),
        "fixture_server_sha256": sha256(Path(__file__).with_name("serve-fixtures.py")),
        "uv": str(args.uv),
        "uv_sha256": sha256(args.uv),
        "uv_version": subprocess.check_output(
            [args.uv, "--version"], text=True
        ).strip(),
        "python": str(args.python),
        "python_version": subprocess.check_output(
            [args.python, "-I", "--version"], text=True
        ).strip(),
        "processes": args.processes,
        "concurrent_builds": 1,
        "concurrent_downloads": args.downloads,
        "fd_limit": args.fd_limit,
        "request_delay_ms": args.delay_ms,
        "body_delay_ms": args.body_delay_ms,
        "warm_iterations": args.warm_iterations,
        "preview_features": args.preview_feature,
        "scenarios": scenarios,
        "expect_coalesced_metadata": args.expect_coalesced_metadata,
        "fixtures": {wheel.path.name: wheel.digest for wheel in wheels},
        "results": results,
    }
    try:
        with tempfile.TemporaryDirectory(
            dir=args.work_directory, prefix="downloads-"
        ) as temporary:
            directory = Path(temporary)
            manifest = directory / "fixtures.json"
            manifest.write_text(json.dumps(selected))
            qualification = Qualification(args, directory)
            if any(name != "interrupted" for name in scenarios):
                with FixtureServer(
                    args.python, args.directory, manifest, args.delay_ms
                ) as server:
                    if "independent" in scenarios:
                        results.append(qualification.independent(server, wheels))
                    if "shared" in scenarios:
                        results.append(qualification.shared(server, wheels[0]))
                    if "metadata-pep658" in scenarios:
                        results.append(
                            qualification.metadata(server, wheels[0], index=True)
                        )
                    if "metadata-range" in scenarios:
                        results.append(
                            qualification.metadata(server, wheels[0], index=False)
                        )
            if "interrupted" in scenarios:
                with FixtureServer(
                    args.python,
                    args.directory,
                    manifest,
                    args.delay_ms,
                    args.body_delay_ms,
                ) as server:
                    results.append(qualification.interrupted(server, wheels[2]))
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
