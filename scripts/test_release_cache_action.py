"""Exercise the Python action hooks without changing the host's network policy."""

from __future__ import annotations

import contextlib
import http.server
import importlib.util
import io
import json
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
ACTION = ROOT / ".github/actions/disable-github-caches"
spec = importlib.util.spec_from_file_location("cache_action", ACTION / "action.py")
action = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = action
spec.loader.exec_module(action)

FAKE_INSTALLER = """
import json
import os
import shutil
import sys
from pathlib import Path
directory = Path(os.environ["FAKE_DIRECTORY"])
operation = sys.argv[2]
with (directory / "operations").open("a") as output:
    output.write(operation + "\\n")
if operation == "install":
    shutil.copyfile(sys.argv[3], directory / "origins.json")
    (directory / "audit.json").write_text(json.dumps({"cache_read_denied": 2}))
"""


class CacheService(http.server.BaseHTTPRequestHandler):
    requests: ClassVar[list] = []
    cache_status = 403
    cache_marker = "denied"
    health_status = 200

    def handle_request(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.requests.append(
            (self.command, self.path, self.headers.get("Authorization"), body)
        )
        health = self.path == "/__uv_cache_proxy_health"
        self.send_response(self.health_status if health else self.cache_status)
        if not health and self.cache_marker:
            self.send_header("X-UV-Cache-Proxy", self.cache_marker)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"{}")

    do_GET = handle_request
    do_POST = handle_request

    def log_message(self, *args):
        pass


class ActionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(dir=ROOT)
        directory = Path(cls.scratch.name)
        cls.certificate, key = directory / "cert.pem", directory / "key.pem"
        names = [
            urlsplit(url).hostname for url in (action.CACHE_URL, action.RESULTS_URL)
        ]
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                f"/CN={names[0]}",
                "-addext",
                "subjectAltName=" + ",".join(f"DNS:{name}" for name in names),
                "-keyout",
                str(key),
                "-out",
                str(cls.certificate),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cls.certificate, key)
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), CacheService)
        cls.server.socket = tls.wrap_socket(cls.server.socket, server_side=True)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.getaddrinfo = staticmethod(socket.getaddrinfo)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.scratch.cleanup()

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=ROOT)
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.environment = self.directory / "environment"
        self.state = self.directory / "state"
        self.environment.touch()
        self.state.touch()
        installer = self.directory / "installer.py"
        installer.write_text(FAKE_INSTALLER)
        self.runner = action.Runner(
            self.directory,
            self.certificate,
            (sys.executable, str(installer)),
            "install.py",
        )
        CacheService.requests = []
        CacheService.cache_status = 403
        CacheService.cache_marker = "denied"
        CacheService.health_status = 200
        environment = {
            "INPUT_ENABLED": "true",
            "ACTIONS_CACHE_URL": action.CACHE_URL + "/prefix/",
            "ACTIONS_RESULTS_URL": action.RESULTS_URL,
            "ACTIONS_RUNTIME_TOKEN": "synthetic-test-only",
            "RUNNER_TEMP": str(self.directory),
            "GITHUB_ENV": str(self.environment),
            "GITHUB_STATE": str(self.state),
            "FAKE_DIRECTORY": str(self.directory),
            "no_proxy": "existing.invalid",
        }
        for patch in (
            mock.patch.dict(os.environ, environment, clear=True),
            mock.patch.object(action, "runner_platform", return_value=self.runner),
            mock.patch.object(action.socket, "getaddrinfo", side_effect=self.resolve),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def resolve(self, hostname, port, *args, **kwargs):
        if hostname in (
            urlsplit(action.CACHE_URL).hostname,
            urlsplit(action.RESULTS_URL).hostname,
        ):
            return self.getaddrinfo(
                "127.0.0.1", self.server.server_port, *args, **kwargs
            )
        return self.getaddrinfo(hostname, port, *args, **kwargs)

    def operations(self):
        return (self.directory / "operations").read_text().splitlines()

    def test_hook_lifecycle(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            action.pre()
        protection = "DNS interception"
        if sys.platform != "win32":
            protection += " and direct-address filtering"
        self.assertEqual(
            output.getvalue(), f"GitHub cache API denial is active ({protection}).\n"
        )
        self.assertEqual(self.operations(), ["install"])
        origins = json.loads((self.directory / "origins.json").read_text())
        self.assertEqual(
            origins,
            {
                urlsplit(url).hostname: {"addresses": ["127.0.0.1"]}
                for url in (action.CACHE_URL, action.RESULTS_URL)
            },
        )
        exports = dict(
            line.split("=", 1) for line in self.environment.read_text().splitlines()
        )
        bypass = ",".join(["existing.invalid", *origins])
        self.assertEqual(
            exports,
            {
                "NODE_EXTRA_CA_CERTS": str(self.certificate),
                "UV_CACHE_PROXY_ACTIVE": "1",
                "UV_CACHE_PROXY_CONFIG": str(self.directory / "origins.json"),
                "no_proxy": bypass,
                "NO_PROXY": bypass,
            },
        )
        self.assertEqual(self.state.read_text(), "installed=true\n")
        health, current, legacy = CacheService.requests
        self.assertEqual(health, ("GET", "/__uv_cache_proxy_health", None, b""))
        self.assertEqual(
            current[:3], ("POST", action.CACHE_READ, "Bearer synthetic-test-only")
        )
        payload = json.loads(current[3])
        self.assertEqual(payload["restore_keys"], [])
        self.assertTrue(payload["key"].startswith("uv-release-cache-check-"))
        self.assertEqual(len(payload["version"]), 64)
        self.assertEqual(urlsplit(legacy[1]).path, "/prefix/_apis/artifactcache/cache")
        self.assertEqual(
            parse_qs(urlsplit(legacy[1]).query),
            {
                "keys": [payload["key"]],
                "version": [payload["version"]],
            },
        )
        self.assertEqual(
            (legacy[0], legacy[2], legacy[3]),
            ("GET", "Bearer synthetic-test-only", b""),
        )
        with mock.patch.dict(os.environ, {**exports, "STATE_installed": "true"}):
            action.main()
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                action.post()
        self.assertEqual(self.operations(), ["install", "cleanup"])
        self.assertEqual(
            output.getvalue(),
            "Cache proxy removed. Denied 2 read requests and 0 write requests.\n",
        )

    def test_denial_must_have_status_and_proxy_marker(self):
        for status, marker in ((200, "denied"), (403, None)):
            with self.subTest(status=status, marker=marker):
                CacheService.cache_status, CacheService.cache_marker = status, marker
                with self.assertRaisesRegex(
                    action.ActionError, "^Cache v2 denial self-test failed$"
                ):
                    action.pre()
                self.assertEqual(self.environment.read_text(), "")
                self.assertEqual(self.state.read_text(), "")
        self.assertEqual(
            self.operations(), ["install", "cleanup", "install", "cleanup"]
        )

    def test_failed_health_check_cleans_up(self):
        CacheService.health_status = 503
        with (
            mock.patch.object(action.time, "sleep"),
            self.assertRaisesRegex(
                action.ActionError, "^Cache proxy failed its health check$"
            ),
        ):
            action.pre()
        self.assertEqual(self.operations(), ["install", "cleanup"])
        self.assertEqual(len(CacheService.requests), 20)
        self.assertEqual(self.state.read_text(), "")

    def test_invalid_audit_still_cleans_up(self):
        (self.directory / "audit.json").write_text("invalid json")
        with (
            mock.patch.dict(os.environ, {"STATE_installed": "true"}),
            self.assertRaises(json.JSONDecodeError),
        ):
            action.post()
        self.assertEqual(self.operations(), ["cleanup"])

    def test_depot_origins(self):
        with mock.patch.object(action.sys, "platform", "linux"):
            origins = action.origins_for(
                ["http://10.1.2.3:977/", "http://10.1.2.3:978/"]
            )
        self.assertEqual(
            origins,
            {
                f"10.1.2.3:{port}": {
                    "scheme": "http",
                    "port": port,
                    "listen_port": port + 19000,
                    "forward_origin": urlsplit(url).hostname,
                    "addresses": ["10.1.2.3"],
                }
                for port, url in ((977, action.CACHE_URL), (978, action.RESULTS_URL))
            },
        )
        with (
            mock.patch.object(action.sys, "platform", "darwin"),
            self.assertRaisesRegex(
                action.ActionError, "^Private cache endpoints require Linux$"
            ),
        ):
            action.origins_for(["http://10.1.2.3:978/"])

    def test_unexpected_endpoints_are_rejected(self):
        for url in (
            "https://example.invalid/",
            action.CACHE_URL + ":444/",
            "https://user:secret@artifactcache.actions.githubusercontent.com/",
            action.CACHE_URL + "\n/",
            action.CACHE_URL + "#fragment",
            "http://127.0.0.1:978/",
            "http://10.1.2.3:443/",
            "http://[::1]:978/",
            "http://10.example.invalid:978/",
        ):
            with (
                self.subTest(url=url),
                self.assertRaisesRegex(
                    action.ActionError, "^Unexpected cache service endpoint$"
                ),
            ):
                action.service_url(url)


class LauncherTests(unittest.TestCase):
    def test_unexpected_exception_is_sanitized(self):
        result = subprocess.run(
            [sys.executable, str(ACTION / "action.py"), "synthetic-secret"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (1, "", "::error::Cache-denial action failed\n"),
        )

    def invoke(self, hook, environment):
        return subprocess.run(
            [shutil.which("node") or "node", str(ACTION / f"{hook}.cjs")],
            env={
                **os.environ,
                "STATE_installed": "",
                "UV_CACHE_PROXY_ACTIVE": "",
                **environment,
            },
            capture_output=True,
            text=True,
            check=False,
        )

    def test_disabled_hooks_do_nothing(self):
        for hook in ("pre", "main", "post"):
            with self.subTest(hook=hook):
                result = self.invoke(hook, {"INPUT_ENABLED": "false"})
                self.assertEqual(
                    (result.returncode, result.stdout, result.stderr), (0, "", "")
                )

    def test_python_failure_reaches_runner(self):
        result = self.invoke("main", {"INPUT_ENABLED": "true"})
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (
                1,
                "",
                "::error::Cache-denial pre hook did not complete\n",
            ),
        )

    def test_invalid_input_is_sanitized(self):
        result = self.invoke("pre", {"INPUT_ENABLED": "synthetic-secret"})
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (
                1,
                "",
                "::error::Invalid enabled input\n",
            ),
        )


if __name__ == "__main__":
    unittest.main()
