"""Exercise crates.io preflight retry behavior without publishing packages."""

import importlib.util
import ssl
import sys
import threading
import unittest
import urllib.error
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from unittest.mock import MagicMock, call, patch
from urllib.request import ProxyHandler, build_opener

SPEC = importlib.util.spec_from_file_location(
    "publish_crates", Path(__file__).with_name("publish-crates.py")
)
assert SPEC is not None and SPEC.loader is not None
PUBLISHER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PUBLISHER
SPEC.loader.exec_module(PUBLISHER)
CRATE = PUBLISHER.Crate("uv", "0.12.23")
URL = PUBLISHER.crate_version_url(CRATE, PUBLISHER.CRATES_IO_API)


def response(status=200):
    result = MagicMock()
    result.__enter__.return_value.status = status
    return result


def http_error(status, retry_after=None):
    return urllib.error.HTTPError(
        URL, status, "fixture", {"Retry-After": retry_after}, None
    )


class CratePreflight(unittest.TestCase):
    def setUp(self):
        self.opener_patch = patch.object(PUBLISHER.urllib.request, "urlopen")
        self.sleep_patch = patch.object(PUBLISHER.time, "sleep")
        self.opener = self.opener_patch.start()
        self.sleep = self.sleep_patch.start()
        self.addCleanup(self.opener_patch.stop)
        self.addCleanup(self.sleep_patch.stop)

    def exists(self):
        return PUBLISHER.crate_version_exists(CRATE, PUBLISHER.CRATES_IO_API)

    def test_present_and_absent_versions_are_not_retried(self):
        self.opener.return_value = response()
        self.assertTrue(self.exists())
        self.assertEqual(self.opener.call_args.kwargs, {"timeout": 15})
        self.opener.side_effect = http_error(404)
        self.assertFalse(self.exists())
        self.sleep.assert_not_called()

    def test_rate_limit_honors_bounded_retry_after(self):
        for header, delay in (("12", 12), ("1000", 30), ("invalid", 1)):
            with self.subTest(header=header):
                self.sleep.reset_mock()
                self.opener.side_effect = [http_error(429, header), response()]
                self.assertTrue(self.exists())
                self.sleep.assert_called_once_with(delay)

    def test_http_date_retry_after(self):
        self.opener.side_effect = [
            http_error(503, "Thu, 01 Jan 1970 00:00:05 GMT"),
            response(),
        ]
        with patch.object(PUBLISHER.time, "time", return_value=0):
            self.assertTrue(self.exists())
        self.sleep.assert_called_once_with(5)

    def test_transient_errors_use_backoff(self):
        self.opener.side_effect = [
            http_error(502),
            urllib.error.URLError("connection reset"),
            TimeoutError("timed out"),
            response(),
        ]
        self.assertTrue(self.exists())
        self.assertEqual(self.sleep.call_args_list, [call(1), call(2), call(4)])
        self.assertTrue(
            all(args.kwargs == {"timeout": 15} for args in self.opener.call_args_list)
        )

    def test_direct_connection_reset_is_retried(self):
        self.opener.side_effect = [ConnectionResetError("connection reset"), response()]
        self.assertTrue(self.exists())
        self.sleep.assert_called_once_with(1)
        self.assertEqual(self.opener.call_count, 2)

    def test_server_disconnect_before_headers_is_retried(self):
        requests = []

        class DisconnectOnce(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                if len(requests) == 1:
                    self.close_connection = True
                    return
                self.send_response(200)
                self.send_header("Content-Length", "0")
                self.end_headers()

            def log_message(self, *arguments):
                pass

        # Exercise urllib's real HTTP stack without inheriting proxy settings.
        self.opener.side_effect = build_opener(ProxyHandler({})).open
        with HTTPServer(("127.0.0.1", 0), DisconnectOnce) as server:
            thread = threading.Thread(
                target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
            )
            thread.start()
            try:
                api_url = f"http://127.0.0.1:{server.server_port}"
                self.assertTrue(PUBLISHER.crate_version_exists(CRATE, api_url))
            finally:
                server.shutdown()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())
        self.assertEqual(requests, ["/crates/uv/0.12.23"] * 2)
        self.assertEqual(self.opener.call_count, 2)
        self.sleep.assert_called_once_with(1)

    def test_permanent_http_errors_fail_without_retry(self):
        for status in (400, 401, 403, 501):
            with self.subTest(status=status):
                self.opener.side_effect = http_error(status)
                with self.assertRaisesRegex(RuntimeError, f"HTTP {status}"):
                    self.exists()
                self.sleep.assert_not_called()

    def test_certificate_errors_fail_without_retry(self):
        self.opener.side_effect = urllib.error.URLError(
            ssl.SSLCertVerificationError("fixture")
        )
        with self.assertRaises(RuntimeError):
            self.exists()
        self.assertEqual(self.opener.call_count, 1)
        self.sleep.assert_not_called()

    def test_exhausted_preflight_never_invokes_cargo_publish(self):
        self.opener.side_effect = http_error(503)
        with patch.object(PUBLISHER.subprocess, "run") as cargo:
            with self.assertRaisesRegex(RuntimeError, "uv@0.12.23.*4 attempts"):
                PUBLISHER.publish_workspace(
                    ["cargo"], [CRATE], PUBLISHER.CRATES_IO_API, []
                )
            cargo.assert_not_called()
        self.assertEqual(self.opener.call_count, 4)
        self.assertEqual(self.sleep.call_count, 3)

    def test_unexpected_success_status_fails(self):
        self.opener.return_value = response(204)
        with self.assertRaisesRegex(RuntimeError, "unexpected status 204"):
            self.exists()
        self.sleep.assert_not_called()

    def test_invalid_retry_after_falls_back(self):
        for value in (None, "", "NaN", "inf", "not a date"):
            with self.subTest(value=value):
                self.assertIsNone(PUBLISHER.retry_after_seconds(value))
        self.assertEqual(PUBLISHER.retry_after_seconds("-5"), 0)


if __name__ == "__main__":
    unittest.main()
