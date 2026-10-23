"""Check protocol-sensitive byte bounds for selected wheel metadata."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path


def module(name: str):
    spec = importlib.util.spec_from_file_location(
        name, Path(__file__).with_name(name + ".py")
    )
    assert spec is not None and spec.loader is not None
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


oracle = module("source_graph_oracle")
scheduling = module("make_scheduling_fixtures")


class MetadataBytesTests(unittest.TestCase):
    def test_wheel_ranges_and_streaming_prefix(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            wheel = scheduling.wheel(
                directory, "uv-bench-choice", 2, None, payload_bytes=65536
            )
            wheel["pep658"] = False
            manifest = directory / "fixtures.json"
            manifest.write_text(json.dumps([wheel]))
            fixtures = oracle.bench.Fixtures(manifest, directory, True)
            entry = fixtures.packages["uv-bench-choice"][0]
            with zipfile.ZipFile(directory / wheel["filename"]) as archive:
                metadata = archive.read("uv_bench_choice-2.0.dist-info/METADATA")
            ranged = oracle.metadata_bytes(fixtures, {"ranges": True}, entry)
            streamed = oracle.metadata_bytes(fixtures, {"ranges": False}, entry)
            self.assertEqual(ranged, len(metadata))
            self.assertGreater(streamed, 65536)
            self.assertLess(streamed, wheel["size"])

    def test_sidecar_does_not_require_the_archive_prefix(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            wheel = scheduling.wheel(
                directory, "uv-bench-choice", 2, None, payload_bytes=65536
            )
            manifest = directory / "fixtures.json"
            manifest.write_text(json.dumps([wheel]))
            fixtures = oracle.bench.Fixtures(manifest, directory, True)
            entry = fixtures.packages["uv-bench-choice"][0]
            self.assertEqual(
                oracle.metadata_bytes(fixtures, {"ranges": False}, entry),
                len(fixtures.metadata[wheel["filename"] + ".metadata"]),
            )


if __name__ == "__main__":
    unittest.main()
