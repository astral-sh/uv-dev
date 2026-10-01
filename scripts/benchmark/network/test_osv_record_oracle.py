"""Check the OSV page and record reference graph and optimistic bounds."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "osv_record_oracle", Path(__file__).with_name("osv_record_oracle.py")
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class OsvRecordOracleTests(unittest.TestCase):
    def setUp(self) -> None:
        dependencies = {
            f"package-{index:04d}": {
                "version": "1.0",
                "pages": 1,
                "vulns_by_page": [[]],
            }
            for index in range(1001)
        }
        dependencies["package-0000"]["vulns_by_page"] = [["SHARED"]]
        dependencies["package-1000"].update(
            pages=2, vulns_by_page=[["SHARED"], ["NEXT"]]
        )
        self.configuration = {
            "record_query_keys": True,
            "dependencies": dependencies,
            "vulnerabilities": {
                identifier: {"id": identifier, "modified": "2026-01-01T00:00:00Z"}
                for identifier in ("SHARED", "NEXT")
            },
        }

    def test_records_need_any_identifying_page_and_are_deduplicated(self) -> None:
        queries, records = oracle.requests(self.configuration, 1001, False)
        self.assertEqual([task["parents"] for task in queries], [set(), set(), {1}])
        self.assertEqual(
            {task["path"]: task["any_parent"] for task in records},
            {"/v1/vulns/NEXT": {2}, "/v1/vulns/SHARED": {0, 1}},
        )
        self.assertEqual(oracle.requests(self.configuration, 1001, True), (queries, []))

    def test_floor_includes_record_chain_and_body_bytes(self) -> None:
        profile = {"latency_ms": 100, "path_latency_ms": {"/v1/vulns/NEXT": 600}}
        cold_bytes, waves, latency = oracle.lower_bound(
            self.configuration, 1001, 2, profile, False
        )
        warm_bytes, warm_waves, warm_latency = oracle.lower_bound(
            self.configuration, 1001, 2, profile, True
        )
        self.assertEqual((waves, latency), (3, 300))
        self.assertEqual((warm_waves, warm_latency), (2, 200))
        self.assertEqual(
            cold_bytes - warm_bytes,
            sum(
                len(oracle.compact(record))
                for record in self.configuration["vulnerabilities"].values()
            ),
        )
        self.assertEqual(
            oracle.lower_bound(self.configuration, 1001, 1, profile, False)[1:],
            (4, 400),
        )
        with self.assertRaisesRegex(ValueError, "Invalid package count"):
            oracle.lower_bound(self.configuration, 1002, 1, profile, False)


if __name__ == "__main__":
    unittest.main()
