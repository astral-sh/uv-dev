"""Source archives are replayed without changing package-index fixtures."""

import hashlib
import importlib.util
import json
import tempfile
import threading
import unittest
import urllib.request
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "serve_fixtures",
    Path(__file__).resolve().parents[1] / "benchmark/serve-fixtures.py",
)
assert SPEC is not None and SPEC.loader is not None
fixture_server = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture_server)


class SourceArchiveFixtures(unittest.TestCase):
    def test_archive_body_does_not_change_indexes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            body = b"source archive contents\n"
            filename = "project-1.0.tar.gz"
            (root / filename).write_bytes(body)
            manifest = root / "fixtures.json"
            manifest.write_text(
                json.dumps(
                    [{"filename": filename, "sha256": hashlib.sha256(body).hexdigest()}]
                )
            )
            fixtures = fixture_server.Fixtures(root, manifest, [], None)
            self.assertEqual(fixtures.simple, {})
            self.assertNotIn(filename.encode(), fixtures.flat)
            with fixture_server.Server(fixtures, 0, False) as server:
                thread = threading.Thread(target=server.serve_forever)
                thread.start()
                try:
                    opener = urllib.request.build_opener(
                        urllib.request.ProxyHandler({})
                    )
                    url = f"http://127.0.0.1:{server.server_port}/files/{filename}"
                    with opener.open(url) as response:
                        self.assertEqual(response.read(), body)
                    request = urllib.request.Request(
                        url, headers={"Range": "bytes=7-13"}
                    )
                    with opener.open(request) as response:
                        self.assertEqual(response.status, 206)
                        self.assertEqual(response.read(), body[7:14])
                finally:
                    server.shutdown()
                    thread.join()


if __name__ == "__main__":
    unittest.main()
