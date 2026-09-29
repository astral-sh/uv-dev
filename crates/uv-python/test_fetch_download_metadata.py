# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "httpx < 1",
# ]
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for the Python download metadata generator."""

import copy
import importlib.util
import json
import sys
import unittest
from pathlib import Path
from types import ModuleType
from typing import Any

import httpx


def load_generator(path: Path) -> ModuleType:
    specification = importlib.util.spec_from_file_location(
        "_uv_fetch_download_metadata", path
    )
    if specification is None or specification.loader is None:
        raise ImportError(f"Cannot load download metadata generator: {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


GENERATOR = load_generator(Path(__file__).with_name("fetch-download-metadata.py"))
# Complete and incomplete records for the same Python version, retained from:
# https://github.com/pypy/pypy/blob/33bdf130f3d62b19d3313e5c2fad562ef10398e6/pypy/tool/release/versions.json
RELEASES = json.loads(
    (Path(__file__).with_name("tests") / "pypy-releases.json").read_text()
)


class PyPyDownloadsTest(unittest.IsolatedAsyncioTestCase):
    async def downloads(self, releases: list[dict[str, Any]]) -> list[Any]:
        requests = []

        def respond(request: httpx.Request) -> httpx.Response:
            requests.append(str(request.url))
            if str(request.url) == GENERATOR.PyPyFinder.RELEASE_URL:
                return httpx.Response(200, json=releases)
            if str(request.url) == GENERATOR.PyPyFinder.CHECKSUM_URL:
                return httpx.Response(200, text="")
            raise AssertionError(f"Unexpected metadata request: {request.url}")

        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            downloads = await GENERATOR.PyPyFinder(client).find()
        self.assertEqual(
            requests,
            [GENERATOR.PyPyFinder.RELEASE_URL, GENERATOR.PyPyFinder.CHECKSUM_URL],
        )
        return downloads

    @staticmethod
    def filenames(downloads: list[Any]) -> list[str]:
        return sorted(download.filename for download in downloads)

    async def test_incomplete_release_is_independent_of_file_order(self):
        incomplete = RELEASES[1]
        files = incomplete["files"]
        for offset in range(len(files)):
            reordered = {**incomplete, "files": files[offset:] + files[:offset]}
            with self.subTest(first=reordered["files"][0]["filename"]):
                self.assertEqual(await self.downloads([reordered]), [])

    async def test_complete_release_survives_an_incomplete_release(self):
        complete, incomplete = RELEASES
        expected = sorted(file["filename"] for file in complete["files"])
        for releases in ([complete, incomplete], [incomplete, complete]):
            with self.subTest(first=releases[0]["pypy_version"]):
                downloads = await self.downloads(releases)
                self.assertEqual(self.filenames(downloads), expected)
                self.assertEqual(
                    {download.build for download in downloads},
                    {complete["pypy_version"]},
                )

    async def test_complete_release_retains_all_platforms(self):
        complete = RELEASES[0]
        downloads = await self.downloads([complete])
        self.assertEqual(
            self.filenames(downloads),
            sorted(file["filename"] for file in complete["files"]),
        )
        self.assertEqual(
            {download.triple.platform for download in downloads},
            {"linux", "darwin", "windows"},
        )

    async def test_first_complete_release_keeps_priority(self):
        complete = RELEASES[0]
        older = copy.deepcopy(complete)
        older["pypy_version"] = "7.3.17"
        for file in older["files"]:
            for key in ("filename", "download_url"):
                file[key] = file[key].replace("7.3.19", "7.3.17")
        downloads = await self.downloads([complete, older])
        self.assertEqual(
            {download.build for download in downloads}, {complete["pypy_version"]}
        )

    async def test_ineligible_releases_do_not_block_complete_releases(self):
        complete, incomplete = RELEASES
        releases = [
            {**incomplete, "stable": False},
            {"stable": True, "python_version": "3.6.0"},
            complete,
        ]
        self.assertEqual(
            self.filenames(await self.downloads(releases)),
            sorted(file["filename"] for file in complete["files"]),
        )

    async def test_release_request_error_is_reported(self):
        requests = []

        def respond(request: httpx.Request) -> httpx.Response:
            requests.append(str(request.url))
            return httpx.Response(503)

        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            with self.assertRaises(httpx.HTTPStatusError):
                await GENERATOR.PyPyFinder(client).find()
        self.assertEqual(requests, [GENERATOR.PyPyFinder.RELEASE_URL])


if __name__ == "__main__":
    unittest.main()
