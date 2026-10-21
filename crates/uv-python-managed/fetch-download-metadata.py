#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "httpx < 1",
# ]
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""
Fetch Python version download metadata.

Generates the provider catalogs in `download-metadata/`.

Usage:

    uv run --locked crates/uv-python-managed/fetch-download-metadata.py

Acknowledgements:

    Derived from https://github.com/mitsuhiko/rye/tree/f9822267a7f00332d15be8551f89a212e7bc9017
    Originally authored by Armin Ronacher under the MIT license
"""
# https://github.com/mitsuhiko/rye/raw/f9822267a7f00332d15be8551f89a212e7bc9017/LICENSE
#
# MIT License
#
# Copyright (c) 2023, Armin Ronacher
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.

import abc
import argparse
import asyncio
import itertools
import json
import logging
import os
import re
from collections.abc import Generator, Iterable
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any, ClassVar, NamedTuple, Self
from urllib.parse import unquote

import httpx

SELF_DIR = Path(__file__).parent
VERSIONS_DIR = SELF_DIR / "download-metadata"
logger = logging.getLogger(__name__)

# The date at which the default CPython musl builds became dynamically linked
# instead of statically.
CPYTHON_MUSL_STATIC_RELEASE_END = 20250311

ARCHIVE_FLAVORS = frozenset({"full", "install_only", "install_only_stripped"})
LEGACY_ARCHIVE_FLAVORS = frozenset({"shared-pgo", "shared-noopt", "static-noopt"})
COMPOUND_VARIANTS = {
    "shared-freethreaded": ("shared", "freethreaded"),
    "shared-noopt": ("shared", "noopt"),
    "shared-pgo": ("shared", "pgo"),
    "static-noopt": ("static", "noopt"),
}
DEFAULT_FLAVOR_PREFERENCES = (
    "install_only_stripped",
    "install_only",
    "shared-pgo",
    "shared-noopt",
    "static-noopt",
)


def batched(iterable: Iterable, n: int) -> Generator[tuple, None, None]:
    """Batch data into tuples of length n. The last batch may be shorter."""
    # batched('ABCDEFG', 3) --> ABC DEF G
    if n < 1:
        raise ValueError("n must be at least one")
    it = iter(iterable)
    while batch := tuple(itertools.islice(it, n)):
        yield batch


@dataclass(frozen=True)
class Arch:
    # The architecture family, e.g. "x86_64", "aarch64".
    family: str
    # The architecture variant, e.g., "v2" in "x86_64_v2"
    variant: str | None = None

    def key(self) -> str:
        return str(self)

    def __str__(self) -> str:
        return (self.family + "_" + self.variant) if self.variant else self.family

    def __gt__(self, other) -> bool:
        return (self.family, self.variant or "") > (other.family, other.variant or "")

    def __lt__(self, other) -> bool:
        return (self.family, self.variant or "") < (other.family, other.variant or "")


type PlatformTripleKey = tuple[str, str, str]


class PlatformTriple(NamedTuple):
    # The operating system, e.g. "linux", "macos", "windows".
    platform: str
    # The architecture, e.g. "x86_64", "aarch64".
    arch: Arch
    # The libc implementation, e.g. "gnu", "musl", "none".
    libc: str

    def key(self) -> PlatformTripleKey:
        return (self.platform, self.arch.key(), self.libc)

    def cargo_dist(self) -> str:
        vendor = {"darwin": "apple", "windows": "pc"}.get(self.platform, "unknown")
        triple = f"{self.arch}-{vendor}-{self.platform}"
        if self.platform == "windows":
            return f"{triple}-msvc"
        if self.libc != "none":
            return f"{triple}-{self.libc}"
        return triple


class Version(NamedTuple):
    major: int
    minor: int
    patch: int
    prerelease: str = ""

    @classmethod
    def from_str(cls, version: str) -> Self:
        major, minor, patch = version.split(".", 3)
        prerelease = ""
        for prerelease_kind in ("a", "b", "rc"):
            parts = patch.split(prerelease_kind, 1)
            if len(parts) == 2:
                patch = parts[0]
                prerelease = prerelease_kind + parts[1]
                break

        return cls(int(major), int(minor), int(patch), prerelease)

    def __str__(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}{self.prerelease}"


class ImplementationName(StrEnum):
    CPYTHON = "cpython"
    PYPY = "pypy"
    GRAALPY = "graalpy"


class Variant(StrEnum):
    FREETHREADED = "freethreaded"
    DEBUG = "debug"
    FREETHREADED_DEBUG = "freethreaded+debug"

    @classmethod
    def from_names(cls: type["Variant"], names: tuple[str, ...]) -> "Variant | None":
        if "debug" in names and "freethreaded" in names:
            return cls.FREETHREADED_DEBUG
        elif "debug" in names:
            return cls.DEBUG
        elif "freethreaded" in names:
            return cls.FREETHREADED
        else:
            return None


@dataclass(frozen=True)
class ArtifactVariants:
    names: tuple[str, ...]
    flavor: str | None

    def __post_init__(self) -> None:
        object.__setattr__(self, "names", tuple(sorted(set(self.names) - {""})))

    @classmethod
    def parse(cls, value: str, platform_names: Iterable[str] = ()) -> Self:
        parts = [part for part in value.split("+") if part]
        if parts and parts[-1] in ARCHIVE_FLAVORS:
            flavor = parts.pop()
        elif value in LEGACY_ARCHIVE_FLAVORS:
            flavor = value
        else:
            flavor = None
        names = list(platform_names)
        for part in parts:
            names.extend(COMPOUND_VARIANTS.get(part, (part,)))
        return cls(tuple(names), flavor)


@dataclass
class PythonDownload:
    release: int
    version: Version
    triple: PlatformTriple
    implementation: ImplementationName
    filename: str
    url: str
    build: str
    artifact_variants: ArtifactVariants
    sha256: str | None = None
    artifact: dict[str, Any] | None = None

    @property
    def variant(self) -> Variant | None:
        return Variant.from_names(self.artifact_variants.names)

    def key(self) -> str:
        if self.variant:
            return f"{self.implementation}-{self.version}+{self.variant}-{self.triple.platform}-{self.triple.arch}-{self.triple.libc}"
        else:
            return f"{self.implementation}-{self.version}-{self.triple.platform}-{self.triple.arch}-{self.triple.libc}"


def is_compatible_download(
    download: PythonDownload, *, pbs_release: int | None = None
) -> bool:
    if "static" in download.artifact_variants.names:
        return False
    if {"gil", "freethreaded"}.issubset(download.artifact_variants.names):
        return False
    return not (
        pbs_release is not None
        and pbs_release < CPYTHON_MUSL_STATIC_RELEASE_END
        and download.triple.libc == "musl"
    )


def default_preference(download: PythonDownload) -> tuple[int, int]:
    flavor_name = download.artifact_variants.flavor
    try:
        flavor = (
            DEFAULT_FLAVOR_PREFERENCES.index(flavor_name)
            if flavor_name is not None
            else len(DEFAULT_FLAVOR_PREFERENCES) + 1
        )
    except ValueError:
        flavor = len(DEFAULT_FLAVOR_PREFERENCES) + 1
    names = download.artifact_variants.names
    return flavor, -int("pgo" in names) - int("lto" in names)


class Finder:
    implementation: ImplementationName

    @abc.abstractmethod
    async def find(self) -> list[PythonDownload]:
        raise NotImplementedError


class CPythonFinder(Finder):
    implementation = ImplementationName.CPYTHON

    NDJSON_URL = "https://releases.astral.sh/github/versions/main/v1/python-build-standalone.ndjson"

    # Normalized mappings to match the Rust types
    ARCH_MAP: ClassVar[dict[str, str]] = {
        "ppc64": "powerpc64",
        "ppc64le": "powerpc64le",
    }

    def __init__(self, client: httpx.AsyncClient):
        self.client = client

    async def find(self) -> list[PythonDownload]:
        return await self._fetch_downloads()

    async def _fetch_downloads(self) -> list[PythonDownload]:
        """Fetch all CPython downloads from the NDJSON release index."""
        logger.info("Fetching CPython release index")
        downloads_by_version: dict[Version, list[PythonDownload]] = {}

        async with self.client.stream("GET", self.NDJSON_URL) as response:
            response.raise_for_status()
            async for line in response.aiter_lines():
                line = line.strip()
                if not line:
                    continue

                record = json.loads(line)
                # Parse "3.11.15+20260303" → version="3.11.15", release=20260303
                version_str, _, date_str = record["version"].partition("+")
                version = Version.from_str(version_str)
                release = int(date_str)

                # Catalog order breaks ties between equally preferred artifacts.
                for artifact in record["artifacts"]:
                    download = self._parse_ndjson_artifact(version, release, artifact)
                    if download is None:
                        continue
                    if not is_compatible_download(download, pbs_release=release):
                        continue
                    logger.debug("Found %s (%s)", download.key(), download.filename)
                    downloads_by_version.setdefault(download.version, []).append(
                        download
                    )

        # Collapse CPython variants to a single flavor per triple and variant
        downloads = []
        for version_downloads in downloads_by_version.values():
            selected: dict[
                tuple[PlatformTripleKey, Variant | None],
                tuple[PythonDownload, tuple[int, int]],
            ] = {}
            for download in version_downloads:
                priority = default_preference(download)
                existing = selected.get((download.triple.key(), download.variant))
                if existing:
                    existing_download, existing_priority = existing
                    if priority >= existing_priority:
                        logger.debug(
                            "Skipping %s (%s): lower priority than %s (%s)",
                            download.key(),
                            download.artifact_variants.flavor,
                            existing_download.key(),
                            existing_download.artifact_variants.flavor,
                        )
                        continue
                selected[(download.triple.key(), download.variant)] = (
                    download,
                    priority,
                )

            # Drop the priorities
            downloads.extend([download for download, _ in selected.values()])

        return downloads

    def _parse_ndjson_artifact(
        self, version: Version, release: int, artifact: dict[str, Any]
    ) -> PythonDownload | None:
        """Parse a single NDJSON artifact entry into a PythonDownload."""
        url = artifact["url"]
        sha256 = artifact.get("sha256")
        filename = unquote(url.rsplit("/", maxsplit=1)[-1])

        platform_str = artifact["platform"]
        variant_str = artifact["variant"]

        # Build options can occur in both platform suffixes and the variant field.
        platform_build_options: list[str] = []
        while "-" in platform_str:
            prefix, _, option = platform_str.rpartition("-")
            if option not in {"static", "shared", "debug", "freethreaded"}:
                break
            platform_str = prefix
            platform_build_options.append(option)

        if len(platform_str.split("-")) > 4:
            logger.debug("Skipping %s: unsupported platform suffix", filename)
            return None

        triple = self._normalize_triple(platform_str)
        if triple is None:
            return None

        return PythonDownload(
            release=release,
            version=version,
            triple=triple,
            implementation=self.implementation,
            filename=filename,
            url=url,
            build=str(release),
            artifact_variants=ArtifactVariants.parse(
                variant_str, platform_build_options
            ),
            sha256=sha256,
            artifact=artifact,
        )

    def _normalize_triple(self, triple: str) -> PlatformTriple | None:
        pieces = triple.split("-")
        try:
            arch = self._normalize_arch(pieces[0])
            operating_system = self._normalize_os(pieces[2])
            if pieces[2] == "linux":
                # On linux, the triple has four segments, the last one is the libc
                libc = pieces[3]
            else:
                libc = "none"
        except IndexError:
            logger.debug("Skipping %r: unknown triple", triple)
            return None

        return PlatformTriple(operating_system, arch, libc)

    def _normalize_arch(self, arch: str) -> Arch:
        arch = self.ARCH_MAP.get(arch, arch)
        pieces = arch.split("_")
        family = "_".join(pieces[:2])
        variant = pieces[2] if len(pieces) > 2 else None
        return Arch(family, variant)

    def _normalize_os(self, os: str) -> str:
        return os


class PyPyFinder(Finder):
    implementation = ImplementationName.PYPY

    RELEASE_URL = "https://raw.githubusercontent.com/pypy/pypy/main/pypy/tool/release/versions.json"
    CHECKSUM_URL = (
        "https://raw.githubusercontent.com/pypy/pypy.org/main/pages/checksums.rst"
    )

    _checksum_re = re.compile(
        r"^\s*(?P<checksum>\w{64})\s+(?P<filename>pypy.+)$", re.MULTILINE
    )

    ARCH_MAPPING: ClassVar[dict[str, str]] = {
        "x64": "x86_64",
        "x86": "i686",
        "i686": "i686",
        "aarch64": "aarch64",
        "arm64": "aarch64",
        "s390x": "s390x",
    }

    PLATFORM_MAPPING: ClassVar[dict[str, str]] = {
        "win32": "windows",
        "win64": "windows",
        "linux": "linux",
        "darwin": "darwin",
    }

    def __init__(self, client: httpx.AsyncClient):
        self.client = client

    async def find(self) -> list[PythonDownload]:
        downloads = await self._fetch_downloads()
        await self._fetch_checksums(downloads)
        return downloads

    async def _fetch_downloads(self) -> list[PythonDownload]:
        resp = await self.client.get(self.RELEASE_URL)
        resp.raise_for_status()
        versions = resp.json()

        results = {}
        incomplete_versions = set()
        for version in versions:
            if not version["stable"]:
                continue
            python_version = Version.from_str(version["python_version"])
            if python_version < (3, 7, 0):
                continue
            pypy_version = version["pypy_version"]
            for file in version["files"]:
                # Only a small number of older pypy builds are bz2; we filter
                # them because uv 0.12+ won't support extracting them.
                if file["filename"].endswith(".tar.bz2"):
                    incomplete_versions.add(version["python_version"])
                    continue

                # If we've filtered out a bz2 distribution above, we want to
                # make sure we filter out its other variants as well. Otherwise
                # we'd have a partial state where some patch versions of PyPy
                # are available on Windows but not other platforms (since Windows
                # uses zip, not bz2).
                if version["python_version"] in incomplete_versions:
                    continue

                arch = self._normalize_arch(file["arch"])
                platform = self._normalize_os(file["platform"])
                libc = "gnu" if platform == "linux" else "none"
                download = PythonDownload(
                    release=0,
                    version=python_version,
                    triple=PlatformTriple(
                        platform=platform,
                        arch=arch,
                        libc=libc,
                    ),
                    implementation=self.implementation,
                    filename=file["filename"],
                    url=file["download_url"],
                    build=pypy_version,
                    artifact_variants=ArtifactVariants((), "install_only"),
                )
                # Only keep the latest pypy version of each arch/platform
                if (python_version, arch, platform) not in results:
                    results[(python_version, arch, platform)] = download

        return list(results.values())

    def _normalize_arch(self, arch: str) -> Arch:
        return Arch(self.ARCH_MAPPING.get(arch, arch), None)

    def _normalize_os(self, os: str) -> str:
        return self.PLATFORM_MAPPING.get(os, os)

    async def _fetch_checksums(self, downloads: list[PythonDownload]) -> None:
        logger.info("Fetching PyPy checksums")
        resp = await self.client.get(self.CHECKSUM_URL)
        resp.raise_for_status()
        text = resp.text

        checksums = {}
        for match in self._checksum_re.finditer(text):
            checksums[match.group("filename")] = match.group("checksum")

        for download in downloads:
            download.sha256 = checksums.get(download.filename)


class PyodideFinder(Finder):
    implementation = ImplementationName.CPYTHON

    RELEASE_URL = "https://api.github.com/repos/pyodide/pyodide/releases?per_page=100"
    METADATA_URL = (
        "https://pyodide.github.io/pyodide/api/pyodide-cross-build-environments.json"
    )

    TRIPLE = PlatformTriple(
        platform="emscripten",
        arch=Arch("wasm32"),
        libc="musl",
    )

    def __init__(self, client: httpx.AsyncClient):
        self.client = client

    async def find(self) -> list[PythonDownload]:
        downloads = await self._fetch_downloads()
        await self._fetch_checksums(downloads, n=10)
        return downloads

    async def _fetch_downloads(self) -> list[PythonDownload]:
        meta_resp = await self.client.get(self.METADATA_URL)
        meta_resp.raise_for_status()
        metadata = meta_resp.json()["releases"]

        # Paginate through all pages of releases via the Link header
        releases = []
        release_url: str | None = self.RELEASE_URL
        while release_url is not None:
            resp = await self.client.get(release_url)
            resp.raise_for_status()
            releases.extend(resp.json())
            next_link = resp.links.get("next", {})
            release_url = next_link.get("url")

        results = {}
        for release in releases:
            # Skip prereleases
            # https://github.com/astral-sh/uv/pull/18958#discussion_r3082735525
            if release.get("prerelease"):
                continue

            pyodide_version = release["tag_name"]
            meta = metadata.get(pyodide_version, None)
            if meta is None:
                base_version = f"{pyodide_version.rsplit('.', 1)[0]}.0"
                meta = metadata.get(base_version, None)
            if meta is None:
                continue

            python_version = Version.from_str(meta["python_version"])

            # Find xbuildenv asset
            for asset in release["assets"]:
                if asset["name"] == f"xbuildenv-{pyodide_version}.tar.gz":
                    break
            else:
                # not found: should not happen but just in case
                continue

            url = asset["browser_download_url"]
            download = PythonDownload(
                release=0,
                version=python_version,
                triple=self.TRIPLE,
                implementation=self.implementation,
                filename=asset["name"],
                url=url,
                build=pyodide_version,
                artifact_variants=ArtifactVariants((), "install_only"),
            )

            # Only keep latest Pyodide version of each Python version
            # arch/platform are all the same for Pyodide (wasm32, emscripten)
            if python_version not in results:
                results[python_version] = download

        return list(results.values())

    async def _fetch_checksums(self, downloads: list[PythonDownload], n: int) -> None:
        for idx, batch in enumerate(batched(downloads, n)):
            logger.info("Fetching Pyodide checksums: %d/%d", idx * n, len(downloads))
            checksum_requests = []
            for download in batch:
                url = download.url + ".sha256"
                checksum_requests.append(self.client.get(url))
            for download, resp in zip(
                batch, await asyncio.gather(*checksum_requests), strict=False
            ):
                try:
                    resp.raise_for_status()
                except httpx.HTTPStatusError as e:
                    if e.response.status_code == 404:
                        continue
                    raise
                download.sha256 = resp.text.strip()


class GraalPyFinder(Finder):
    implementation = ImplementationName.GRAALPY

    RELEASE_URL = "https://api.github.com/repos/oracle/graalpython/releases"

    PLATFORM_MAPPING: ClassVar[dict[str, str]] = {
        "windows": "windows",
        "linux": "linux",
        "macos": "darwin",
    }

    ARCH_MAPPING: ClassVar[dict[str, str]] = {
        "amd64": "x86_64",
        "aarch64": "aarch64",
    }

    GRAALPY_VERSION_RE = re.compile(r"-(\d+\.\d+\.\d+)$", re.ASCII)
    CPY_VERSION_RE = re.compile(r"Python (\d+\.\d+(\.\d+)?)", re.ASCII)
    GRAALPY_ASSET_VERSION_RE = re.compile(r"graalpy(\d+\.\d+(\.\d+)?)-", re.ASCII)
    PLATFORM_RE = re.compile(r"(\w+)-(\w+)\.(?:zip|tar\.gz)$", re.ASCII)

    def __init__(self, client: httpx.AsyncClient):
        self.client = client

    async def find(self) -> list[PythonDownload]:
        downloads = await self._fetch_downloads()
        await self._fetch_checksums(downloads, n=10)
        return downloads

    async def _fetch_downloads(self) -> list[PythonDownload]:
        # This will only download the first page, i.e., ~30 releases of
        # GraalPy. Since GraalPy releases 6 times a year and has a support
        # window of 2 years this is plenty.
        resp = await self.client.get(self.RELEASE_URL)
        resp.raise_for_status()
        releases = resp.json()

        results = {}
        for release in releases:
            m = self.GRAALPY_VERSION_RE.search(release["tag_name"])
            if not m:
                continue
            graalpy_version = m.group(1)
            for asset in release["assets"]:
                url = asset["browser_download_url"]
                m = self.PLATFORM_RE.search(url)
                if not m:
                    continue
                platform = self._normalize_os(m.group(1))
                arch = self._normalize_arch(m.group(2))
                libc = "gnu" if platform == "linux" else "none"
                sha256 = None
                if digest := asset["digest"]:
                    sha256 = digest.removeprefix("sha256:")
                m = self.GRAALPY_ASSET_VERSION_RE.search(asset["name"])
                if not m:
                    m = self.CPY_VERSION_RE.search(release["body"])
                if not m:
                    continue
                python_version_str = m.group(1)
                if not m.group(2):
                    python_version_str += ".0"
                python_version = Version.from_str(python_version_str)
                download = PythonDownload(
                    release=0,
                    version=python_version,
                    triple=PlatformTriple(
                        platform=platform,
                        arch=arch,
                        libc=libc,
                    ),
                    implementation=self.implementation,
                    filename=asset["name"],
                    url=url,
                    build=graalpy_version,
                    artifact_variants=ArtifactVariants((), "install_only"),
                    sha256=sha256,
                )
                # Only keep the latest GraalPy version of each arch/platform
                key = (python_version, arch, platform)
                if key not in results or graalpy_version > results[key].build:
                    results[key] = download

        return list(results.values())

    def _normalize_arch(self, arch: str) -> Arch:
        return Arch(self.ARCH_MAPPING.get(arch, arch), None)

    def _normalize_os(self, os: str) -> str:
        return self.PLATFORM_MAPPING.get(os, os)

    async def _fetch_checksums(self, downloads: list[PythonDownload], n: int) -> None:
        downloads = list(filter(lambda d: not d.sha256, downloads))
        for idx, batch in enumerate(batched(downloads, n)):
            logger.info("Fetching GraalPy checksums: %d/%d", idx * n, len(downloads))
            checksum_requests = []
            for download in batch:
                url = download.url + ".sha256"
                checksum_requests.append(self.client.get(url))
            for download, resp in zip(
                batch, await asyncio.gather(*checksum_requests), strict=False
            ):
                try:
                    resp.raise_for_status()
                except httpx.HTTPStatusError as e:
                    if e.response.status_code == 404:
                        continue
                    raise
                download.sha256 = resp.text.strip()


def prerelease_sort_key(prerelease: str) -> tuple[int, int]:
    if prerelease.startswith("a"):
        return 0, int(prerelease[1:])
    if prerelease.startswith("b"):
        return 1, int(prerelease[1:])
    if prerelease.startswith("rc"):
        return 2, int(prerelease[2:])
    return 3, 0


def filter_cpython_versions(downloads: list[PythonDownload]) -> list[PythonDownload]:
    """Keep recent CPython patches while retaining all other implementations."""
    groups: dict[
        tuple[int, int, PlatformTripleKey, Variant | None], list[PythonDownload]
    ] = {}
    other_downloads: list[PythonDownload] = []

    for download in downloads:
        if (
            download.implementation != ImplementationName.CPYTHON
            or download.triple.platform == "emscripten"
        ):
            other_downloads.append(download)
            continue

        group = (
            download.version.major,
            download.version.minor,
            download.triple.key(),
            download.variant,
        )
        groups.setdefault(group, []).append(download)

    selected_downloads: list[PythonDownload] = []
    for group_downloads in groups.values():
        stable_downloads = [
            download for download in group_downloads if not download.version.prerelease
        ]
        stable_downloads.sort(key=lambda download: download.version.patch, reverse=True)
        selected_downloads.extend(stable_downloads[:3])

        prerelease_downloads = [
            download for download in group_downloads if download.version.prerelease
        ]

        if prerelease_downloads:
            selected_downloads.append(
                max(
                    prerelease_downloads,
                    key=lambda download: (
                        download.version.patch,
                        prerelease_sort_key(download.version.prerelease),
                    ),
                )
            )

    return selected_downloads + other_downloads


def render(downloads: list[PythonDownload], *, prune: bool = False) -> None:
    """Render the selected release artifacts in each provider's NDJSON catalog."""
    downloads = [download for download in downloads if is_compatible_download(download)]
    if prune:
        downloads = filter_cpython_versions(downloads)

    def variant_sort_key(variant: Variant | None) -> int:
        if variant is None:
            return 0
        match variant:
            case Variant.FREETHREADED:
                return 1
            case Variant.FREETHREADED_DEBUG:
                return 2
            case Variant.DEBUG:
                return 3
        raise ValueError(f"Missing sort key implementation for variant: {variant}")

    def sort_key(download: PythonDownload) -> tuple:
        # Sort by implementation, version (latest first), and then by triple.
        impl_order = [
            ImplementationName.CPYTHON,
            ImplementationName.PYPY,
            ImplementationName.GRAALPY,
        ]
        prerelease = prerelease_sort_key(download.version.prerelease)
        return (
            impl_order.index(download.implementation),
            -download.version.major,
            -download.version.minor,
            -download.version.patch,
            -prerelease[0],
            -prerelease[1],
            variant_sort_key(download.variant),
            download.triple,
        )

    downloads.sort(key=sort_key)

    catalogs: dict[str, dict[str, list[dict[str, Any]]]] = {
        "python-build-standalone": {},
        "pypy": {},
        "graalpy": {},
        "pyodide": {},
    }
    for download in downloads:
        key = download.key()
        if (download.version.major, download.version.minor) < (3, 8):
            logger.info(
                "Skipping unsupported version %s%s",
                key,
                f" ({download.artifact_variants.flavor})"
                if download.artifact_variants.flavor
                else "",
            )
            continue

        logger.info(
            "Selected %s%s",
            key,
            f" ({download.artifact_variants.flavor})"
            if download.artifact_variants.flavor
            else "",
        )
        if download.triple.platform == "emscripten":
            provider = "pyodide"
        elif download.implementation == ImplementationName.CPYTHON:
            provider = "python-build-standalone"
        else:
            provider = download.implementation

        artifact = download.artifact
        if artifact is None:
            flavor = download.artifact_variants.flavor
            if flavor is None or flavor not in ARCHIVE_FLAVORS:
                raise ValueError("Provider adapters must specify an archive flavor")
            archive_format = next(
                extension
                for extension in ("tar.gz", "tar.zst", "tar.bz2", "zip")
                if download.filename.endswith(f".{extension}")
            )
            artifact = {
                "url": download.url,
                "platform": download.triple.cargo_dist(),
                "variant": "+".join((*download.artifact_variants.names, flavor)),
                "archive_format": archive_format,
                "sha256": download.sha256,
            }
        version = f"{download.version}+{download.build}"
        catalogs[provider].setdefault(version, []).append(artifact)

    VERSIONS_DIR.mkdir(parents=True, exist_ok=True)
    for provider, releases in catalogs.items():
        records = (
            {"version": version, "artifacts": artifacts}
            for version, artifacts in releases.items()
        )
        (VERSIONS_DIR / f"{provider}.ndjson").write_text(
            "".join(
                json.dumps(record, separators=(",", ":")) + "\n" for record in records
            ),
            newline="\n",
        )


async def find(*, prune: bool = False) -> None:
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        logger.warning(
            "`GITHUB_TOKEN` env var not found, you may hit rate limits for GitHub API requests."
        )

    headers = {
        "X-GitHub-Api-Version": "2022-11-28",
        "Accept-Encoding": "gzip, deflate",
    }
    if token:
        headers["Authorization"] = "Bearer " + token
    client = httpx.AsyncClient(follow_redirects=True, headers=headers, timeout=60)

    finders = [
        CPythonFinder(client),
        PyPyFinder(client),
        GraalPyFinder(client),
        PyodideFinder(client),
    ]
    downloads = []

    async with client:
        for finder in finders:
            logger.info("Finding %s downloads...", finder.implementation)
            downloads.extend(await finder.find())

    render(downloads, prune=prune)


def main() -> None:
    parser = argparse.ArgumentParser(description="Fetch Python version metadata.")
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Enable debug logging",
    )
    parser.add_argument(
        "-q",
        "--quiet",
        action="store_true",
        help="Disable logging",
    )
    parser.add_argument(
        "--prune",
        action="store_true",
        help="Retain only recent CPython releases in the generated metadata",
    )
    args = parser.parse_args()

    if args.quiet:
        log_level = logging.CRITICAL
    elif args.verbose:
        log_level = logging.DEBUG
    else:
        log_level = logging.INFO

    logging.basicConfig(
        level=log_level,
        format="%(asctime)s %(levelname)s %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )
    # Silence httpx logging
    logging.getLogger("httpx").setLevel(logging.WARNING)
    logging.getLogger("httpcore").setLevel(logging.WARNING)

    asyncio.run(find(prune=args.prune))


if __name__ == "__main__":
    main()
