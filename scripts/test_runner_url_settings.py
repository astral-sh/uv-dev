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
from url_settings import compile_policy, runtime_services, service_url


class URLSettingsTests(unittest.TestCase):
    def test_runner_metadata_is_narrow_and_contains_no_queries(self):
        environment = {
            "ACTIONS_RUNTIME_URL": "https://pipelines.actions.githubusercontent.com/account/",
            "ACTIONS_RESULTS_URL": "http://10.2.3.4:978/",
            "ACTIONS_RUNTIME_TOKEN": "synthetic-secret",
        }
        self.assertEqual(
            runtime_services(environment),
            {
                "runtime": environment["ACTIONS_RUNTIME_URL"],
                "results": "https://results-receiver.actions.githubusercontent.com/",
            },
        )
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

    def test_effective_example_preserves_url_restrictions(self):
        services = {
            "runtime": "https://pipelines.actions.githubusercontent.com/account/",
            "results": "https://results-receiver.actions.githubusercontent.com/",
        }
        domain = load(ACTION / "policies.json", "github")
        policy = compile_policy(
            ACTION / "url-policies.json", "github-api-probe", domain, services
        )
        self.assertTrue(all(domain.permits(host) for host in policy.hosts))
        self.assertLessEqual(len(policy.hosts), 128)
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
