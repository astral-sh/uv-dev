"""Exercise strict URL matching without changing the host network."""

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

from url_policy import URLPolicy, load, profile_config


def policy(url="https://allowed.example/path", methods=None, **settings):
    return URLPolicy.from_dict(
        {
            "rules": [
                {
                    "url": url,
                    "methods": methods if methods is not None else ["GET"],
                    **settings,
                }
            ]
        }
    )


class URLPolicyTests(unittest.TestCase):
    def test_default_deny_and_exact_authority(self):
        allowed = policy()
        self.assertEqual(allowed.hosts, ("allowed.example",))
        self.assertTrue(
            allowed.permits("https", "allowed.example", 443, "GET", "/path")
        )
        for arguments in (
            ("http", "allowed.example", 80, "GET", "/path"),
            ("https", "other.example", 443, "GET", "/path"),
            ("https", "allowed.example.attacker.test", 443, "GET", "/path"),
            ("https", "allowed.example", 80, "GET", "/path"),
            ("https", "allowed.example", "443", "GET", "/path"),
            ("https", "allowed.example", True, "GET", "/path"),
            ("https", "allowed.example", 443, "POST", "/path"),
            ("https", "allowed.example", 443, "get", "/path"),
            ("https", "allowed.example", 443, "CONNECT", "/path"),
            ("https", "allowed.example", 443, "GET", "/path/"),
            ("https", "allowed.example", 443, "GET", "/other"),
            ("https", "allowed.example", 443, "GET", "https://allowed.example/path"),
        ):
            with self.subTest(arguments=arguments):
                self.assertFalse(allowed.permits(*arguments))
        empty = URLPolicy.from_dict({"rules": []})
        self.assertEqual(empty.hosts, ())
        self.assertFalse(empty.permits("https", "allowed.example", 443, "GET", "/path"))

    def test_default_ports_and_canonical_hosts(self):
        allowed = URLPolicy.from_dict(
            {
                "rules": [
                    {"url": "HTTPS://ALLOWED.EXAMPLE.:443/path", "methods": ["GET"]},
                    {"url": "http://other.example:80", "methods": ["HEAD"]},
                    {"url": "https://allowed.example/other", "methods": ["POST"]},
                ]
            }
        )
        self.assertEqual(allowed.hosts, ("allowed.example", "other.example"))
        self.assertTrue(
            allowed.permits("https", "ALLOWED.EXAMPLE.", 443, "GET", "/path")
        )
        self.assertTrue(allowed.permits("http", "other.example", 80, "HEAD", "/"))
        self.assertEqual(URLPolicy.from_dict(allowed.to_dict()), allowed)

    def test_query_and_method_are_exact(self):
        allowed = policy(methods=["GET", "HEAD"])
        for target in ("/path?", "/path?x=1", "/path?x=1&x=2"):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )
        allowed = policy("https://allowed.example/path?x=%41&y=2")
        self.assertTrue(
            allowed.permits("https", "allowed.example", 443, "GET", "/path?x=%41&y=2")
        )
        for target in (
            "/path?x=A&y=2",
            "/path?y=2&x=%41",
            "/path?x=%41&y=2&z=3",
            "/path?x=%41&y=2?",
            "/path",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )
        bare = policy("https://allowed.example/path?")
        self.assertTrue(bare.permits("https", "allowed.example", 443, "GET", "/path?"))
        self.assertFalse(bare.permits("https", "allowed.example", 443, "GET", "/path"))

    def test_prefix_has_a_segment_boundary(self):
        allowed = policy("https://allowed.example/packages/", match="prefix")
        for target in ("/packages/", "/packages/file", "/packages/sub/file"):
            with self.subTest(target=target):
                self.assertTrue(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )
        for target in (
            "/packages",
            "/packages-other/file",
            "/other/packages/file",
            "/packages/file?download=1",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )

    def test_query_any_is_explicit_and_preserves_signed_data(self):
        allowed = policy(
            "https://allowed.example/packages/", match="prefix", query="any"
        )
        for target in (
            "/packages/",
            "/packages/file?",
            "/packages/file?sig=a%2Fb%2Bc%3D&se=2026-09-02T00%3A00%3A00Z",
            "/packages/file?percent=%25&space=%20&path=/a/../b",
            "/packages/file?value=one;two&space=%20",
        ):
            with self.subTest(target=target):
                self.assertTrue(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )
        for target in (
            "/packages/file?x=%",
            "/packages/file?x=%0a",
            "/packages/file?x=%80",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )

    def test_paths_are_not_decoded_before_matching(self):
        allowed = policy("https://allowed.example/%41")
        self.assertTrue(allowed.permits("https", "allowed.example", 443, "GET", "/%41"))
        self.assertFalse(allowed.permits("https", "allowed.example", 443, "GET", "/A"))
        self.assertFalse(
            allowed.permits("https", "allowed.example", 443, "GET", "/%61")
        )

    def test_ambiguous_paths_and_queries_are_rejected(self):
        allowed = policy("https://allowed.example/", match="prefix", query="any")
        self.assertFalse(allowed.permits("https", "allowed.example", 443, "GET", ""))
        for target in (
            "*",
            "/./private",
            "/a/../private",
            "/a/.",
            "/a/..",
            "/allowed/..;x/private",
            "/allowed/..%20/private",
            "/allowed/file;parameter",
            "/allowed/encoded%20space",
            "//private",
            "/a//private",
            "/a\\private",
            "/a#fragment",
            "/a#",
            "/a?value=\\private",
            "/a?value=#fragment",
            "/a?value=%0d%0a",
            "/a?value=%7f",
            "/a?value=%c3%a9",
            "/a%",
            "/a%1",
            "/a%GG",
            "/a%2fprivate",
            "/a%2Fprivate",
            "/a%5cprivate",
            "/a/%2e%2e/private",
            "/a/%252e%252e/private",
            "/a%3fprivate",
            "/a%23private",
            "/a%3bprivate",
            "/a%00private",
            "/a%7fprivate",
            "/a%c3%a9",
            "/a\r\nprivate",
            "/a\tprivate",
            "/a private",
            "/café",
            "/a?query=two words",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", target)
                )
                with self.assertRaises(ValueError):
                    policy("https://allowed.example" + target)

    def test_rule_schema_is_strict(self):
        valid = {"url": "https://allowed.example/path", "methods": ["GET"]}
        for rule in (
            None,
            [],
            {},
            {"url": valid["url"]},
            {"methods": ["GET"]},
            {**valid, "extra": True},
            {**valid, "methods": []},
            {**valid, "methods": "GET"},
            {**valid, "methods": ["get"]},
            {**valid, "methods": ["TRACE"]},
            {**valid, "methods": ["CONNECT"]},
            {**valid, "methods": ["GET", "GET"]},
            {**valid, "methods": [None]},
            {**valid, "match": "glob"},
            {**valid, "match": []},
            {**valid, "match": "prefix"},
            {**valid, "query": "ignore"},
            {**valid, "url": "https://allowed.example/path?q=1", "query": "any"},
            {**valid, "url": None},
            {**valid, "url": "ftp://allowed.example/path"},
            {**valid, "url": "https:allowed.example/path"},
            {**valid, "url": "https:///path"},
            {**valid, "url": "https://user:pass@allowed.example/path"},
            {**valid, "url": "https://@allowed.example/path"},
            {**valid, "url": "https://*.allowed.example/path"},
            {**valid, "url": "https://127.0.0.1/path"},
            {**valid, "url": "https://[::1]/path"},
            {**valid, "url": "https://2130706433/path"},
            {**valid, "url": "https://0177.0.0.1/path"},
            {**valid, "url": "https://0x7f000001/path"},
            {**valid, "url": "https://allowed.example:444/path"},
            {**valid, "url": "https://allowed.example:0443/path"},
            {**valid, "url": "https://allowed.example:/path"},
            {**valid, "url": "https://allowed.example:443:443/path"},
            {**valid, "url": "https://allowed.example/path#fragment"},
            {**valid, "url": "https://allowed.example/path#"},
            {**valid, "url": " https://allowed.example/path"},
            {**valid, "url": "https://allowed.exa\nmple/path"},
            {**valid, "url": "https://allöwed.example/path"},
        ):
            with self.subTest(rule=rule), self.assertRaises(ValueError):
                URLPolicy.from_dict({"rules": [rule]})
        for document in (None, [], {}, {"rules": ()}, {"rules": [], "extra": True}):
            with self.subTest(document=document), self.assertRaises(ValueError):
                URLPolicy.from_dict(document)

    def test_profile_loading_preserves_explicit_runner_services(self):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        rules = [{"url": "https://allowed.example/path", "methods": ["GET"]}]
        with tempfile.TemporaryDirectory(
            prefix="uv-url-policy-", dir=scratch
        ) as temporary:
            path = Path(temporary) / "policies.json"
            document = {"version": 1, "profiles": {"release": {"rules": rules}}}
            path.write_text(json.dumps(document))
            expected_rules = policy().to_dict()["rules"]
            self.assertEqual(
                profile_config(path, "release"),
                {"rules": expected_rules, "runner_services": False},
            )
            self.assertEqual(load(path, "release"), policy())
            document["profiles"]["release"]["runner_services"] = True
            path.write_text(json.dumps(document))
            self.assertEqual(
                profile_config(path, "release"),
                {"rules": expected_rules, "runner_services": True},
            )
            for invalid in (
                {"version": True, "profiles": {}},
                {"version": 2, "profiles": {}},
                {"version": 1, "profiles": []},
                {"version": 1, "profiles": {}, "extra": True},
                {"version": 1, "profiles": {"other": {"rules": rules}}},
                {
                    "version": 1,
                    "profiles": {
                        "release": {"rules": rules, "runner_services": "true"}
                    },
                },
                {
                    "version": 1,
                    "profiles": {"release": {"rules": rules, "extra": True}},
                },
            ):
                with self.subTest(document=invalid):
                    path.write_text(json.dumps(invalid))
                    with self.assertRaises(ValueError):
                        profile_config(path, "release")
                    with self.assertRaises(ValueError):
                        load(path, "release")


if __name__ == "__main__":
    unittest.main()
