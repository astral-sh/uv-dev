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
from git import Actor, GitCommandError, Repo


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
ALIAS_URL = "https://downloads.python.org/pypy/20220502/python.tar.gz"


class Progress:
    def __init__(self):
        self.completed = 0

    def update(self, count: int):
        self.completed += count

    def close(self):
        pass


class InterruptedStream(httpx.AsyncByteStream):
    async def __aiter__(self):
        yield b"partial archive"
        raise httpx.ReadError("interrupted response")


class PythonMirrorPathsTest(unittest.IsolatedAsyncioTestCase):
    def test_supported_paths_are_decoded(self):
        self.assertEqual(
            MIRROR.sanitize_url(URL.replace("python.tar.gz", "python%2Bdebug.tar.gz")),
            Path("20220502") / "python+debug.tar.gz",
        )
        self.assertEqual(
            MIRROR.sanitize_url("https://downloads.python.org/pypy/pypy.tar.bz2"),
            Path("pypy.tar.bz2"),
        )

    def test_nonrelative_archive_paths_are_rejected(self):
        prefix = MIRROR.PREFIXES[0]
        for suffix in (
            "",
            "/python.tar.gz",
            "../python.tar.gz",
            "20220502/../../python.tar.gz",
            "%2e%2e/python.tar.gz",
            "20220502/%2e%2e/python.tar.gz",
            "20220502//python.tar.gz",
            "20220502/./python.tar.gz",
            "20220502/",
            "C%3A/python.tar.gz",
            "C%3Apython.tar.gz",
            "%5C%5Cserver%5Cpython.tar.gz",
            "20220502%5C..%5Cpython.tar.gz",
            "python.tar.gz%00",
        ):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                MIRROR.sanitize_url(prefix + suffix)
        with self.assertRaises(ValueError):
            MIRROR.sanitize_url("https://example.com/python.tar.gz")

    async def test_invalid_archive_cannot_replace_file_outside_target(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "mirror" / "archives"
            target.mkdir(parents=True)
            outside = root / "outside.tar.gz"
            outside.write_bytes(b"outside mirror")
            invalid_url = MIRROR.PREFIXES[0] + "../../outside.tar.gz"
            requests = []
            progress = Progress()
            original_client = httpx.AsyncClient

            def respond(request: httpx.Request) -> httpx.Response:
                requests.append(str(request.url))
                return httpx.Response(200, content=CONTENT)

            def client(*args, **kwargs):
                return original_client(
                    *args, transport=httpx.MockTransport(respond), **kwargs
                )

            with ExitStack() as stack:
                stack.enter_context(patch.object(MIRROR.httpx, "AsyncClient", client))
                stack.enter_context(patch.object(MIRROR, "tqdm", return_value=progress))
                successful, errors = await MIRROR.download_files(
                    {(invalid_url, CHECKSUM), (URL, CHECKSUM)}, target, 2
                )

            self.assertEqual(outside.read_bytes(), b"outside mirror")
            self.assertEqual(successful, 1)
            self.assertEqual(
                errors, [(invalid_url, f"Invalid mirror archive path in {invalid_url}")]
            )
            self.assertEqual(requests, [URL])
            self.assertEqual(progress.completed, 2)
            self.assertEqual((target / MIRROR.sanitize_url(URL)).read_bytes(), CONTENT)

    async def test_existing_file_cannot_make_unsupported_url_successful(self):
        with TemporaryDirectory() as directory:
            destination = Path(directory) / "python.tar.gz"
            destination.write_bytes(CONTENT)
            progress = Progress()
            errors = []
            requests = []
            url = "https://example.com/python.tar.gz"

            def respond(request: httpx.Request) -> httpx.Response:
                requests.append(str(request.url))
                return httpx.Response(200, content=CONTENT)

            async with httpx.AsyncClient(
                transport=httpx.MockTransport(respond)
            ) as client:
                self.assertFalse(
                    await MIRROR.download_file(
                        client, url, destination, None, progress, errors
                    )
                )
            self.assertEqual(destination.read_bytes(), CONTENT)
            self.assertEqual(requests, [])
            self.assertEqual(
                errors, [(url, f"No valid prefix found for {url}. Skipping.")]
            )
            self.assertEqual(progress.completed, 1)


class PythonMirrorDestinationsTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.target = self.root / "mirror"
        self.destination = self.target / MIRROR.sanitize_url(URL)

    async def download(
        self,
        urls: set[tuple[str, str | None]],
        *,
        contents: dict[str, bytes] | None = None,
        status: int = 200,
        max_concurrent: int = 2,
        target: Path | None = None,
    ):
        self.requests = []
        self.progress = Progress()
        original_client = httpx.AsyncClient

        def respond(request: httpx.Request) -> httpx.Response:
            url = str(request.url)
            self.requests.append(url)
            return httpx.Response(status, content=(contents or {}).get(url, CONTENT))

        def client(*args, **kwargs):
            return original_client(
                *args, transport=httpx.MockTransport(respond), **kwargs
            )

        with ExitStack() as stack:
            stack.enter_context(patch.object(MIRROR.httpx, "AsyncClient", client))
            stack.enter_context(
                patch.object(MIRROR, "tqdm", return_value=self.progress)
            )
            return await MIRROR.download_files(
                urls, target or self.target, max_concurrent
            )

    def conflict_errors(self):
        message = f"Conflicting mirror archive entries for {MIRROR.sanitize_url(URL)}"
        return [(url, message) for url in sorted((URL, ALIAS_URL))]

    async def test_conflicting_archive_destinations_are_rejected(self):
        other_content = b"different complete archive"
        other_checksum = hashlib.sha256(other_content).hexdigest()
        self.destination.parent.mkdir(parents=True)
        for max_concurrent in (1, 2):
            with self.subTest(max_concurrent=max_concurrent):
                self.destination.write_bytes(b"previous archive")
                successful, errors = await self.download(
                    {(URL, CHECKSUM), (ALIAS_URL, other_checksum)},
                    contents={ALIAS_URL: other_content},
                    max_concurrent=max_concurrent,
                )
                self.assertEqual(successful, 0)
                self.assertEqual(errors, self.conflict_errors())
                self.assertEqual(self.requests, [])
                self.assertEqual(self.progress.completed, 2)
                self.assertEqual(self.destination.read_bytes(), b"previous archive")

    async def test_matching_archive_destinations_are_downloaded_once(self):
        successful, errors = await self.download(
            {(URL, CHECKSUM), (ALIAS_URL, CHECKSUM)}
        )
        self.assertEqual(successful, 2)
        self.assertEqual(errors, [])
        self.assertEqual(self.requests, [min(URL, ALIAS_URL)])
        self.assertEqual(self.progress.completed, 2)
        self.assertEqual(self.destination.read_bytes(), CONTENT)

    async def test_unverified_archive_destinations_are_rejected(self):
        for index, checksums in enumerate(
            ((CHECKSUM, None), (None, None), ("", ""), ("invalid", "invalid"))
        ):
            with self.subTest(checksums=checksums):
                target = self.root / f"mirror-{index}"
                successful, errors = await self.download(
                    {(URL, checksums[0]), (ALIAS_URL, checksums[1])}, target=target
                )
                self.assertEqual(successful, 0)
                self.assertEqual(errors, self.conflict_errors())
                self.assertEqual(self.requests, [])
                self.assertEqual(self.progress.completed, 2)
                self.assertFalse(target.exists())

    async def test_conflict_does_not_block_an_independent_destination(self):
        other_url = URL.replace("python.tar.gz", "other-python.tar.gz")
        self.destination.parent.mkdir(parents=True)
        self.destination.write_bytes(b"previous archive")
        successful, errors = await self.download(
            {(URL, CHECKSUM), (ALIAS_URL, None), (other_url, CHECKSUM)}
        )
        self.assertEqual(successful, 1)
        self.assertEqual(errors, self.conflict_errors())
        self.assertEqual(self.requests, [other_url])
        self.assertEqual(self.progress.completed, 3)
        self.assertEqual(self.destination.read_bytes(), b"previous archive")
        self.assertEqual(
            (self.target / MIRROR.sanitize_url(other_url)).read_bytes(), CONTENT
        )

    async def test_failed_shared_download_marks_every_url_failed(self):
        successful, errors = await self.download(
            {(URL, CHECKSUM), (ALIAS_URL, CHECKSUM)}, status=503
        )
        self.assertEqual(successful, 0)
        self.assertEqual([url for url, _ in errors], sorted((URL, ALIAS_URL)))
        self.assertEqual(errors[0][1], errors[1][1])
        self.assertEqual(self.requests, [min(URL, ALIAS_URL)])
        self.assertEqual(self.progress.completed, 2)
        self.assertFalse(self.destination.exists())


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


class PythonMirrorHistoryTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.repository = Repo.init(self.root)
        self.addCleanup(self.repository.close)
        self.versions_file = (
            self.root / "crates" / "uv-python" / "download-metadata.json"
        )
        self.versions_file.parent.mkdir(parents=True)
        self.relative_path = str(self.versions_file.relative_to(self.root))
        self.actor = Actor("Python mirror test", "mirror@example.com")

    def commit(self, contents: str):
        self.versions_file.write_text(contents)
        self.repository.index.add([self.relative_path])
        return self.repository.index.commit(
            "Update Python metadata", author=self.actor, committer=self.actor
        )

    def collect(self):
        with ExitStack() as stack:
            stack.enter_context(patch.object(MIRROR, "REPO_ROOT", self.root))
            stack.enter_context(
                patch.object(MIRROR, "VERSIONS_FILE", self.versions_file)
            )
            return MIRROR.collect_metadata_from_git_history()

    def test_complete_history_returns_every_revision(self):
        earlier = {"url": URL, "sha256": None}
        current = {
            "url": URL.replace("python.tar.gz", "current.tar.gz"),
            "sha256": None,
        }
        self.commit(json.dumps({"python": earlier}))
        self.commit(json.dumps({"python": current}))
        self.assertEqual(self.collect(), [current, earlier])

    def test_deleted_metadata_revision_is_skipped(self):
        earlier = {"url": URL, "sha256": None}
        self.commit(json.dumps({"python": earlier}))
        self.repository.index.remove([self.relative_path], working_tree=True)
        self.repository.index.commit(
            "Remove Python metadata", author=self.actor, committer=self.actor
        )
        self.assertEqual(self.collect(), [earlier])

    def test_malformed_metadata_does_not_return_partial_history(self):
        self.commit(json.dumps({"python": {"url": URL, "sha256": None}}))
        malformed = self.commit("{invalid JSON")
        self.commit(json.dumps({"python": {"url": URL, "sha256": None}}))
        with self.assertRaisesRegex(ValueError, malformed.hexsha):
            self.collect()

    def test_interrupted_git_history_does_not_return_partial_metadata(self):
        commit = self.commit(json.dumps({"python": {"url": URL, "sha256": None}}))

        def interrupted():
            yield commit
            raise GitCommandError("git log", 128, stderr="history read failed")

        with ExitStack() as stack:
            stack.enter_context(
                patch.object(MIRROR, "Repo", return_value=self.repository)
            )
            stack.enter_context(
                patch.object(
                    self.repository, "iter_commits", return_value=interrupted()
                )
            )
            with self.assertRaises(GitCommandError):
                self.collect()


class PythonMirrorCliTest(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.target = self.root / "mirror"
        self.requests = []

    def run_cli(
        self,
        responses: dict[str, int],
        client_error: OSError | None = None,
        history_error: GitCommandError | None = None,
    ):
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
            arguments = [str(MIRROR.__file__), "--target", str(self.target)]
            if history_error is not None:
                stack.enter_context(patch("git.Repo", side_effect=history_error))
                arguments.append("--from-all-history")
            stack.enter_context(
                patch.object(
                    sys,
                    "argv",
                    arguments,
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

    def test_history_error_exits_nonzero_before_downloads(self):
        code, output = self.run_cli(
            {}, history_error=GitCommandError("git log", 128, stderr="history failed")
        )
        self.assertEqual(code, 1)
        self.assertEqual(self.requests, [])
        self.assertEqual(output, "")

    def test_conflicting_destinations_exit_nonzero_before_downloads(self):
        code, output = self.run_cli({URL: 200, ALIAS_URL: 200})
        self.assertEqual(code, 1)
        self.assertEqual(self.requests, [])
        self.assertIn("Successfully downloaded: 0 files.", output)
        self.assertIn("Failed downloads:", output)
        self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main()
