"""Check the reviewed URL bundle and its explicit runner-service exceptions."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

import proxy
import url_proxy
from policy import Policy, load
from url_settings import compile_policy, oidc_url, runtime_services, service_url


class URLSettingsTests(unittest.TestCase):
    def test_runner_metadata_is_narrow_and_contains_no_queries(self):
        environment = {
            "ACTIONS_RUNTIME_URL": "https://pipelines.actions.githubusercontent.com/account/",
            "ACTIONS_RESULTS_URL": "http://10.2.3.4:978/",
            "ACTIONS_ID_TOKEN_REQUEST_URL": "https://identity.actions.githubusercontent.com/job/oidctoken?opaque=synthetic-secret",
            "ACTIONS_RUNTIME_TOKEN": "synthetic-secret",
        }
        self.assertEqual(
            runtime_services(environment),
            {
                "runtime": environment["ACTIONS_RUNTIME_URL"],
                "results": "https://results-receiver.actions.githubusercontent.com/",
                "oidc": "https://identity.actions.githubusercontent.com/job/oidctoken",
            },
        )
        self.assertNotIn("synthetic-secret", json.dumps(runtime_services(environment)))
        for value in (
            "https://api.github.com/",
            "https://user@pipelines.actions.githubusercontent.com/",
            "https://pipelines.actions.githubusercontent.com/?token=synthetic-secret",
            "https://pipelines.actions.githubusercontent.com/#fragment",
            "https://pipelines.actions.githubusercontent.com:444/",
            "http://127.0.0.1:978/",
            "https://pipelines.actions.githubusercontent.com/\n",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                service_url(value)
        for value in ("http://10.2.3.4:977/", "http://example.invalid:978/"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                runtime_services({**environment, "ACTIONS_RESULTS_URL": value})
        for value in (
            "https://api.github.com/job/oidctoken",
            "https://user@identity.actions.githubusercontent.com/job/oidctoken",
            "https://identity.actions.githubusercontent.com/job/oidctoken#fragment",
            "https://identity.actions.githubusercontent.com/job/oidctoken#",
            "http://identity.actions.githubusercontent.com/job/oidctoken",
            "https://identity.actions.githubusercontent.com:444/job/oidctoken",
            "https://identity.actions.githubusercontent.com/job/oidctoken?bad=\n",
            "https://identity.actions.githubusercontent.com/job/../oidctoken",
            "https://identity.actions.githubusercontent.com/job/./oidctoken",
            "https://identity.actions.githubusercontent.com/job/..;x/oidctoken",
            "https://identity.actions.githubusercontent.com/job/..%20/oidctoken",
            "https://identity.actions.githubusercontent.com/job/%2e%2e/oidctoken",
            "https://identity.actions.githubusercontent.com/job/%2foidctoken",
            "https://identity.actions.githubusercontent.com/job\\oidctoken",
            "https://identity.actions.githubusercontent.com/job/{integer}/oidctoken",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                oidc_url(value)

    def test_oidc_repeated_slash_exception_is_one_exact_injected_route(self):
        endpoint = "https://identity.actions.githubusercontent.com/job//oidctoken"
        services = runtime_services(
            {
                "ACTIONS_RUNTIME_URL": "https://pipelines.actions.githubusercontent.com/account/",
                "ACTIONS_ID_TOKEN_REQUEST_URL": endpoint + "?opaque=synthetic-secret",
            }
        )
        self.assertEqual(services["oidc"], endpoint)
        domain = load(ACTION / "policies.json", "github")
        policy = compile_policy(
            ACTION / "url-policies.json", "github-api-probe", domain, services
        )
        self.assertEqual(
            [
                item.to_dict()
                for item in policy.rules
                if item.host == "identity.actions.githubusercontent.com"
            ],
            [
                {
                    "url": endpoint,
                    "methods": ["GET"],
                    "match": "exact",
                    "query": "any",
                    "allow_repeated_slashes": True,
                }
            ],
        )
        self.assertNotIn("synthetic-secret", json.dumps(policy.to_dict()))
        self.assertTrue(
            policy.permits(
                "https",
                "identity.actions.githubusercontent.com",
                443,
                "GET",
                "/job//oidctoken?audience=synthetic%2Fvalue%3D",
            )
        )
        for method, target in (
            ("POST", "/job//oidctoken"),
            ("GET", "/job/oidctoken"),
            ("GET", "/job///oidctoken"),
            ("GET", "//job/oidctoken"),
            ("GET", "/job//oidctoken/other"),
            ("GET", "/job//../oidctoken"),
            ("GET", "/job//oidctoken?bad=%0a"),
        ):
            with self.subTest(method=method, target=target):
                self.assertFalse(
                    policy.permits(
                        "https",
                        "identity.actions.githubusercontent.com",
                        443,
                        method,
                        target,
                    )
                )
        denied = Policy(
            domain.allow, (*domain.deny, "identity.actions.githubusercontent.com")
        )
        with self.assertRaisesRegex(
            ValueError, "^URL profile exceeds its domain policy$"
        ):
            compile_policy(
                ACTION / "url-policies.json", "github-api-probe", denied, services
            )

    def test_oidc_root_path_is_a_valid_exact_route(self):
        endpoint = "https://identity.actions.githubusercontent.com"
        for value in (endpoint, endpoint + "/?opaque=synthetic-secret"):
            with self.subTest(value=value):
                self.assertEqual(oidc_url(value), endpoint + "/")
        policy = compile_policy(
            ACTION / "url-policies.json",
            "github-api-probe",
            load(ACTION / "policies.json", "github"),
            {
                "runtime": "https://pipelines.actions.githubusercontent.com/",
                "results": "https://results-receiver.actions.githubusercontent.com/",
                "oidc": endpoint,
            },
        )
        self.assertEqual(
            [
                item.to_dict()
                for item in policy.rules
                if item.host == "identity.actions.githubusercontent.com"
            ],
            [
                {
                    "url": endpoint + "/",
                    "methods": ["GET"],
                    "match": "exact",
                    "query": "any",
                }
            ],
        )
        self.assertTrue(
            policy.permits(
                "https",
                "identity.actions.githubusercontent.com",
                443,
                "GET",
                "/?audience=synthetic",
            )
        )
        for method, target in (("POST", "/"), ("GET", "//"), ("GET", "/oidctoken")):
            with self.subTest(method=method, target=target):
                self.assertFalse(
                    policy.permits(
                        "https",
                        "identity.actions.githubusercontent.com",
                        443,
                        method,
                        target,
                    )
                )

    def test_effective_example_preserves_url_restrictions(self):
        services = {
            "runtime": "https://pipelines.actions.githubusercontent.com/account/",
            "results": "https://results-receiver.actions.githubusercontent.com/",
            "oidc": "https://identity.actions.githubusercontent.com/job/oidctoken",
        }
        domain = load(ACTION / "policies.json", "github")
        policy = compile_policy(
            ACTION / "url-policies.json", "github-api-probe", domain, services
        )
        self.assertTrue(all(domain.permits(host) for host in policy.hosts))
        self.assertLessEqual(len(policy.hosts), 128)
        self.assertTrue(
            policy.permits(
                "https",
                "identity.actions.githubusercontent.com",
                443,
                "GET",
                "/job/oidctoken?audience=synthetic",
            )
        )
        for method, path in (
            ("POST", "/job/oidctoken"),
            ("GET", "/job/oidctoken/other"),
            ("GET", "/another/oidctoken"),
        ):
            with self.subTest(method=method, path=path):
                self.assertFalse(
                    policy.permits(
                        "https",
                        "identity.actions.githubusercontent.com",
                        443,
                        method,
                        path,
                    )
                )
        for method in ("GET", "HEAD"):
            self.assertTrue(
                policy.permits(
                    "https", "api.github.com", 443, method, "/repos/astral-sh/uv-dev"
                )
            )
        for method, path in (
            ("POST", "/repos/astral-sh/uv-dev"),
            ("GET", "/repos/astral-sh/uv-dev?unexpected=1"),
            ("GET", "/repos/astral-sh/uv-dev/issues"),
            ("GET", "/repos/astral-sh/uv-dev/../uv"),
        ):
            with self.subTest(method=method, path=path):
                self.assertFalse(
                    policy.permits("https", "api.github.com", 443, method, path)
                )
        self.assertTrue(
            policy.permits(
                "https",
                "results-receiver.actions.githubusercontent.com",
                443,
                "POST",
                "/twirp/results.services.receiver.Receiver/GetStepLogsSignedBlobURL",
            )
        )
        self.assertTrue(
            policy.permits(
                "https",
                "results-receiver.actions.githubusercontent.com",
                443,
                "POST",
                "/twirp/github.actions.results.api.v1.ArtifactService/ListArtifacts",
            )
        )
        self.assertFalse(
            policy.permits(
                "https",
                "results-receiver.actions.githubusercontent.com",
                443,
                "POST",
                "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL",
            )
        )
        self.assertFalse(
            policy.permits(
                "https",
                "pipelines.actions.githubusercontent.com",
                443,
                "GET",
                "/account/_apis/artifactcache/cache?keys=synthetic",
            )
        )
        for host, path in (
            ("pipelines.actions.githubusercontent.com", "/account/renewjob"),
            ("pipelines.actions.githubusercontent.com", "/account/completejob"),
            (
                "run-actions-1-azure-eastus.actions.githubusercontent.com",
                "/renewjob",
            ),
            (
                "run-actions-3-azure-eastus.actions.githubusercontent.com",
                "/completejob",
            ),
            (
                "run-actions-1-azure-eastus.actions.githubusercontent.com",
                "/58/completejob",
            ),
            (
                "run-actions-2-azure-eastus.actions.githubusercontent.com",
                "/176/renewjob",
            ),
        ):
            with self.subTest(host=host, path=path):
                self.assertTrue(policy.permits("https", host, 443, "POST", path))
                self.assertFalse(policy.permits("https", host, 443, "GET", path))
                self.assertFalse(
                    policy.permits("https", host, 443, "POST", path + "/extra")
                )
                self.assertFalse(
                    policy.permits("https", host, 443, "POST", path + "?unexpected=1")
                )
        self.assertFalse(
            policy.permits(
                "https",
                "run-actions-4-azure-eastus.actions.githubusercontent.com",
                443,
                "POST",
                "/completejob",
            )
        )
        self.assertFalse(
            policy.permits(
                "https",
                "run-actions-1-azure-eastus.actions.githubusercontent.com",
                443,
                "POST",
                "/other/completejob",
            )
        )
        self.assertTrue(
            policy.permits(
                "https",
                "productionresultssa0.blob.core.windows.net",
                443,
                "PUT",
                "/signed-log?sig=synthetic%2Fvalue%3D",
            )
        )
        self.assertFalse(
            policy.permits(
                "https", "other.blob.core.windows.net", 443, "PUT", "/signed-log"
            )
        )

    def test_profile_cannot_expand_domain_permissions(self):
        with self.assertRaisesRegex(
            ValueError, "^URL profile exceeds its domain policy$"
        ):
            compile_policy(
                ACTION / "url-policies.json",
                "github-api-probe",
                Policy(("api.github.com",)),
                {
                    "runtime": "https://pipelines.actions.githubusercontent.com/",
                    "results": "https://results-receiver.actions.githubusercontent.com/",
                },
            )

    def test_runtime_candidate_preserves_relative_uri_resolution(self):
        services = runtime_services(
            {
                "ACTIONS_RUNTIME_URL": "https://pipelines.actions.githubusercontent.com/account",
                "ACTIONS_RESULTS_URL": "https://results-receiver.actions.githubusercontent.com/",
            }
        )
        self.assertEqual(
            services["runtime"],
            "https://pipelines.actions.githubusercontent.com/account",
        )
        policy = compile_policy(
            ACTION / "url-policies.json",
            "github-api-probe",
            load(ACTION / "policies.json", "github"),
            services,
        )
        self.assertTrue(
            policy.permits(
                "https",
                "pipelines.actions.githubusercontent.com",
                443,
                "POST",
                "/completejob",
            )
        )
        self.assertFalse(
            policy.permits(
                "https",
                "pipelines.actions.githubusercontent.com",
                443,
                "POST",
                "/account/completejob",
            )
        )

    def test_runner_exceptions_are_opt_in(self):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="uv-url-settings-", dir=scratch
        ) as temporary:
            path = Path(temporary) / "profiles.json"
            path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "profiles": {
                            "exact": {
                                "rules": [
                                    {
                                        "url": "https://api.github.com/rate_limit",
                                        "methods": ["GET"],
                                    }
                                ]
                            }
                        },
                    }
                )
            )
            policy = compile_policy(path, "exact", Policy(("api.github.com",)), {})
            self.assertEqual(policy.hosts, ("api.github.com",))
            self.assertEqual(len(policy.rules), 1)

    def test_service_requires_an_explicit_mode_and_selects_strict_handlers(self):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="uv-url-service-", dir=scratch
        ) as temporary:
            directory = Path(temporary)
            (directory / "audit").mkdir()
            (directory / "policies.json").write_text(
                json.dumps(
                    {
                        "version": 1,
                        "profiles": {
                            "exact": {"allow": ["api.github.com"], "deny": []}
                        },
                    }
                )
            )
            settings = {"profile": "exact", "resolvers": []}
            (directory / "settings.json").write_text(json.dumps(settings))
            with self.assertRaises(KeyError):
                proxy.serve(directory)
            (directory / "settings.json").write_text(
                json.dumps({**settings, "url_profile": False})
            )
            with self.assertRaises(TypeError):
                proxy.serve(directory)
            (directory / "settings.json").write_text(
                json.dumps({**settings, "url_profile": "exact"})
            )
            (directory / "url-policy.json").write_text(
                json.dumps(
                    {
                        "rules": [
                            {
                                "url": "https://api.github.com/rate_limit",
                                "methods": ["GET"],
                            }
                        ]
                    }
                )
            )
            handlers = []

            class Server:
                def __init__(self, _address, handler):
                    handlers.append(handler)

                def serve_forever(self):
                    pass

            with (
                patch.object(proxy, "TCPServer", Server),
                patch.object(proxy, "UDPServer", Server),
                patch.object(proxy.threading, "Thread"),
                patch.object(url_proxy, "make_tls_context") as context,
            ):
                proxy.serve(directory)
            context.assert_called_once()
            self.assertEqual(
                handlers,
                [
                    url_proxy.HTTPHandler,
                    url_proxy.TLSHandler,
                    proxy.DNSStreamHandler,
                    proxy.DNSDatagramHandler,
                ]
                * 2,
            )


if __name__ == "__main__":
    unittest.main()
