#!/usr/bin/env python3
"""Replay uv release-age policies against a frozen local Simple API corpus."""

from __future__ import annotations

import argparse
import hashlib
import io
import itertools
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import zipfile
from datetime import datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

NAME = r"[a-z0-9]+(?:-[a-z0-9]+)*"
VERSION_COMPONENT = r"(?:0|[1-9][0-9]*)"
VERSION = rf"{VERSION_COMPONENT}(?:\.{VERSION_COMPONENT})*"
BOUND = (
    rf"(?:(?:==|!=) *{VERSION}(?:\.\*)?"
    rf"|~= *{VERSION}\.{VERSION_COMPONENT}"
    rf"|(?:===|<=|>=|<|>) *{VERSION})"
)
REQUIREMENT = re.compile(rf"(?P<name>{NAME})(?: *{BOUND}(?: *, *{BOUND})*)?")


def validate_corpus(corpus: dict) -> None:
    """Keep requirements inside the frozen index and the supported simple grammar."""
    for release in corpus["releases"]:
        if not re.fullmatch(NAME, release["name"]):
            raise ValueError(f"Expected a normalized package name: {release['name']!r}")
        if not re.fullmatch(VERSION, release["version"]):
            raise ValueError(
                f"Expected a dotted numeric version: {release['version']!r}"
            )
        for requirement in release.get("requires_dist", []):
            if not REQUIREMENT.fullmatch(requirement):
                raise ValueError(f"Unsupported dependency requirement: {requirement!r}")
    for scenario in corpus["scenarios"]:
        roots = set()
        if not scenario["requirements"]:
            raise ValueError("Each scenario must have at least one requirement")
        for requirement in scenario["requirements"]:
            match = REQUIREMENT.fullmatch(requirement)
            if match is None:
                raise ValueError(f"Unsupported root requirement: {requirement!r}")
            roots.add(match.group("name"))
        if set(scenario["roots"]) != roots:
            raise ValueError(
                "Scenario roots must match the normalized requirement names"
            )


def release_files(release: dict) -> list[dict]:
    files = release.get("files")
    if files is None:
        return [{"tag": "py3-none-any", "upload_time": release.get("upload_time")}]
    if not isinstance(files, list) or not files:
        raise ValueError("Each release must contain at least one file")
    return files


def assets_for(corpus: dict) -> dict[str, tuple[str, bytes]]:
    validate_corpus(corpus)
    assets = {}
    projects: dict[str, list] = {}
    seen = set()
    as_of = datetime.fromisoformat(corpus["as_of"])
    if as_of.utcoffset() is None:
        raise ValueError("as_of must include a timezone")
    for release in corpus["releases"]:
        name, version = release["name"], release["version"]
        components = [int(value) for value in version.split(".")]
        while len(components) > 1 and components[-1] == 0:
            components.pop()
        identity = (name, tuple(components))
        if identity in seen:
            raise ValueError(f"Duplicate release: {name}=={version}")
        seen.add(identity)
        normalized = name.replace("-", "_")
        metadata = f"Metadata-Version: 2.2\nName: {name}\nVersion: {version}\n"
        metadata += "".join(
            f"Requires-Dist: {value}\n" for value in release.get("requires_dist", [])
        )
        metadata = metadata.encode()
        tags = set()
        for specification in release_files(release):
            tag = specification["tag"]
            if not re.fullmatch(
                r"[a-z0-9_]+(?:\.[a-z0-9_]+)*-[a-z0-9_]+(?:\.[a-z0-9_]+)*-[a-z0-9_]+(?:\.[a-z0-9_]+)*",
                tag,
            ):
                raise ValueError(f"Unsupported wheel tag: {tag!r}")
            if tag in tags:
                raise ValueError(f"Duplicate wheel tag: {name}=={version}: {tag}")
            tags.add(tag)
            upload_time = specification.get("upload_time")
            if upload_time is not None:
                timestamp = datetime.fromisoformat(upload_time)
                if timestamp.utcoffset() is None or timestamp > as_of:
                    raise ValueError(
                        f"Invalid frozen file upload time: {upload_time!r}"
                    )
            filename = f"{normalized}-{version}-{tag}.whl"
            wheel_tags = "".join(
                f"Tag: {python}-{abi}-{platform}\n"
                for python, abi, platform in itertools.product(
                    *(component.split(".") for component in tag.split("-"))
                )
            )
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as wheel:
                prefix = f"{normalized}-{version}.dist-info/"
                for entry, content in {
                    "METADATA": metadata,
                    "WHEEL": f"Wheel-Version: 1.0\nRoot-Is-Purelib: true\n{wheel_tags}".encode(),
                    "RECORD": b"",
                }.items():
                    wheel.writestr(zipfile.ZipInfo(prefix + entry), content)
            data = buffer.getvalue()
            path = f"/files/{filename}"
            assets[path] = ("application/octet-stream", data)
            assets[path + ".metadata"] = ("text/plain", metadata)
            file = {
                "filename": filename,
                "url": path,
                "size": len(data),
                "hashes": {"sha256": hashlib.sha256(data).hexdigest()},
                "core-metadata": {"sha256": hashlib.sha256(metadata).hexdigest()},
            }
            if upload_time is not None:
                file["upload-time"] = upload_time
            projects.setdefault(name, []).append(file)
    for name, files in projects.items():
        data = {"meta": {"api-version": "1.0"}, "name": name, "files": files}
        assets[f"/simple/{name}/"] = (
            "application/vnd.pypi.simple.v1+json",
            json.dumps(data).encode(),
        )
    return assets


def replay(
    uv: str, corpus: dict, days: list[int], workspace: Path, check: bool
) -> dict:
    assets = assets_for(corpus)

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            asset = assets.get(urlsplit(self.path).path)
            if asset is None:
                self.send_error(404)
                return
            content_type, content = asset
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    index = f"http://127.0.0.1:{server.server_port}/simple"
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    for variable in ("NO_PROXY", "no_proxy"):
        environment[variable] = ",".join(
            filter(
                None, [environment.get(variable, ""), "127.0.0.1", "localhost", "::1"]
            )
        )
    observations = []
    labels = {
        (release["name"], release["version"]): release.get("labels", [])
        for release in corpus["releases"]
    }
    try:
        for number, scenario in enumerate(corpus["scenarios"]):
            requirements = workspace / f"scenario-{number}.in"
            requirements.write_text("\n".join(scenario["requirements"]) + "\n")
            baseline = None
            for age in days:
                cutoff = datetime.fromisoformat(corpus["as_of"]) - timedelta(days=age)
                command = [
                    uv,
                    "--no-config",
                    "pip",
                    "compile",
                    str(requirements),
                    "--index-url",
                    index,
                    "--python",
                    sys.executable,
                    "--no-build",
                    "--cache-dir",
                    str(workspace / f"cache-{number}-{age}"),
                    "--format",
                    "pylock.toml",
                ]
                if age:
                    command.extend(["--exclude-newer", cutoff.isoformat()])
                started = time.perf_counter()
                process = subprocess.run(
                    command,
                    env=environment,
                    text=True,
                    capture_output=True,
                    check=False,
                    timeout=60,
                )
                selected = None
                selected_files = {}
                if process.returncode == 0:
                    packages = tomllib.loads(process.stdout)["packages"]
                    selected = {
                        package["name"]: package["version"] for package in packages
                    }
                    selected_files = {
                        package["name"]: sorted(
                            Path(urlsplit(wheel["url"]).path).name
                            for wheel in package.get("wheels", [])
                        )
                        for package in packages
                    }
                elif process.returncode != 1 or (
                    "No solution found when resolving dependencies"
                    not in process.stderr
                ):
                    raise RuntimeError(
                        f"uv failed outside dependency resolution: {process.stderr}"
                    )
                if age == 0:
                    baseline = selected
                selected_labels = [
                    {
                        "name": name,
                        "version": version,
                        "scope": "root" if name in scenario["roots"] else "transitive",
                        "labels": labels[(name, version)],
                    }
                    for name, version in (selected or {}).items()
                    if labels[(name, version)]
                ]
                observation = {
                    "scenario": scenario["name"],
                    "minimum_age_days": age,
                    "cutoff": cutoff.isoformat() if age else None,
                    "returncode": process.returncode,
                    "selected": selected,
                    "selected_files": selected_files,
                    "versions_changed_from_baseline": selected != baseline,
                    "selected_labels": selected_labels,
                    "seconds": time.perf_counter() - started,
                    "stderr": process.stderr,
                }
                observations.append(observation)
                if check and selected != scenario["expected"][str(age)]:
                    raise AssertionError(json.dumps(observation, indent=2))
                if (
                    check
                    and "expected_files" in scenario
                    and selected_files != scenario["expected_files"][str(age)]
                ):
                    raise AssertionError(json.dumps(observation, indent=2))
    finally:
        server.shutdown()
        thread.join()
        server.server_close()
    with open(uv, "rb") as binary:
        uv_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    return {
        "uv_sha256": uv_sha256,
        "uv_executable": str(Path(uv).resolve()),
        "uv": subprocess.check_output([uv, "--version"], text=True).strip(),
        "python": sys.version,
        "as_of": corpus["as_of"],
        "description": corpus["description"],
        "releases": len(corpus["releases"]),
        "files": sum(len(release_files(release)) for release in corpus["releases"]),
        "missing_upload_times": sum(
            file.get("upload_time") is None
            for release in corpus["releases"]
            for file in release_files(release)
        ),
        "observations": observations,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", required=True)
    parser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).with_name("release_age_fixture.json"),
    )
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--days", nargs="+", type=int, default=[0, 1, 2, 7, 14])
    parser.add_argument(
        "--check", action="store_true", help="Compare against corpus expectations"
    )
    args = parser.parse_args()
    if (
        any(day < 0 for day in args.days)
        or args.days[0] != 0
        or len(set(args.days)) != len(args.days)
    ):
        parser.error("--days must start with 0 and contain unique nonnegative ages")
    uv = shutil.which(args.uv)
    if uv is None:
        parser.error(f"uv executable not found: {args.uv}")
    corpus_bytes = args.corpus.read_bytes()
    corpus = json.loads(corpus_bytes)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="release-age-", dir=args.work_dir
    ) as directory:
        report = replay(
            str(Path(uv).resolve()),
            corpus,
            args.days,
            Path(directory).resolve(),
            args.check,
        )
        report["corpus_sha256"] = hashlib.sha256(corpus_bytes).hexdigest()
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
