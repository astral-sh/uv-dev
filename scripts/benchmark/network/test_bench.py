"""Checks for the network replay protocol and shared bottleneck."""

from __future__ import annotations

import base64
import concurrent.futures
import csv
import hashlib
import http.client
import importlib.util
import io
import json
import os
import shutil
import socket
import subprocess
import tarfile
import tempfile
import threading
import time
import unittest
import urllib.request
import zipfile
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


class NetworkFloorTests(unittest.TestCase):
    def test_uniform_concurrent_requests(self) -> None:
        self.assertEqual(bench.concurrent_latency_floor([150] * 9, 2), (5, 750))
        self.assertEqual(bench.concurrent_latency_floor([150] * 9, 50), (1, 150))
        self.assertEqual(bench.concurrent_latency_floor([150] * 9, 2, 100), (5, 750))

    def test_heterogeneous_requests_overlap_rtt(self) -> None:
        waves, latency = bench.concurrent_latency_floor([100, 0, 0], 2, 100)
        with patch.dict(os.environ, {"UV_BENCH_NETEM": '{"rtt_ms":100}'}):
            self.assertEqual(bench.network_floor({}, 0, waves, latency), 0.2)


class RejectedPairTests(unittest.TestCase):
    def test_rejected_pair_retains_raw_samples_and_accepted_results(self) -> None:
        for accepted_pairs in (0, 1):
            with (
                self.subTest(accepted_pairs=accepted_pairs),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                manifest = root / "fixtures.json"
                manifest.write_text("[]")
                profiles = root / "profiles.json"
                profiles.write_text('{"fast":{}}')
                binary = root / "uv"
                binary.write_bytes(b"binary")
                output = root / "result.json"
                sample = {
                    "seconds": 1.0,
                    "stdout_sha256": "same-output",
                    "stderr_sha256": "same-error",
                    "stderr": "original diagnostic",
                    "verified_tree": None,
                    "verified_files": {},
                }
                # Reject the initial warmup, or retain one accepted pair before
                # rejecting the next measured pair.
                successful_iterations = 2 if accepted_pairs else 0
                observations = [dict(sample) for _ in range(2 * successful_iterations)]
                observations.extend(
                    [
                        dict(sample),
                        dict(
                            sample,
                            stderr_sha256="different-error",
                            stderr="changed diagnostic",
                        ),
                    ]
                )
                argv = [
                    "bench.py",
                    "--manifest",
                    str(manifest),
                    "--directory",
                    str(root),
                    "run",
                    "--parent",
                    str(binary),
                    "--head",
                    str(binary),
                    "--parent-sha",
                    "parent",
                    "--head-sha",
                    "head",
                    "--profile",
                    "fast",
                    "--profiles",
                    str(profiles),
                    "--work-dir",
                    str(root / "trials"),
                    "--output",
                    str(output),
                    "--pairs",
                    "2",
                    "--warmups",
                    "1",
                    "--compare-stderr",
                    "--",
                    "pip",
                    "compile",
                ]
                with (
                    patch("sys.argv", argv),
                    patch.object(
                        bench.subprocess, "check_output", return_value="uv test"
                    ),
                    patch.object(bench, "netem_profile", return_value={}),
                    patch.object(bench, "run_one", side_effect=observations),
                    self.assertRaisesRegex(ValueError, "diagnostic outputs differ"),
                ):
                    bench.main()
                rejected = json.loads(output.with_suffix(".failure.json").read_text())
                self.assertEqual(len(rejected["pairs"]), accepted_pairs)
                self.assertNotIn("summary", rejected)
                failure = rejected["failure"]
                self.assertEqual(failure["iteration"], successful_iterations)
                self.assertEqual(failure["warmup"], accepted_pairs == 0)
                self.assertEqual(
                    failure["pair"]["parent"]["stderr"], "original diagnostic"
                )
                self.assertEqual(
                    failure["pair"]["head"]["stderr"], "changed diagnostic"
                )
                if accepted_pairs:
                    self.assertEqual(
                        json.loads(output.read_text())["pairs"], rejected["pairs"]
                    )
                else:
                    self.assertFalse(output.exists())


class KernelCounterTests(unittest.TestCase):
    def test_tcp_table_omits_gauges(self) -> None:
        self.assertEqual(
            bench.tcp_counters(
                "Ip: InReceives\nIp: 100\n"
                "Tcp: RtoAlgorithm MaxConn CurrEstab InSegs RetransSegs\n"
                "Tcp: 1 -1 2 10 3\n"
                "TcpExt: TCPSynRetrans TCPTimeouts\nTcpExt: 1 4\n"
            ),
            {
                "TcpInSegs": 10,
                "TcpRetransSegs": 3,
                "TcpExtTCPSynRetrans": 1,
                "TcpExtTCPTimeouts": 4,
            },
        )
        with self.assertRaisesRegex(ValueError, "Incomplete"):
            bench.tcp_counters("Tcp: InSegs\n")
        with self.assertRaisesRegex(ValueError, "columns differ"):
            bench.tcp_counters("Tcp: InSegs RetransSegs\nTcp: 1\n")

    def test_delta_rejects_network_changes(self) -> None:
        before = {
            "kernel": "test",
            "network_namespace": "net:[1]",
            "placement": "loopback egress",
            "device": "lo",
            "qdisc": {
                "kind": "netem",
                "handle": "1:",
                "root": True,
                "options": {"limit": 1000},
                "bytes": 100,
                "packets": 2,
                "drops": 0,
                "overlimits": 0,
                "requeues": 0,
                "qlen": 1,
            },
            "tcp": {"TcpInSegs": 2, "TcpRetransSegs": 1},
        }
        after = json.loads(json.dumps(before))
        after["qdisc"].update(bytes=250, packets=5, drops=1, qlen=0)
        after["tcp"].update(TcpInSegs=5, TcpRetransSegs=2)
        result = bench.kernel_network_delta(before, after)
        self.assertEqual(result["delta"]["qdisc"]["bytes"], 150)
        self.assertEqual(result["delta"]["qdisc"]["drops"], 1)
        self.assertNotIn("qlen", result["delta"]["qdisc"])
        self.assertEqual(result["delta"]["tcp"]["TcpRetransSegs"], 1)
        after["qdisc"]["options"]["limit"] = 2000
        with self.assertRaisesRegex(ValueError, "qdisc options changed"):
            bench.kernel_network_delta(before, after)
        after["qdisc"]["options"]["limit"] = 1000
        after["tcp"]["TcpRetransSegs"] = 0
        with self.assertRaisesRegex(ValueError, "counters decreased"):
            bench.kernel_network_delta(before, after)


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
            routes={"/releases/example.tar.gz": self.path},
            metadata={},
            simple={},
            flat=b"<!doctype html><a href='/files/example.whl'>example</a>",
            hashes={self.path.name: hashlib.sha256(self.body).hexdigest()},
        )
        self.servers = []

    def tearDown(self) -> None:
        for server, thread in self.servers:
            server.shutdown()
            server.server_close()
            thread.join()
        self.directory.cleanup()

    def server(self, profile: dict, *, git_root: Path | None = None) -> Server:
        server = Server(self.fixtures, profile, git_root=git_root)
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

    def test_authentication_challenge_body_and_retry(self) -> None:
        authorization = "Basic dXNlcjpwYXNzd29yZA=="
        for status, chunked in (
            (401, False),
            (403, False),
            (404, False),
            (401, True),
            (403, True),
            (404, True),
        ):
            with self.subTest(status=status, chunked=chunked):
                server = self.server(
                    {
                        "auth_challenges": {
                            "/files/example.whl": {
                                "authorization": authorization,
                                "status": status,
                                "body_bytes": 17,
                                "chunked": chunked,
                            }
                        }
                    }
                )
                connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
                try:
                    connection.request("GET", "/files/example.whl")
                    response = connection.getresponse()
                    self.assertEqual(response.status, status)
                    self.assertEqual(
                        response.getheader("Transfer-Encoding"),
                        "chunked" if chunked else None,
                    )
                    self.assertEqual(
                        response.getheader("Content-Length"), None if chunked else "17"
                    )
                    self.assertEqual(response.read(), b"!" * 17)
                    if status == 401:
                        self.assertEqual(
                            response.getheader("WWW-Authenticate"),
                            'Basic realm="network-replay"',
                        )
                    connection.request(
                        "GET",
                        "/files/example.whl",
                        headers={"Authorization": authorization, "Range": "bytes=0-3"},
                    )
                    response = connection.getresponse()
                    self.assertEqual(
                        (response.status, response.read()), (206, self.body[:4])
                    )
                finally:
                    connection.close()
                server.wait_idle()
                events = sorted(server.events, key=lambda event: event["attempt"])
                self.assertEqual(
                    [event["authentication"] for event in events],
                    ["challenge", "accepted"],
                )
                self.assertEqual([event["bytes"] for event in events], [17, 4])
                self.assertEqual(
                    events[0].get("transfer_encoding"), "chunked" if chunked else None
                )
                self.assertEqual(
                    events[0]["origin_connection"], events[1]["origin_connection"]
                )
                self.assertTrue(
                    all(authorization not in json.dumps(event) for event in events)
                )

    def test_transient_error_body_and_retry(self) -> None:
        for chunked in (False, True):
            with self.subTest(chunked=chunked):
                server = self.server(
                    {
                        "path_failures": {
                            "/files/example.whl": {
                                "status": 503,
                                "count": 1,
                                "body_bytes": 12345,
                                "chunked": chunked,
                                "retry_after": "2",
                            }
                        }
                    }
                )
                connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
                try:
                    connection.request("GET", "/files/example.whl")
                    response = connection.getresponse()
                    self.assertEqual(response.status, 503)
                    self.assertEqual(response.getheader("Retry-After"), "2")
                    self.assertEqual(
                        response.getheader("Transfer-Encoding"),
                        "chunked" if chunked else None,
                    )
                    self.assertEqual(
                        response.getheader("Content-Length"),
                        None if chunked else "12345",
                    )
                    self.assertEqual(response.read(), b"!" * 12345)
                    connection.request(
                        "GET", "/files/example.whl", headers={"Range": "bytes=0-3"}
                    )
                    response = connection.getresponse()
                    self.assertIsNone(response.getheader("Retry-After"))
                    self.assertEqual(
                        (response.status, response.read()), (206, self.body[:4])
                    )
                finally:
                    connection.close()
                server.wait_idle()
                events = sorted(server.events, key=lambda event: event["attempt"])
                self.assertEqual([event["status"] for event in events], [503, 206])
                self.assertEqual([event["bytes"] for event in events], [12345, 4])
                self.assertEqual(
                    events[0]["origin_connection"], events[1]["origin_connection"]
                )

    def test_disconnect_before_headers_is_recorded(self) -> None:
        server = self.server(
            {"path_failures": {"/files/example.whl": {"disconnect": True, "count": 1}}}
        )
        with self.assertRaises(http.client.RemoteDisconnected):
            self.get(server)
        self.assertEqual(self.get(server), (200, self.body))
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["attempt"])
        self.assertEqual([event["status"] for event in events], [0, 200])
        self.assertEqual([event["bytes"] for event in events], [0, len(self.body)])
        self.assertEqual(events[0]["injected_disconnect"], "before-headers")

    def test_delayed_status_after_interruption(self) -> None:
        server = self.server(
            {
                "cut_after_bytes": 17,
                "cut_count": 1,
                "path_failures": {
                    "/files/example.whl": {
                        "status": 503,
                        "start_after": 1,
                        "count": 2,
                    }
                },
            }
        )
        with self.assertRaises(http.client.IncompleteRead) as error:
            self.get(server)
        self.assertEqual(error.exception.partial, self.body[:17])
        for _ in range(2):
            self.assertEqual(
                self.get(server, "bytes=17-"), (503, b"Injected transient failure")
            )
        self.assertEqual(self.get(server, "bytes=17-"), (206, self.body[17:]))
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["attempt"])
        self.assertEqual([event["status"] for event in events], [200, 503, 503, 206])
        self.assertEqual(
            [event["range"] for event in events], [None] + ["bytes=17-"] * 3
        )
        self.assertTrue(events[0]["injected_disconnect"])

    def test_delayed_disconnect(self) -> None:
        server = self.server(
            {
                "path_failures": {
                    "/files/example.whl": {
                        "disconnect": True,
                        "start_after": 1,
                        "count": 1,
                    }
                }
            }
        )
        self.assertEqual(self.get(server), (200, self.body))
        with self.assertRaises(http.client.RemoteDisconnected):
            self.get(server)
        self.assertEqual(self.get(server), (200, self.body))
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["attempt"])
        self.assertEqual([event["status"] for event in events], [200, 0, 200])

    def test_delayed_truncation(self) -> None:
        server = self.server(
            {
                "path_failures": {
                    "/files/example.whl": {
                        "cut_after_bytes": 17,
                        "start_after": 1,
                        "count": 1,
                    }
                }
            }
        )
        self.assertEqual(self.get(server), (200, self.body))
        with self.assertRaises(http.client.IncompleteRead) as error:
            self.get(server)
        self.assertEqual(error.exception.partial, self.body[:17])
        self.assertEqual(self.get(server), (200, self.body))
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["attempt"])
        self.assertEqual(
            [event["bytes"] for event in events], [len(self.body), 17, len(self.body)]
        )

    def test_invalid_failure_start(self) -> None:
        for start_after in (-1, True, "1"):
            with (
                self.subTest(start_after=start_after),
                self.assertRaisesRegex(ValueError, "Invalid failure start"),
            ):
                self.server(
                    {
                        "path_failures": {
                            "/files/example.whl": {"start_after": start_after}
                        }
                    }
                )

    def test_truncated_metadata_body_is_recorded(self) -> None:
        self.fixtures.simple["example"] = b'{"name":"example","files":[]}'
        for path, body in (
            ("/simple/example/", self.fixtures.simple["example"]),
            ("/flat/extra", self.fixtures.flat),
        ):
            with self.subTest(path=path):
                server = self.server(
                    {"path_failures": {path: {"cut_after_bytes": 17, "count": 1}}}
                )
                connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
                try:
                    connection.request("GET", path)
                    response = connection.getresponse()
                    with self.assertRaises(http.client.IncompleteRead) as error:
                        response.read()
                    self.assertEqual(error.exception.partial, body[:17])
                    connection.close()
                    connection.request("GET", path)
                    response = connection.getresponse()
                    self.assertEqual(response.read(), body)
                finally:
                    connection.close()
                server.wait_idle()
                events = sorted(server.events, key=lambda event: event["attempt"])
                self.assertEqual([event["status"] for event in events], [200, 200])
                self.assertEqual([event["bytes"] for event in events], [17, len(body)])
                self.assertEqual(events[0]["response_length"], len(body))
                self.assertTrue(events[0]["injected_disconnect"])

    def test_connection_reuse_is_recorded(self) -> None:
        server = self.server({})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            for _ in range(2):
                connection.request(
                    "GET", "/files/example.whl", headers={"Range": "bytes=0-3"}
                )
                response = connection.getresponse()
                self.assertEqual(response.read(), self.body[:4])
        finally:
            connection.close()
        self.get(server, "bytes=0-3")
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["start"])
        self.assertEqual(events[0]["origin_connection"], events[1]["origin_connection"])
        self.assertNotEqual(
            events[1]["origin_connection"], events[2]["origin_connection"]
        )

    def test_osv_query_pages_and_failures_are_scoped_to_the_batch(self) -> None:
        self.fixtures.osv = {
            "dependencies": {
                "first": {"version": "1.0", "pages": 2},
                "second": {"version": "2.0", "pages": 1},
            }
        }
        server = self.server({"osv_failures": {"first:0": {"status": 503, "count": 1}}})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)

        def send(queries: list[dict]) -> tuple[int, bytes]:
            connection.request(
                "POST",
                "/v1/querybatch",
                json.dumps({"queries": queries}),
                {"Content-Type": "application/json"},
            )
            response = connection.getresponse()
            return response.status, response.read()

        first = {"package": {"name": "first", "ecosystem": "PyPI"}, "version": "1.0"}
        second = {
            "package": {"name": "second", "ecosystem": "PyPI"},
            "version": "2.0",
        }
        try:
            self.assertEqual(send([first])[0], 503)
            self.assertEqual(send([second]), (200, b'{"results":[{"vulns":[]}]}'))
            status, body = send([first])
            self.assertEqual(status, 200)
            self.assertEqual(
                json.loads(body),
                {"results": [{"vulns": [], "next_page_token": "first:1"}]},
            )
            self.assertEqual(
                send([dict(first, page_token="first:1")]),
                (200, b'{"results":[{"vulns":[]}]}'),
            )
            self.assertEqual(send([dict(first, page_token="second:1")])[0], 400)
            self.assertEqual(send([dict(first, version="3.0")])[0], 400)
        finally:
            connection.close()
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["start"])
        self.assertEqual([event["attempt"] for event in events], [1, 1, 2, 1])
        self.assertEqual(events[0]["query_sha256"], events[2]["query_sha256"])
        self.assertNotEqual(events[2]["query_sha256"], events[3]["query_sha256"])

    def test_osv_response_order_and_batch_limit(self) -> None:
        configuration = {
            "dependencies": {
                "first": {"version": "1.0", "pages": 2},
                "second": {"version": "2.0", "pages": 1},
            }
        }
        first = {"package": {"name": "first", "ecosystem": "PyPI"}, "version": "1.0"}
        second = {
            "package": {"name": "second", "ecosystem": "PyPI"},
            "version": "2.0",
        }
        body, identity = bench.osv_query_response(
            configuration, json.dumps({"queries": [second, first]}).encode()
        )
        self.assertEqual(
            json.loads(body),
            {"results": [{"vulns": []}, {"vulns": [], "next_page_token": "first:1"}]},
        )
        self.assertEqual(identity["first_query"], "second:0")
        self.assertNotIn("query_keys", identity)
        traced_body, traced_identity = bench.osv_query_response(
            dict(configuration, record_query_keys=True),
            json.dumps({"queries": [second, first]}).encode(),
        )
        self.assertEqual(traced_body, body)
        self.assertEqual(traced_identity["query_sha256"], identity["query_sha256"])
        self.assertEqual(traced_identity["query_keys"], ["second:0", "first:0"])
        for queries in ([], [first] * 1001):
            with self.assertRaisesRegex(ValueError, "between 1 and 1000"):
                bench.osv_query_response(
                    configuration, json.dumps({"queries": queries}).encode()
                )

    def test_osv_records_match_identifiers_and_support_revalidation(self) -> None:
        record = {"id": "OSV-BENCH-1", "modified": "2026-01-01T00:00:00Z"}
        self.fixtures.osv = {
            "dependencies": {
                "first": {
                    "version": "1.0",
                    "pages": 2,
                    "vulns_by_page": [[record["id"]], [record["id"]]],
                }
            },
            "vulnerabilities": {record["id"]: record},
        }
        query = {"package": {"name": "first", "ecosystem": "PyPI"}, "version": "1.0"}
        body, _ = bench.osv_query_response(
            self.fixtures.osv, json.dumps({"queries": [query]}).encode()
        )
        self.assertEqual(
            json.loads(body),
            {"results": [{"vulns": [record], "next_page_token": "first:1"}]},
        )
        route = "/v1/vulns/" + record["id"]
        server = self.server({"path_failures": {route: {"status": 503, "count": 1}}})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            connection.request("GET", route)
            response = connection.getresponse()
            self.assertEqual(response.status, 503)
            response.read()
            connection.request("GET", route)
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            self.assertEqual(response.getheader("Content-Type"), "application/json")
            etag = response.getheader("ETag")
            self.assertEqual(json.loads(response.read()), record)
            connection.request("GET", route, headers={"If-None-Match": etag})
            response = connection.getresponse()
            self.assertEqual((response.status, response.read()), (304, b""))
            connection.request("GET", "/v1/vulns/OSV-MISSING")
            response = connection.getresponse()
            self.assertEqual(response.status, 404)
            response.read()
        finally:
            connection.close()
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["start"])
        self.assertEqual([event["status"] for event in events], [503, 200, 304, 404])

    def test_osv_summary_modification_overrides_leave_records_unchanged(self) -> None:
        record = {"id": "OSV-BENCH-1", "modified": "2026-01-01T00:00:00Z"}
        configuration = {
            "dependencies": {
                "first": {
                    "version": "1.0",
                    "pages": 1,
                    "vulns_by_page": [[record["id"]]],
                }
            },
            "vulnerabilities": {record["id"]: record},
        }
        query = {"package": {"name": "first", "ecosystem": "PyPI"}, "version": "1.0"}
        for value in (None, "invalid timestamp", 123, "2026-01-02T00:00:00Z"):
            with self.subTest(modified=value):
                body, _ = bench.osv_query_response(
                    dict(configuration, summary_modified={record["id"]: value}),
                    json.dumps({"queries": [query]}).encode(),
                )
                expected = {"id": record["id"]}
                if value is not None:
                    expected["modified"] = value
                self.assertEqual(json.loads(body), {"results": [{"vulns": [expected]}]})
                self.assertEqual(configuration["vulnerabilities"][record["id"]], record)

    def test_osv_service_prefixes_retain_distinct_request_paths(self) -> None:
        record = {"id": "OSV-BENCH-1", "modified": "2026-01-01T00:00:00Z"}
        self.fixtures.osv = {
            "dependencies": {
                "first": {
                    "version": "1.0",
                    "pages": 1,
                    "vulns_by_page": [[record["id"]]],
                }
            },
            "vulnerabilities": {record["id"]: record},
        }
        left = "/left/v1/vulns/" + record["id"]
        right = "/right/v1/vulns/" + record["id"]
        server = self.server(
            {
                "osv_path_prefixes": ["/left", "/right"],
                "path_failures": {left: {"status": 503, "count": 1}},
            }
        )
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        query = {"package": {"name": "first", "ecosystem": "PyPI"}, "version": "1.0"}
        try:
            for prefix in ("left", "right"):
                connection.request(
                    "POST",
                    f"/{prefix}/v1/querybatch",
                    json.dumps({"queries": [query]}),
                    {"Content-Type": "application/json"},
                )
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(
                    json.loads(response.read()), {"results": [{"vulns": [record]}]}
                )
            for route, status in ((left, 503), (right, 200), (left, 200)):
                connection.request("GET", route)
                response = connection.getresponse()
                self.assertEqual(response.status, status)
                body = response.read()
                if status == 200:
                    self.assertEqual(json.loads(body), record)
            connection.request("GET", "/unknown/v1/vulns/" + record["id"])
            response = connection.getresponse()
            self.assertEqual(response.status, 404)
            response.read()
        finally:
            connection.close()
        server.wait_idle()
        events = sorted(server.events, key=lambda event: event["start"])
        self.assertEqual(
            [(event["path"], event["attempt"], event["status"]) for event in events],
            [
                ("/left/v1/querybatch", 1, 200),
                ("/right/v1/querybatch", 1, 200),
                (left, 1, 503),
                (right, 1, 200),
                (left, 2, 200),
                ("/unknown/v1/vulns/" + record["id"], 1, 404),
            ],
        )

    def test_artifact_alias_supports_resumption(self) -> None:
        server = self.server({})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            connection.request(
                "GET", "/releases/example.tar.gz", headers={"Range": "bytes=42-99"}
            )
            response = connection.getresponse()
            self.assertEqual(response.status, 206)
            self.assertEqual(response.read(), self.body[42:100])
            self.assertEqual(
                response.getheader("ETag"),
                f'"{hashlib.sha256(self.body).hexdigest()}"',
            )
        finally:
            connection.close()

    def test_failures_can_be_limited_to_one_path(self) -> None:
        server = self.server(
            {"path_failures": {"/flat/extra": {"status": 503, "count": 1}}}
        )
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            for expected in (503, 200):
                connection.request("GET", "/flat/extra")
                response = connection.getresponse()
                self.assertEqual(response.status, expected)
                response.read()
            connection.request(
                "GET", "/files/example.whl", headers={"Range": "bytes=0-3"}
            )
            response = connection.getresponse()
            self.assertEqual(response.status, 206)
            self.assertEqual(response.read(), self.body[:4])
        finally:
            connection.close()

    def test_head_can_omit_range_advertisement(self) -> None:
        server = self.server({"head_ranges": False})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            connection.request("HEAD", "/files/example.whl")
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            self.assertIsNone(response.getheader("Accept-Ranges"))
            response.read()
            connection.request(
                "GET", "/files/example.whl", headers={"Range": "bytes=42-99"}
            )
            response = connection.getresponse()
            self.assertEqual(response.getheader("Accept-Ranges"), "bytes")
            self.assertEqual(
                (response.status, response.read()), (206, self.body[42:100])
            )
        finally:
            connection.close()

    def test_retry_after_is_only_sent_on_injected_failures(self) -> None:
        server = self.server(
            {
                "path_failures": {
                    "/files/example.whl": {
                        "status": 503,
                        "count": 1,
                        "retry_after": "0",
                    }
                }
            }
        )
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            for status, header in [(503, "0"), (200, None)]:
                connection.request("GET", "/files/example.whl")
                response = connection.getresponse()
                self.assertEqual(response.status, status)
                self.assertEqual(response.getheader("Retry-After"), header)
                response.read()
        finally:
            connection.close()
        server.wait_idle()
        self.assertEqual(
            [event.get("retry_after") for event in server.events], ["0", None]
        )

    def test_conditional_requests_precede_ranges(self) -> None:
        server = self.server({})
        etag = f'"{hashlib.sha256(self.body).hexdigest()}"'
        for range_, validator, expected_status in [
            ("bytes=0-3", etag, 304),
            ("bytes=999999-", etag, 304),
            ("bytes=0-3", f"W/{etag}", 304),
            ("bytes=0-3", '"different"', 206),
        ]:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
            try:
                connection.request(
                    "GET",
                    "/files/example.whl",
                    headers={"Range": range_, "If-None-Match": validator},
                )
                response = connection.getresponse()
                self.assertEqual(response.status, expected_status)
                self.assertEqual(
                    response.read(), b"" if expected_status == 304 else self.body[:4]
                )
            finally:
                connection.close()

        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        try:
            connection.request(
                "GET",
                "/files/example.whl",
                headers={"Range": "bytes=0-3", "If-Range": '"different"'},
            )
            response = connection.getresponse()
            self.assertEqual((response.status, response.read()), (200, self.body))
        finally:
            connection.close()

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
        self.assertIn(b"/files/example-1.0.tar.gz#sha256=", fixtures.flat)
        self.assertNotIn(b"data-core-metadata", fixtures.flat)
        manifest.write_text(
            json.dumps(
                [
                    {
                        "filename": source.name,
                        "sha256": bench.digest(source),
                        "pep658": False,
                    }
                ]
            )
        )
        fixtures = bench.Fixtures(manifest, directory, pep658=True)
        self.assertEqual(fixtures.packages["example"][0]["core-metadata"], False)
        self.assertNotIn(b"data-core-metadata", fixtures.flat)

    def test_flat_indexes(self) -> None:
        server = self.server({})
        with urllib.request.urlopen(server.url + "/flat/one") as response:
            self.assertEqual(response.headers["Content-Type"], "text/html")
            self.assertEqual(response.read(), self.fixtures.flat)

    @unittest.skipUnless(shutil.which("git"), "Git unavailable")
    def test_smart_git_http_replay(self) -> None:
        root = Path(self.directory.name)
        repository = root / "example.git"
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_AUTHOR_NAME="uv test",
            GIT_AUTHOR_EMAIL="uv-test@example.com",
            GIT_COMMITTER_NAME="uv test",
            GIT_COMMITTER_EMAIL="uv-test@example.com",
        )

        def git(*args: str, input: bytes | None = None) -> bytes:
            return subprocess.check_output(["git", *args], input=input, env=env)

        git("init", "--bare", "--initial-branch=main", str(repository))
        tree = git("--git-dir", str(repository), "mktree", input=b"").strip().decode()
        commit = (
            git("--git-dir", str(repository), "commit-tree", tree, input=b"fixture\n")
            .strip()
            .decode()
        )
        git("--git-dir", str(repository), "update-ref", "refs/heads/main", commit)
        server = self.server({}, git_root=root)
        checkout = root / "checkout"
        git("clone", "--quiet", server.url + "/git/example.git", str(checkout))
        self.assertEqual(
            git("-C", str(checkout), "rev-parse", "HEAD").decode().strip(), commit
        )
        server.wait_idle()
        self.assertTrue(any(event["method"] == "POST" for event in server.events))
        self.assertTrue(all(event["status"] == 200 for event in server.events))
        self.assertTrue(any(event["request_bytes"] > 0 for event in server.events))
        with self.assertRaises(urllib.error.HTTPError) as raised:
            urllib.request.urlopen(
                server.url + "/git/example.git/info/refs?service=git-receive-pack"
            )
        self.assertEqual(raised.exception.code, 404)
        raised.exception.close()

    @unittest.skipUnless(shutil.which("git"), "Git unavailable")
    def test_smart_git_http_failures(self) -> None:
        root = Path(self.directory.name)
        repository = root / "example.git"
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_AUTHOR_NAME="uv test",
            GIT_AUTHOR_EMAIL="uv-test@example.com",
            GIT_COMMITTER_NAME="uv test",
            GIT_COMMITTER_EMAIL="uv-test@example.com",
        )

        def git(*args: str, input: bytes | None = None) -> bytes:
            return subprocess.check_output(["git", *args], input=input, env=env)

        git("init", "--bare", "--initial-branch=main", str(repository))
        tree = git("--git-dir", str(repository), "mktree", input=b"").strip().decode()
        commit = (
            git("--git-dir", str(repository), "commit-tree", tree, input=b"fixture\n")
            .strip()
            .decode()
        )
        git("--git-dir", str(repository), "update-ref", "refs/heads/main", commit)
        server = self.server(
            {
                "path_failures": {
                    "/git/example.git/info/refs": {"status": 403, "count": 1},
                    "/git/example.git/git-upload-pack": {"status": 503, "count": 1},
                }
            },
            git_root=root,
        )
        checkout = root / "checkout"
        for status in (403, 503):
            with (
                self.subTest(status=status),
                self.assertRaises(subprocess.CalledProcessError),
            ):
                git("clone", "--quiet", server.url + "/git/example.git", str(checkout))
        git("clone", "--quiet", server.url + "/git/example.git", str(checkout))
        self.assertEqual(
            git("-C", str(checkout), "rev-parse", "HEAD").decode().strip(), commit
        )
        server.wait_idle()
        failures = [event for event in server.events if event["status"] != 200]
        self.assertEqual([event["status"] for event in failures], [403, 503])
        self.assertEqual([event["method"] for event in failures], ["GET", "POST"])
        self.assertEqual(failures[0]["request_bytes"], 0)
        self.assertGreater(failures[1]["request_bytes"], 0)
        self.assertTrue(all(event["bytes"] == 26 for event in failures))

    def test_simple_index_alias_keeps_original_request_path(self) -> None:
        self.fixtures.simple["example"] = b'{"name":"example","files":[]}'
        alias = "/indexes/one/example/"
        server = self.server({"path_aliases": {alias: "/simple/example/"}})
        with urllib.request.urlopen(server.url + alias) as response:
            self.assertEqual(
                response.headers["Content-Type"],
                "application/vnd.pypi.simple.v1+json",
            )
            self.assertEqual(response.read(), self.fixtures.simple["example"])
        server.wait_idle()
        self.assertEqual(server.events[0]["path"], alias)

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

    def test_cancelled_request_is_recorded_before_next_trial(self) -> None:
        server = self.server({"latency_ms": 150})
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        connection.request("GET", "/files/example.whl")
        deadline = time.monotonic() + 2
        while not server.active:
            if time.monotonic() >= deadline:
                self.fail("Fixture request did not start")
            time.sleep(0.005)
        connection.close()
        server.wait_idle()
        self.assertEqual(server.active, 0)
        self.assertEqual(len(server.events), 1)
        self.assertEqual(server.events[0]["path"], "/files/example.whl")

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

    def test_date_validated_ranges(self) -> None:
        modified = "Wed, 30 Sep 2026 12:00:00 GMT"
        server = self.server(
            {
                "artifact_etag": False,
                "artifact_last_modified": modified,
                "response_date": "Wed, 30 Sep 2026 12:01:00 GMT",
            }
        )
        for validator, status, expected in [
            (modified, 206, self.body[:4]),
            ("Wed, 30 Sep 2026 11:59:59 GMT", 200, self.body),
            ('"missing"', 200, self.body),
        ]:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
            try:
                connection.request(
                    "GET",
                    "/files/example.whl",
                    headers={"Range": "bytes=0-3", "If-Range": validator},
                )
                response = connection.getresponse()
                self.assertIsNone(response.getheader("ETag"))
                self.assertEqual(response.getheader("Last-Modified"), modified)
                self.assertEqual((response.status, response.read()), (status, expected))
            finally:
                connection.close()

    def test_date_validated_resumable_oracle(self) -> None:
        modified = "Wed, 30 Sep 2026 12:00:00 GMT"
        for etag, date, resumes in [
            (False, "Wed, 30 Sep 2026 12:01:00 GMT", True),
            (False, "Wed, 30 Sep 2026 12:00:59 GMT", False),
            ("weak", "Wed, 30 Sep 2026 12:01:00 GMT", False),
        ]:
            server = self.server(
                {
                    "artifact_etag": etag,
                    "artifact_last_modified": modified,
                    "response_date": date,
                    "cut_after_bytes": 65536,
                    "cut_count": 3,
                }
            )
            loopback = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            body = bench.read_resumable(
                loopback, server.url + "/files/example.whl", len(self.body)
            )
            self.assertEqual(body, self.body)
            server.wait_idle()
            self.assertEqual(
                [event["if_range"] for event in server.events],
                [None, modified, modified, modified] if resumes else [None] * 4,
            )
            self.assertEqual(
                sum(event["bytes"] for event in server.events),
                len(self.body) if resumes else len(self.body) + 3 * 65536,
            )

    def test_raw_fixture_oracle(self) -> None:
        directory = Path(self.directory.name)
        manifest = directory / "raw-fixtures.json"
        manifest.write_text(
            json.dumps(
                [
                    {
                        "kind": "raw",
                        "filename": self.path.name,
                        "sha256": bench.digest(self.path),
                    }
                ]
            )
        )
        fixtures = bench.Fixtures(manifest, directory, pep658=True)
        output = directory / "oracle.json"
        bench.oracle(
            fixtures,
            {"cut_after_bytes": 65536, "cut_count": 3},
            [self.path.name],
            "raw-resume",
            output,
        )
        result = json.loads(output.read_text())
        self.assertEqual(result["actual_bytes"], len(self.body))
        self.assertEqual(result["required_artifact_and_index_bytes"], len(self.body))
        self.assertEqual(result["requests"], 4)

    def test_bandwidth_is_shared(self) -> None:
        server = self.server({"bytes_per_second": len(self.body) * 2})
        start = time.perf_counter()
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            responses = list(pool.map(lambda _: self.get(server), range(2)))
        elapsed = time.perf_counter() - start
        self.assertEqual(responses, [(200, self.body)] * 2)
        self.assertGreaterEqual(elapsed, 0.95)
        self.assertLess(elapsed, 3)

    def test_artifact_origins_have_independent_capabilities(self) -> None:
        directory = Path(self.directory.name)
        wheel = directory / "example-1.0-py3-none-any.whl"
        with zipfile.ZipFile(wheel, "w") as archive:
            archive.writestr(
                "example-1.0.dist-info/METADATA",
                "Metadata-Version: 2.1\nName: example\nVersion: 1.0\n",
            )
        manifest = directory / "manifest.json"
        manifest.write_text(
            json.dumps([{"filename": wheel.name, "sha256": bench.digest(wheel)}])
        )
        fixtures = bench.Fixtures(manifest, directory, pep658=False)
        replay = bench.Replay(
            fixtures,
            {
                "artifact_origins": {
                    "no-range": {
                        "filenames": [wheel.name],
                        "profile": {"ranges": False},
                    }
                }
            },
        )
        replay.start()
        try:
            indexed = json.loads(replay.main.fixtures.simple["example"])["files"][0]
            self.assertEqual(indexed["url"], replay.file_urls[wheel.name])
            self.assertEqual(
                json.loads(fixtures.simple["example"])["files"][0]["url"],
                f"/files/{wheel.name}",
            )
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            for origin, status, expected in (
                ("no-range", 200, wheel.read_bytes()),
                ("index", 206, wheel.read_bytes()[:4]),
            ):
                with opener.open(
                    urllib.request.Request(
                        replay.urls[origin] + f"/files/{wheel.name}",
                        headers={"Range": "bytes=0-3"},
                    )
                ) as response:
                    self.assertEqual(response.status, status)
                    self.assertEqual(response.read(), expected)
            replay.wait_idle()
            self.assertEqual(
                {event["origin"] for event in replay.events}, {"index", "no-range"}
            )
            self.assertEqual(
                bench.normalize_output(
                    (replay.main.url + " " + indexed["url"]).encode(),
                    {
                        "base": replay.main.url,
                        "work": directory,
                        "origin:no-range": replay.urls["no-range"],
                    },
                ),
                f"[INDEX] [ORIGIN:no-range]/files/{wheel.name}".encode(),
            )
            replay.reset()
            self.assertFalse(replay.events)
            self.assertIs(replay.main.limiter, replay.servers["no-range"].limiter)
        finally:
            replay.stop()

    def test_bandwidth_is_shared_across_artifact_origins(self) -> None:
        directory = Path(self.directory.name)
        second = directory / "second.whl"
        second.write_bytes(self.body)
        manifest = directory / "manifest.json"
        manifest.write_text(
            json.dumps(
                [
                    {"kind": "raw", "filename": path.name, "sha256": bench.digest(path)}
                    for path in (self.path, second)
                ]
            )
        )
        fixtures = bench.Fixtures(manifest, directory, pep658=False)
        replay = bench.Replay(
            fixtures,
            {
                "bytes_per_second": len(self.body) * 2,
                "artifact_origins": {"files": {"filenames": [self.path.name]}},
            },
        )
        replay.start()
        try:

            def fetch(url: str) -> bytes:
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with opener.open(url) as response:
                    return response.read()

            start = time.perf_counter()
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                responses = list(
                    pool.map(
                        fetch,
                        [
                            replay.file_urls[self.path.name],
                            replay.main.url + f"/files/{second.name}",
                        ],
                    )
                )
            elapsed = time.perf_counter() - start
            self.assertEqual(responses, [self.body] * 2)
            self.assertGreaterEqual(elapsed, 0.95)
            self.assertLess(elapsed, 3)
            replay.wait_idle()
            self.assertEqual(bench.maximum_active(replay.events), 2)
        finally:
            replay.stop()

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
            self.assertEqual(bench.network_floor({}, 1, 2, 1650), 2.05)
            self.assertEqual(bench.network_floor({}, 1, 2, 1650, 1000), 3.05)

    def test_refresh_follows_subcommand(self) -> None:
        args = SimpleNamespace(
            work_dir=Path(self.directory.name),
            directory=Path(self.directory.name),
            requirement=["example==1"],
            python="3.12",
            command=["pip", "compile", "{work}/requirements.in"],
            cache_mode="refresh",
            refresh_mode="flag",
            timeout=10,
            verify_tree=None,
            http2_proxy=None,
            git_root=None,
            templates={},
            setup_commands=[],
            env={},
            verify_file=[],
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

        args.command = ["pip", "list", "--outdated"]
        args.refresh_mode = "implicit"
        commands.clear()
        with patch.object(bench.subprocess, "run", side_effect=run):
            bench.run_one(Path("uv"), self.fixtures, {}, args)
        warm_command, timed_command = commands
        self.assertNotIn("--refresh", warm_command)
        self.assertEqual(timed_command, warm_command)

        args.expected_exit_code = 1
        completed = subprocess.CompletedProcess([], 1, b"findings\n", b"")
        commands.clear()
        with patch.object(bench.subprocess, "run", side_effect=run):
            measured = bench.run_one(Path("uv"), self.fixtures, {}, args)
        self.assertEqual(measured["exit_code"], 1)
        self.assertEqual(len(commands), 2)

        completed = subprocess.CompletedProcess([], 0, b"", b"")
        with (
            patch.object(bench.subprocess, "run", side_effect=run),
            self.assertRaisesRegex(RuntimeError, "Cache warmup returned 0"),
        ):
            bench.run_one(Path("uv"), self.fixtures, {}, args)
        args.cache_mode = "cold"
        with (
            patch.object(bench.subprocess, "run", side_effect=run),
            self.assertRaisesRegex(RuntimeError, "Command failed \\(0\\)"),
        ):
            bench.run_one(Path("uv"), self.fixtures, {}, args)

    def test_setup_and_result_file_verification(self) -> None:
        args = SimpleNamespace(
            work_dir=Path(self.directory.name),
            directory=Path(self.directory.name),
            requirement=[],
            python="3.12",
            command=["lock", "--index", "{index}"],
            cache_mode="cold",
            timeout=10,
            verify_tree=None,
            http2_proxy=None,
            git_root=None,
            templates={
                "uv.toml": 'sources = { example = { index = "fixture" } }\n',
                "pyproject.toml": '[project]\nname = "fixture"\n',
            },
            setup_commands=[["venv", "{work}/env"]],
            env={"UV_CONCURRENT_DOWNLOADS": "2"},
            verify_file=["{work}/uv.lock"],
        )
        commands = []

        def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess:
            commands.append(command.copy())
            work = kwargs["cwd"]
            self.assertEqual((work / "uv.toml").read_text(), args.templates["uv.toml"])
            self.assertEqual(kwargs["env"]["UV_CONCURRENT_DOWNLOADS"], "2")
            if getattr(args, "no_python_env", False):
                self.assertNotIn("UV_PYTHON", kwargs["env"])
            else:
                self.assertEqual(kwargs["env"]["UV_PYTHON"], "3.12")
            for key in ("VIRTUAL_ENV", "CONDA_PREFIX", "PYTHONPATH", "PYTHONHOME"):
                self.assertNotIn(key, kwargs["env"])
            if "lock" in command:
                (work / "uv.lock").write_text(command[-1])
                return subprocess.CompletedProcess([], 0, b"", command[-1].encode())
            return subprocess.CompletedProcess([], 0, b"", b"")

        with (
            patch.dict(os.environ, {"VIRTUAL_ENV": "/outer", "PYTHONPATH": "/outer"}),
            patch.object(bench.subprocess, "run", side_effect=run),
        ):
            result = bench.run_one(Path("/uv"), self.fixtures, {}, args)
        self.assertIn("venv", commands[0])
        self.assertIn("lock", commands[1])
        self.assertEqual(commands[0][1], "--config-file")
        self.assertEqual(
            result["verified_files"],
            {"{work}/uv.lock": hashlib.sha256(b"[INDEX]/simple").hexdigest()},
        )
        self.assertEqual(
            result["stderr_sha256"], hashlib.sha256(b"[INDEX]/simple").hexdigest()
        )

        args.no_python_env = True
        with (
            patch.dict(os.environ, {"UV_PYTHON": "/outer/python"}),
            patch.object(bench.subprocess, "run", side_effect=run),
        ):
            without_python = bench.run_one(Path("/uv"), self.fixtures, {}, args)
        self.assertEqual(result["verified_files"], without_python["verified_files"])

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

    def test_tree_normalization_is_limited_to_selected_files(self) -> None:
        roots = [Path(self.directory.name) / variant for variant in ("parent", "head")]
        results = []
        for root in roots:
            root.mkdir()
            (root / "_sysconfigdata_test.py").write_text(f"prefix = {str(root)!r}\n")
            (root / "module.py").write_text("answer = 42\n")
            results.append(
                bench.tree_digest(
                    root,
                    ["_sysconfigdata_*.py"],
                    {"work": root, "base": "http://127.0.0.1"},
                )
            )
        self.assertEqual(*results)
        (roots[1] / "module.py").write_text("answer = 43\n")
        self.assertNotEqual(
            results[0],
            bench.tree_digest(
                roots[1],
                ["_sysconfigdata_*.py"],
                {"work": roots[1], "base": "http://127.0.0.1"},
            ),
        )

    def test_tree_normalization_updates_and_checks_recorded_hashes(self) -> None:
        roots = [Path(self.directory.name) / variant for variant in ("parent", "head")]

        def install(root: Path, port: int, *, extra: str = "") -> None:
            metadata = root / "package-1.0.dist-info"
            metadata.mkdir(exist_ok=True, parents=True)
            contents = f'{{"url":"http://127.0.0.1:{port}/wheel{extra}"}}'.encode()
            (metadata / "direct_url.json").write_bytes(contents)
            (root / "module.py").write_text("answer = 42\n")
            encoded = base64.urlsafe_b64encode(hashlib.sha256(contents).digest())
            with (metadata / "RECORD").open("w", newline="") as record:
                csv.writer(record, lineterminator="\n").writerows(
                    [
                        [
                            "package-1.0.dist-info/direct_url.json",
                            f"sha256={encoded.rstrip(b'=').decode()}",
                            str(len(contents)),
                        ],
                        ["package-1.0.dist-info/RECORD", "", ""],
                    ]
                )

        def normalized(root: Path, port: int) -> dict:
            return bench.tree_digest(
                root,
                ["**/direct_url.json"],
                {"work": root, "base": f"http://127.0.0.1:{port}"},
            )

        install(roots[0], 1234)
        install(roots[1], 56789)
        self.assertNotEqual(bench.tree_digest(roots[0]), bench.tree_digest(roots[1]))
        self.assertEqual(normalized(roots[0], 1234), normalized(roots[1], 56789))
        (roots[1] / "module.py").write_text("answer = 43\n")
        self.assertNotEqual(normalized(roots[0], 1234), normalized(roots[1], 56789))
        install(roots[1], 56789, extra="-changed")
        self.assertNotEqual(normalized(roots[0], 1234), normalized(roots[1], 56789))
        metadata = roots[1] / "package-1.0.dist-info"
        (metadata / "direct_url.json").write_text("tampered")
        with self.assertRaisesRegex(ValueError, "RECORD hash does not match"):
            normalized(roots[1], 56789)
        install(roots[1], 56789)
        record = metadata / "RECORD"
        rows = list(csv.reader(io.StringIO(record.read_text())))
        rows[0][2] = str(int(rows[0][2]) + 1)
        with record.open("w", newline="") as file:
            csv.writer(file, lineterminator="\n").writerows(rows)
        with self.assertRaisesRegex(ValueError, "RECORD size does not match"):
            normalized(roots[1], 56789)

    def test_tree_symlink_normalization_is_limited_to_selected_trial_targets(
        self,
    ) -> None:
        roots = [Path(self.directory.name) / variant for variant in ("parent", "head")]
        for root in roots:
            root.mkdir()
            (root / "python3.12").write_bytes(b"interpreter")
            (root / "python").symlink_to(root / "python3.12")

        def normalized(root: Path) -> dict:
            return bench.tree_digest(
                root, context={"work": root}, normalized_symlinks=["python"]
            )

        self.assertNotEqual(bench.tree_digest(roots[0]), bench.tree_digest(roots[1]))
        self.assertEqual(normalized(roots[0]), normalized(roots[1]))
        for root in roots:
            (root / "other").symlink_to(root / "python3.12")
        self.assertNotEqual(normalized(roots[0]), normalized(roots[1]))
        for root in roots:
            (root / "other").unlink()
        (roots[1] / "python").unlink()
        (roots[1] / "python").symlink_to("python3.12")
        self.assertNotEqual(normalized(roots[0]), normalized(roots[1]))
        (roots[1] / "python").unlink()
        (roots[1] / "python").symlink_to(roots[1] / ".." / "python3.12")
        self.assertNotEqual(normalized(roots[0]), normalized(roots[1]))


if __name__ == "__main__":
    unittest.main()
