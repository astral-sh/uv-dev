"""Exercise strict URL matching without changing the host network."""

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

from url_policy import URLPolicy, load, parse_url, profile_config, request_target


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

    def test_repeated_slashes_require_an_explicit_exact_path(self):
        url = "https://allowed.example/job//oidctoken?api-version=2.0"
        target = "/job//oidctoken?api-version=2.0"
        with self.assertRaises(ValueError):
            request_target(target)
        with self.assertRaises(ValueError):
            parse_url(url)
        self.assertEqual(
            request_target(target, allow_repeated_slashes=True),
            ("/job//oidctoken", "?api-version=2.0"),
        )
        self.assertEqual(
            parse_url(url, allow_repeated_slashes=True),
            ("https", "allowed.example", 443, "/job//oidctoken", "?api-version=2.0"),
        )
        allowed = policy(url, allow_repeated_slashes=True)
        self.assertTrue(allowed.permits("https", "allowed.example", 443, "GET", target))
        for variant in (
            "/job/oidctoken?api-version=2.0",
            "/job///oidctoken?api-version=2.0",
            "//job/oidctoken?api-version=2.0",
            "/job//oidctoken/?api-version=2.0",
            "/job//oidctoken/extra?api-version=2.0",
            "/job//oidctoken",
            "/job//oidctoken?",
            "/job//oidctoken?api-version=%32.0",
            "/job//oidctoken?api-version=2.0&extra=1",
        ):
            with self.subTest(target=variant):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "GET", variant)
                )
        self.assertFalse(
            allowed.permits("https", "allowed.example", 443, "POST", target)
        )
        self.assertFalse(allowed.permits("https", "other.example", 443, "GET", target))
        self.assertEqual(
            allowed.to_dict(),
            {
                "rules": [
                    {
                        "url": url,
                        "methods": ["GET"],
                        "match": "exact",
                        "query": "exact",
                        "allow_repeated_slashes": True,
                    }
                ]
            },
        )
        self.assertEqual(URLPolicy.from_dict(allowed.to_dict()), allowed)
        self.assertEqual(policy(allow_repeated_slashes=False), policy())
        self.assertNotIn("allow_repeated_slashes", policy().to_dict()["rules"][0])

    def test_repeated_slash_exception_does_not_widen_other_rules(self):
        allowed = URLPolicy.from_dict(
            {
                "rules": [
                    {
                        "url": "https://allowed.example/job//oidctoken",
                        "methods": ["GET"],
                        "query": "any",
                        "allow_repeated_slashes": True,
                    },
                    {
                        "url": "https://allowed.example/",
                        "methods": ["GET"],
                        "match": "prefix",
                        "query": "any",
                    },
                    {
                        "url": "https://allowed.example/{integer}/completejob",
                        "methods": ["POST"],
                        "match": "template",
                    },
                ]
            }
        )
        self.assertTrue(
            allowed.permits(
                "https",
                "allowed.example",
                443,
                "GET",
                "/job//oidctoken?audience=a%2Fb%20c",
            )
        )
        self.assertTrue(
            allowed.permits("https", "allowed.example", 443, "GET", "/other/path")
        )
        self.assertTrue(
            allowed.permits("https", "allowed.example", 443, "POST", "/58/completejob")
        )
        for method, target in (
            ("GET", "/other//path"),
            ("GET", "//job/oidctoken"),
            ("GET", "/job///oidctoken"),
            ("GET", "/job//oidctoken/extra"),
            ("GET", "/job//oidctoken?invalid=%0a"),
            ("POST", "/job//oidctoken"),
            ("POST", "/58//completejob"),
            ("POST", "//58/completejob"),
        ):
            with self.subTest(method=method, target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, method, target)
                )
        self.assertEqual(URLPolicy.from_dict(allowed.to_dict()), allowed)

    def test_repeated_slash_exception_keeps_other_normalization_checks(self):
        for target in (
            "/job//../oidctoken",
            "/job//./oidctoken",
            "/job//..;x/oidctoken",
            "/job//..%20/oidctoken",
            "/job//%2e/oidctoken",
            "/job//%2foidctoken",
            "/job//%25oidctoken",
            "/job//oidctoken#fragment",
            "/job//oidctoken?invalid=%",
            "/job//oidctoken?invalid=%0a",
        ):
            with self.subTest(target=target):
                with self.assertRaises(ValueError):
                    request_target(target, allow_repeated_slashes=True)
                with self.assertRaises(ValueError):
                    policy(
                        "https://allowed.example" + target,
                        allow_repeated_slashes=True,
                    )

    def test_repeated_slash_schema_only_permits_boolean_exact_opt_in(self):
        for setting in (None, 0, 1, "true", [], {}):
            with self.subTest(setting=setting), self.assertRaises(ValueError):
                policy(allow_repeated_slashes=setting)
        for setting in (False, True):
            for match, url in (
                ("prefix", "https://allowed.example/path/"),
                ("template", "https://allowed.example/{integer}/path"),
            ):
                with (
                    self.subTest(setting=setting, match=match),
                    self.assertRaises(ValueError),
                ):
                    policy(url, match=match, allow_repeated_slashes=setting)
        for settings in ({}, {"allow_repeated_slashes": False}):
            with self.subTest(settings=settings), self.assertRaises(ValueError):
                policy("https://allowed.example/job//oidctoken", **settings)
        with self.assertRaises(ValueError):
            parse_url(
                "https://allowed.example/{integer}/path",
                template=True,
                allow_repeated_slashes=True,
            )

    def test_templates_match_complete_ascii_decimal_segments(self):
        allowed = policy(
            "https://allowed.example/{integer}/completejob",
            methods=["POST"],
            match="template",
        )
        for target in ("/0/completejob", "/58/completejob", "/0058/completejob"):
            with self.subTest(target=target):
                self.assertTrue(
                    allowed.permits("https", "allowed.example", 443, "POST", target)
                )
        for target in (
            "//completejob",
            "/+58/completejob",
            "/-58/completejob",
            "/5.8/completejob",
            "/0x3a/completejob",
            "/58a/completejob",
            "/%35%38/completejob",
            "/５８/completejob",
            "/58/CompleteJob",
            "/58/completejob/",
            "/58/completejob/extra",
            "/58/completejob?",
            "/58/completejob?unexpected=1",
            "/58/../completejob",
            "/58;parameter/completejob",
            "/58%20/completejob",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "POST", target)
                )
        self.assertFalse(
            allowed.permits("https", "allowed.example", 443, "GET", "/58/completejob")
        )
        self.assertEqual(URLPolicy.from_dict(allowed.to_dict()), allowed)

    def test_templates_keep_literal_segments_and_query_rules(self):
        allowed = policy(
            "https://allowed.example/runs/{integer}/jobs/{integer}/?api-version=6.0",
            methods=["POST"],
            match="template",
        )
        self.assertTrue(
            allowed.permits(
                "https",
                "allowed.example",
                443,
                "POST",
                "/runs/58/jobs/9/?api-version=6.0",
            )
        )
        for target in (
            "/runs/58/jobs/9/",
            "/runs/58/jobs/9/?api-version=%36.0",
            "/runs/58/jobs/9/?api-version=6.0&extra=1",
            "/runs/58/job/9/?api-version=6.0",
            "/runs/58/jobs/9/more/?api-version=6.0",
        ):
            with self.subTest(target=target):
                self.assertFalse(
                    allowed.permits("https", "allowed.example", 443, "POST", target)
                )
        literal = policy(
            "https://allowed.example/*/{integer}/fixed",
            match="template",
            query="any",
        )
        self.assertTrue(
            literal.permits("https", "allowed.example", 443, "GET", "/*/7/fixed?q=%20")
        )
        self.assertFalse(
            literal.permits("https", "allowed.example", 443, "GET", "/other/7/fixed")
        )
        self.assertEqual(URLPolicy.from_dict(literal.to_dict()), literal)

    def test_invalid_templates_are_rejected(self):
        for url in (
            "https://allowed.example/58/completejob",
            "https://{integer}.example/completejob",
            "https://allowed.example:{integer}/completejob",
            "https://allowed.example/completejob?shard={integer}",
            "https://allowed.example/{integer}/completejob?shard={integer}",
            "https://allowed.example/{integer}suffix/completejob",
            "https://allowed.example/prefix{integer}/completejob",
            "https://allowed.example/{integer}{integer}/completejob",
            "https://allowed.example/{id}/completejob",
            "https://allowed.example/{Integer}/completejob",
            "https://allowed.example/{integer*}/completejob",
            "https://allowed.example/%7Binteger%7D/completejob",
            "https://allowed.example//{integer}/completejob",
            "https://allowed.example/{integer}/../completejob",
            "https://allowed.example/{integer};parameter/completejob",
            "https://allowed.example/{integer}/%2e/completejob",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                policy(url, match="template")
        for mode in ("exact", "prefix"):
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                policy("https://allowed.example/{integer}/", match=mode)

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
