import http.client
import http.server
import importlib.util
import json
import ssl
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path
from typing import ClassVar

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "cache_proxy", ROOT / ".github/actions/cache-proxy/proxy.py"
)
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


class Upstream(http.server.BaseHTTPRequestHandler):
    requests: ClassVar[list] = []

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.requests.append((self.path, self.headers.get("Authorization"), body))
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class ProxyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(dir=ROOT)
        directory = Path(cls.scratch.name)
        cls.cert, key = directory / "cert.pem", directory / "key.pem"
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
                "/CN=example.invalid",
                "-addext",
                "subjectAltName=DNS:example.invalid",
                "-keyout",
                str(key),
                "-out",
                str(cls.cert),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cls.cert, key)
        cls.upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        cls.upstream.socket = tls.wrap_socket(cls.upstream.socket, server_side=True)
        cls.trust = ssl.create_default_context(cafile=str(cls.cert))
        cls.server = proxy.ProxyServer(
            ("127.0.0.1", 0),
            {
                "example.invalid": {
                    "addresses": ["127.0.0.1"],
                    "port": cls.upstream.server_port,
                }
            },
            context=cls.trust,
        )
        cls.server.socket = tls.wrap_socket(cls.server.socket, server_side=True)
        for server in (cls.upstream, cls.server):
            threading.Thread(target=server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        for server in (cls.server, cls.upstream):
            server.shutdown()
            server.server_close()
        cls.scratch.cleanup()

    def request(self, method, path, body=b"{}", host="example.invalid"):
        connection = proxy.PinnedConnection(
            "example.invalid", "127.0.0.1", self.server.server_port, self.trust
        )
        connection.request(
            method,
            path,
            body=body,
            headers={"Host": host, "Authorization": "Bearer synthetic-test-only"},
        )
        response = connection.getresponse()
        result = (
            response.status,
            response.getheader("X-UV-Cache-Proxy"),
            response.read(),
        )
        connection.close()
        return result

    def test_cache_v2_read_denied(self):
        status, marker, body = self.request(
            "POST",
            "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL",
        )
        self.assertEqual((status, marker), (403, "denied"))
        self.assertEqual(json.loads(body)["code"], "permission_denied")

    def test_cache_v2_write_denied(self):
        status, _, body = self.request(
            "POST", "/twirp/github.actions.results.api.v1.CacheService/CreateCacheEntry"
        )
        self.assertEqual(status, 403)
        self.assertEqual(json.loads(body)["msg"], "cache write denied: uv cache proxy")

    def test_legacy_cache_denied(self):
        self.assertEqual(
            self.request(
                "GET", "/some-prefix/_apis/artifactcache/cache?keys=synthetic"
            )[0],
            403,
        )

    def test_encoded_cache_route_denied(self):
        self.assertEqual(self.request("GET", "/_apis%2Fartifactcache/cache")[0], 403)

    def test_artifact_forwarded_without_changing_body(self):
        path = "/twirp/github.actions.results.api.v1.ArtifactService/CreateArtifact"
        body = b'{"synthetic": true}'
        self.assertEqual(self.request("POST", path, body), (200, None, body))
        self.assertEqual(
            Upstream.requests[-1], (path, "Bearer synthetic-test-only", body)
        )

    def test_unconfigured_host_rejected(self):
        self.assertEqual(
            self.request("POST", "/anything", host="untrusted.invalid")[0], 421
        )


if __name__ == "__main__":
    unittest.main()
