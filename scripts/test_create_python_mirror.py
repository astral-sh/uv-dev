# /// script
# requires-python = ">=3.8"
# dependencies = [
#     "gitpython",
#     "httpx",
#     "tqdm",
# ]
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Offline regression tests for Python mirror archive publication."""

from __future__ import annotations

import asyncio
import builtins
import hashlib
import importlib.util
import io
import json
import runpy
import sys
import unittest
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from types import ModuleType
from unittest.mock import patch

import httpx


def load_mirror(path: Path) -> ModuleType:
    specification = importlib.util.spec_from_file_location("_uv_python_mirror", path)
    if specification is None or specification.loader is None:
        raise ImportError(f"Cannot load Python mirror script: {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


MIRROR = load_mirror(Path(__file__).with_name("create-python-mirror.py"))
URL = "https://github.com/astral-sh/python-build-standalone/releases/download/20220502/python.tar.gz"
CONTENT = b"complete archive"
CHECKSUM = hashlib.sha256(CONTENT).hexdigest()


class Progress:
    def __init__(self):
        self.completed = 0

    def update(self, count: int):
        self.completed += count


class InterruptedStream(httpx.AsyncByteStream):
    async def __aiter__(self):
        yield b"partial archive"
        raise httpx.ReadError("interrupted response")


class PythonMirrorDownloadsTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.destination = self.root / "mirror" / "python.tar.gz"
        self.progress = Progress()
        self.errors = []
        self.requests = []

    async def download(self, response: httpx.Response, checksum: str | None):
        def respond(request: httpx.Request) -> httpx.Response:
            self.requests.append(str(request.url))
            return response

        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            return await MIRROR.download_file(
                client,
                URL,
                self.destination,
                checksum,
                self.progress,
                self.errors,
            )

    def assert_final_files(self, expected: list[str]):
        self.assertEqual(
            sorted(path.name for path in self.destination.parent.iterdir()), expected
        )

    async def test_interrupted_checksumless_download_is_not_reused(self):
        self.assertFalse(
            await self.download(httpx.Response(200, stream=InterruptedStream()), None)
        )
        self.assertFalse(self.destination.exists())
        self.assert_final_files([])
        self.assertTrue(await self.download(httpx.Response(200, content=CONTENT), None))
        self.assertEqual(self.destination.read_bytes(), CONTENT)
        self.assertEqual(self.requests, [URL, URL])
        self.assertEqual(self.errors, [(URL, "interrupted response")])
        self.assertEqual(self.progress.completed, 2)
        self.assert_final_files([self.destination.name])

    async def test_interrupted_replacement_keeps_existing_archive(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"previous archive")
        self.assertFalse(
            await self.download(
                httpx.Response(200, stream=InterruptedStream()), CHECKSUM
            )
        )
        self.assertEqual(self.destination.read_bytes(), b"previous archive")
        self.assertEqual(self.requests, [URL])
        self.assertEqual(self.errors, [(URL, "interrupted response")])
        self.assert_final_files([self.destination.name])

    async def test_checksum_failure_does_not_publish(self):
        self.assertFalse(
            await self.download(httpx.Response(200, content=b"wrong archive"), CHECKSUM)
        )
        self.assertFalse(self.destination.exists())
        self.assertEqual(self.errors, [(URL, "Checksum mismatch")])
        self.assertEqual(self.progress.completed, 1)
        self.assert_final_files([])

    async def test_checksum_failure_keeps_existing_archive(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"previous archive")
        self.assertFalse(
            await self.download(httpx.Response(200, content=b"wrong archive"), CHECKSUM)
        )
        self.assertEqual(self.destination.read_bytes(), b"previous archive")
        self.assertEqual(self.errors, [(URL, "Checksum mismatch")])
        self.assert_final_files([self.destination.name])

    async def test_verified_replacement_publishes_complete_archive(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"previous archive")
        self.assertTrue(
            await self.download(httpx.Response(200, content=CONTENT), CHECKSUM)
        )
        self.assertEqual(self.destination.read_bytes(), CONTENT)
        self.assertEqual(self.errors, [])
        self.assertEqual(self.progress.completed, 1)
        self.assert_final_files([self.destination.name])

    async def test_matching_archive_is_not_requested(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(CONTENT)
        self.assertTrue(await self.download(httpx.Response(503), CHECKSUM))
        self.assertEqual(self.destination.read_bytes(), CONTENT)
        self.assertEqual(self.requests, [])
        self.assertEqual(self.errors, [])
        self.assertEqual(self.progress.completed, 1)

    async def test_http_error_keeps_existing_archive(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"previous archive")
        self.assertFalse(await self.download(httpx.Response(503), CHECKSUM))
        self.assertEqual(self.destination.read_bytes(), b"previous archive")
        self.assertEqual(len(self.errors), 1)
        self.assertEqual(self.progress.completed, 1)
        self.assert_final_files([self.destination.name])

    async def test_failed_atomic_replace_keeps_existing_archive(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"previous archive")
        with patch.object(Path, "replace", side_effect=OSError("replacement denied")):
            self.assertFalse(
                await self.download(httpx.Response(200, content=CONTENT), CHECKSUM)
            )
        self.assertEqual(self.destination.read_bytes(), b"previous archive")
        self.assertEqual(self.errors, [(URL, "replacement denied")])
        self.assert_final_files([self.destination.name])

    async def test_cancellation_discards_staged_download(self):
        started = asyncio.Event()
        release = asyncio.Event()

        class PausedStream(httpx.AsyncByteStream):
            async def __aiter__(self):
                yield b"partial archive"
                started.set()
                await release.wait()

        task = asyncio.create_task(
            self.download(httpx.Response(200, stream=PausedStream()), None)
        )
        await started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(self.destination.exists())
        self.assert_final_files([])


class PythonMirrorArgumentsTest(unittest.TestCase):
    def parse_arguments(self, *arguments: str):
        with patch.object(sys, "argv", [str(MIRROR.__file__), *arguments]):
            return MIRROR.parse_arguments()

    def test_default_download_concurrency(self):
        self.assertEqual(self.parse_arguments().max_concurrent, 20)

    def test_positive_download_concurrency(self):
        for value in (1, 20, 100):
            with self.subTest(value=value):
                self.assertEqual(
                    self.parse_arguments("--max-concurrent", str(value)).max_concurrent,
                    value,
                )

    def test_nonpositive_download_concurrency(self):
        for value in (0, -1):
            with self.subTest(value=value):
                error = io.StringIO()
                with redirect_stderr(error), self.assertRaises(SystemExit) as caught:
                    self.parse_arguments("--max-concurrent", str(value))
                self.assertEqual(caught.exception.code, 2)
                self.assertIn("--max-concurrent", error.getvalue())
                self.assertIn("must be greater than zero", error.getvalue())

    def test_noninteger_download_concurrency(self):
        error = io.StringIO()
        with redirect_stderr(error), self.assertRaises(SystemExit) as caught:
            self.parse_arguments("--max-concurrent", "many")
        self.assertEqual(caught.exception.code, 2)
        self.assertIn("invalid int value", error.getvalue())


class PythonMirrorCliTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.target = self.root / "mirror"
        self.requests = []

    def run_cli(self, responses: dict[str, int], client_error: OSError | None = None):
        original_open = builtins.open
        original_client = httpx.AsyncClient
        metadata = json.dumps({url: {"url": url, "sha256": None} for url in responses})

        def open_metadata(path, *args, **kwargs):
            if path == MIRROR.VERSIONS_FILE:
                return io.StringIO(metadata)
            return original_open(path, *args, **kwargs)

        def respond(request: httpx.Request) -> httpx.Response:
            url = str(request.url)
            self.requests.append(url)
            return httpx.Response(responses[url], content=CONTENT)

        def client(*args, **kwargs):
            if client_error is not None:
                raise client_error
            return original_client(
                *args, transport=httpx.MockTransport(respond), **kwargs
            )

        output = io.StringIO()
        with ExitStack() as stack:
            stack.enter_context(patch.object(builtins, "open", open_metadata))
            stack.enter_context(patch.object(httpx, "AsyncClient", client))
            stack.enter_context(
                patch.object(
                    sys,
                    "argv",
                    [str(MIRROR.__file__), "--target", str(self.target)],
                )
            )
            stack.enter_context(redirect_stdout(output))
            try:
                runpy.run_path(str(MIRROR.__file__), run_name="__main__")
            except SystemExit as error:
                return error.code, output.getvalue()
        return 0, output.getvalue()

    def test_complete_download_failure_exits_nonzero(self):
        code, output = self.run_cli({URL: 503})
        self.assertEqual(code, 1)
        self.assertEqual(self.requests, [URL])
        self.assertIn("Successfully downloaded: 0 files.", output)
        self.assertIn("Failed downloads:", output)
        self.assertFalse((self.target / MIRROR.sanitize_url(URL)).exists())

    def test_mixed_download_failure_exits_nonzero(self):
        other_url = URL.replace("python.tar.gz", "other-python.tar.gz")
        code, output = self.run_cli({URL: 200, other_url: 503})
        self.assertEqual(code, 1)
        self.assertCountEqual(self.requests, [URL, other_url])
        self.assertIn("Successfully downloaded: 1 files.", output)
        self.assertIn("Failed downloads:", output)
        self.assertEqual((self.target / MIRROR.sanitize_url(URL)).read_bytes(), CONTENT)
        self.assertFalse((self.target / MIRROR.sanitize_url(other_url)).exists())

    def test_successful_downloads_exit_zero(self):
        code, output = self.run_cli({URL: 200})
        self.assertEqual(code, 0)
        self.assertEqual(self.requests, [URL])
        self.assertIn("Successfully downloaded: 1 files.", output)
        self.assertNotIn("Failed downloads:", output)
        self.assertEqual((self.target / MIRROR.sanitize_url(URL)).read_bytes(), CONTENT)

    def test_download_exception_exits_nonzero(self):
        code, output = self.run_cli({URL: 200}, OSError("client unavailable"))
        self.assertEqual(code, 1)
        self.assertEqual(self.requests, [])
        self.assertEqual(output, "")

    def test_empty_selection_exits_zero(self):
        code, output = self.run_cli({})
        self.assertEqual(code, 0)
        self.assertEqual(self.requests, [])
        self.assertEqual(output, "")


if __name__ == "__main__":
    unittest.main()
