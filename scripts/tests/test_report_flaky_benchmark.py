import argparse
import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "report_flaky_benchmark", Path(__file__).parents[1] / "report-flaky-benchmark.py"
)
assert SPEC is not None and SPEC.loader is not None
reporter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reporter)

HEAD = "9dbd30c" + "0" * 33
BASE = "25ea3bc" + "0" * 33
UPDATED_AT = "2026-09-20T15:30:55Z"
URI = "crates/uv-bench/benches/uv_pypi_types.rs::digest::from_hex/uppercase[48]"
URL = (
    "https://app.codspeed.io/astral-sh/uv-dev/branches/test?"
    "uri=crates%2Fuv-bench%2Fbenches%2Fuv_pypi_types.rs%3A%3Adigest%3A%3A"
    "from_hex%2Fuppercase%5B48%5D&runnerMode=Simulation"
)
BODY = (
    reporter.REPORT_MARKER + "\n"
    f"| ⚡ | Simulation | [`` from_hex/uppercase[48] ``]({URL}) | 7.6 µs | 6.9 µs | +10.1% |\n"
    "<sub>Comparing <code>test</code> (9dbd30c) with <code>main</code> (25ea3bc)</sub>"
)


def comment():
    return {
        "id": 42,
        "issue_url": f"https://api.github.com/repos/{reporter.REPOSITORY}/issues/1994",
        "html_url": f"https://github.com/{reporter.REPOSITORY}/pull/1994#issuecomment-42",
        "user": {"id": reporter.CODSPEED_BOT_ID, "type": "Bot"},
        "performed_via_github_app": {"id": reporter.CODSPEED_APP_ID},
        "updated_at": UPDATED_AT,
        "body": BODY,
    }


def pull_request():
    repository = {"id": reporter.REPOSITORY_ID, "full_name": reporter.REPOSITORY}
    return {
        "number": 1994,
        "state": "open",
        "title": "Add regression test",
        "body": "A regression test.",
        "html_url": f"https://github.com/{reporter.REPOSITORY}/pull/1994",
        "base": {"repo": repository},
        "head": {"repo": repository, "sha": HEAD},
    }


def context():
    return {
        "pull_request": {"number": 1994},
        "head_sha": HEAD,
        "base_sha": BASE,
        "comment": {
            "id": 42,
            "updated_at": UPDATED_AT,
            "body": BODY,
            "url": comment()["html_url"],
        },
        "benchmarks": reporter.candidates(BODY),
    }


def diagnosis():
    return {
        "findings": [
            {
                "uri": URI,
                "mode": "Simulation",
                "decision": "create",
                "reason": "The measured code is unchanged.",
                "issue": {
                    "title": "Spurious change in `from_hex/uppercase[48]`",
                    "body": "CodSpeed reports a change when only an integration test changed.",
                },
            }
        ]
    }


class ReportFlakyBenchmarkTests(unittest.TestCase):
    def test_parses_benchmark_name_with_brackets(self):
        self.assertEqual(
            reporter.candidates(BODY),
            [
                {
                    "uri": URI,
                    "mode": "Simulation",
                    "name": "from_hex/uppercase[48]",
                    "url": URL,
                    "row": BODY.splitlines()[1],
                }
            ],
        )
        self.assertEqual(
            reporter.candidates(BODY.replace("uv-dev/branches", "uv/branches")), []
        )
        self.assertEqual(
            reporter.candidates(
                BODY.replace("runnerMode=Simulation", "runnerMode=Unknown")
            ),
            [],
        )

    def test_collects_the_reported_revisions(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "context.json"
            with patch.object(
                reporter,
                "gh",
                side_effect=[
                    comment(),
                    pull_request(),
                    {"sha": BASE},
                    {"html_url": "comparison", "status": "ahead", "files": []},
                ],
            ) as gh:
                reporter.collect(
                    argparse.Namespace(
                        pull_request="1994",
                        head_sha=HEAD,
                        comment_id="42",
                        updated_at=UPDATED_AT,
                        context=path,
                    )
                )
            saved = json.loads(path.read_text())
            self.assertEqual(saved["base_sha"], BASE)
            self.assertEqual(saved["benchmarks"][0]["uri"], URI)
            self.assertEqual(
                gh.call_args.args,
                ("api", f"repos/{reporter.REPOSITORY}/compare/{BASE}...{HEAD}"),
            )

    def test_rejects_stale_or_untrusted_reports(self):
        for field, value in [
            ("issue_url", "https://api.github.com/repos/other/repo/issues/1994"),
            ("user", {"id": 1, "type": "Bot"}),
            ("performed_via_github_app", {"id": 1}),
            ("updated_at", "2026-09-21T00:00:00Z"),
            ("body", BODY.replace("(9dbd30c)", "(aaaaaaa)")),
        ]:
            changed = comment()
            changed[field] = value
            with (
                self.subTest(field=field),
                patch.object(reporter, "gh", side_effect=[changed, pull_request()]),
            ):
                self.assertIsNone(
                    reporter.current_report("1994", HEAD, "42", UPDATED_AT)
                )
        for side in ("head", "base"):
            pull = copy.deepcopy(pull_request())
            pull[side]["repo"]["id"] = 1
            with patch.object(reporter, "gh", side_effect=[comment(), pull]):
                self.assertIsNone(
                    reporter.current_report("1994", HEAD, "42", UPDATED_AT)
                )

    def test_rejects_unreported_benchmarks_and_mentions(self):
        for field, value in [("uri", "other-benchmark"), ("mode", "Walltime")]:
            finding = diagnosis()
            finding["findings"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                reporter.issue_payloads(context(), finding)
        finding = diagnosis()
        finding["findings"][0]["issue"]["body"] = "Ping @somebody"
        with self.assertRaises(ValueError):
            reporter.issue_payloads(context(), finding)

    def test_creates_then_deduplicates_without_comments(self):
        payload = reporter.issue_payloads(context(), diagnosis())[0]
        issue = {
            **payload,
            "html_url": f"https://github.com/{reporter.REPOSITORY}/issues/123",
        }
        with tempfile.TemporaryDirectory() as directory:
            context_path = Path(directory) / "context.json"
            diagnosis_path = Path(directory) / "diagnosis.json"
            context_path.write_text(json.dumps(context()))
            diagnosis_path.write_text(json.dumps(diagnosis()))
            args = argparse.Namespace(context=context_path, diagnosis=diagnosis_path)
            with patch.object(
                reporter, "gh", side_effect=[comment(), pull_request(), [[]], issue]
            ) as gh:
                reporter.report(args)
                self.assertEqual(
                    gh.call_args.args,
                    (
                        "api",
                        "--method",
                        "POST",
                        f"repos/{reporter.REPOSITORY}/issues",
                        "--input",
                        "-",
                    ),
                )
                self.assertEqual(gh.call_args.kwargs["payload"], payload)
            with patch.object(
                reporter, "gh", side_effect=[comment(), pull_request(), [[issue]]]
            ) as gh:
                reporter.report(args)
                self.assertEqual(gh.call_count, 3)
            with patch.object(reporter, "gh", return_value={}) as gh:
                reporter.report(args)
                self.assertEqual(gh.call_count, 1)


if __name__ == "__main__":
    unittest.main()
