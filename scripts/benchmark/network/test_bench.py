"""Checks for the network replay protocol and shared bottleneck."""

from __future__ import annotations

import concurrent.futures
import hashlib
import http.client
import importlib.util
import io
import json
import os
import socket
import subprocess
import tarfile
import tempfile
import threading
import time
import unittest
import urllib.request
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
Server = bench.Server
summary = bench.summary


class ReplayTests(unittest.TestCase):
    def setUp(self) -> None:
        scratch = Path.home() / "code" / "tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=scratch)
        self.body = bytes(range(256)) * 1024
        self.path = Path(self.directory.name) / "example.whl"
        self.path.write_bytes(self.body)
        self.fixtures = SimpleNamespace(
            files={self.path.name: self.path},
            metadata={},
            simple={},
            hashes={self.path.name: hashlib.sha256(self.body).hexdigest()},
        )
        self.servers = []

    def tearDown(self) -> None:
        for server, thread in self.servers:
            server.shutdown()
            server.server_close()
            thread.join()
        self.directory.cleanup()

    def server(self, profile: dict) -> Server:
        server = Server(self.fixtures, profile)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.servers.append((server, thread))
        return server

    def get(self, server: Server, range_: str | None = None) -> tuple[int, bytes]:
        connection = http.client.HTTPConnection(
            "127.0.0.1", server.server_port, timeout=10
        )
        try:
            connection.request(
                "GET", "/files/example.whl", headers={"Range": range_} if range_ else {}
            )
            response = connection.getresponse()
            return response.status, response.read()
        finally:
            connection.close()

    def test_ranges(self) -> None:
        server = self.server({})
        for header, expected in [
            ("bytes=42-99", self.body[42:100]),
            ("bytes=-17", self.body[-17:]),
            ("bytes=262140-", self.body[262140:]),
        ]:
            with self.subTest(header=header):
                self.assertEqual(self.get(server, header), (206, expected))
        self.assertEqual(self.get(server, "bytes=999999-"), (416, b""))

    def test_source_distribution_metadata(self) -> None:
        directory = Path(self.directory.name)
        source = directory / "example-1.0.tar.gz"
        metadata = b"Metadata-Version: 2.4\nName: example\nVersion: 1.0\n\n"
        with tarfile.open(source, "w:gz") as archive:
            member = tarfile.TarInfo("example-1.0/PKG-INFO")
            member.size = len(metadata)
            archive.addfile(member, io.BytesIO(metadata))
        manifest = directory / "source-fixtures.json"
        manifest.write_text(
            json.dumps([{"filename": source.name, "sha256": bench.digest(source)}])
        )
        fixtures = bench.Fixtures(manifest, directory, pep658=False)
        self.assertEqual(fixtures.metadata[source.name + ".metadata"], metadata)
        self.assertEqual(fixtures.packages["example"][0]["core-metadata"], False)

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "Unix sockets unavailable")
    def test_unix_origin(self) -> None:
        socket_path = Path(self.directory.name) / "origin.sock"
        server = Server(self.fixtures, {}, socket_path)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.servers.append((server, thread))
        connection = http.client.HTTPConnection("localhost", timeout=10)
        connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.sock.connect(str(socket_path))
        try:
            connection.request(
                "GET", "/files/example.whl", headers={"Range": "bytes=0-3"}
            )
            response = connection.getresponse()
            self.assertEqual((response.status, response.read()), (206, self.body[:4]))
        finally:
            connection.close()

    def test_interruption_then_resume(self) -> None:
        server = self.server({"cut_after_bytes": 16384, "cut_count": 1})
        with self.assertRaises(http.client.IncompleteRead) as raised:
            self.get(server)
        partial = raised.exception.partial
        status, rest = self.get(server, f"bytes={len(partial)}-")
        self.assertEqual(status, 206)
        self.assertEqual(partial + rest, self.body)

    def test_resumable_oracle(self) -> None:
        server = self.server({"cut_after_bytes": 65536, "cut_count": 3})
        loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        body = bench.read_resumable(
            loopback, server.url + "/files/example.whl", len(self.body)
        )
        self.assertEqual(body, self.body)
        deadline = time.monotonic() + 5
        while server.active and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(
            [event["range"] for event in server.events],
            [None, "bytes=65536-", "bytes=131072-", "bytes=196608-"],
        )

    def test_bandwidth_is_shared(self) -> None:
        server = self.server({"bytes_per_second": len(self.body) * 2})
        start = time.perf_counter()
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            responses = list(pool.map(lambda _: self.get(server), range(2)))
        elapsed = time.perf_counter() - start
        self.assertEqual(responses, [(200, self.body)] * 2)
        self.assertGreaterEqual(elapsed, 0.95)
        self.assertLess(elapsed, 3)

    def test_paired_interval(self) -> None:
        result = summary(
            [{"parent": {"seconds": 2}, "head": {"seconds": 1}}] * 8, samples=100
        )
        self.assertEqual(result["ratio_95ci"], [0.5, 0.5])
        self.assertTrue(result["qualifies_5_percent"])

    def test_kernel_shape_contributes_to_bound(self) -> None:
        with patch.dict(os.environ, {"UV_BENCH_NETEM": '{"rtt_ms":200,"rate_mbit":1}'}):
            self.assertEqual(bench.network_floor({}, 125000, 2), 1)
            self.assertEqual(bench.network_floor({"latency_ms": 100}, 1, 2), 0.6)

    def test_refresh_follows_subcommand(self) -> None:
        args = SimpleNamespace(
            work_dir=Path(self.directory.name),
            requirement=["example==1"],
            python="3.12",
            command=["pip", "compile", "{work}/requirements.in"],
            cache_mode="refresh",
            timeout=10,
            verify_tree=None,
            http2_proxy=None,
        )
        completed = subprocess.CompletedProcess([], 0, b"example==1\n", b"")
        commands = []

        def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess:
            commands.append(command.copy())
            return completed

        with patch.object(bench.subprocess, "run", side_effect=run):
            bench.run_one(Path("uv"), self.fixtures, {}, args)
        warm_command, timed_command = commands
        self.assertNotIn("--refresh", warm_command)
        self.assertEqual(timed_command, [*warm_command, "--refresh"])

    def test_tree_digest_detects_content_changes(self) -> None:
        root = Path(self.directory.name) / "installed"
        root.mkdir()
        file = root / "module.py"
        file.write_text("answer = 42\n")
        original = bench.tree_digest(root)
        self.assertEqual(original, bench.tree_digest(root))
        file.write_text("answer = 43\n")
        self.assertNotEqual(original, bench.tree_digest(root))
        file.write_text("answer = 42\n")
        file.chmod(file.stat().st_mode | 0o111)
        self.assertNotEqual(original, bench.tree_digest(root))


if __name__ == "__main__":
    unittest.main()
