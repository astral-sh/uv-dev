import copy
import importlib.util
import json
import subprocess
import sys
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "ci_timings", Path(__file__).parents[1] / "ci-timings.py"
)
timings = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = timings
SPEC.loader.exec_module(timings)

SHA = "a" * 40
WORKFLOW_ENV = {
    "GITHUB_REPOSITORY": timings.REPOSITORY,
    "GITHUB_REF": "refs/heads/main",
    "GITHUB_EVENT_NAME": "workflow_dispatch",
}


def job(seconds=120, *, name="bench / walltime", labels=None, conclusion="success"):
    started = datetime(2026, 10, 15, tzinfo=UTC)
    completed = started + timedelta(seconds=seconds)
    return {
        "id": 101,
        "name": name,
        "labels": labels or ["runner-linux"],
        "status": "completed",
        "conclusion": conclusion,
        "started_at": started.isoformat(),
        "completed_at": completed.isoformat(),
        "steps": [
            {
                "name": "Run benchmarks",
                "status": "completed",
                "conclusion": "success",
                "started_at": started.isoformat(),
                "completed_at": completed.isoformat(),
            }
        ],
    }


def run(run_id=100, branch="feature", *, created="2026-10-16T00:00:00Z"):
    return {
        "id": run_id,
        "repository": {"id": timings.REPOSITORY_ID},
        "head_repository": {"id": timings.REPOSITORY_ID},
        "path": timings.WORKFLOW,
        "workflow_id": 10,
        "event": "pull_request",
        "head_sha": SHA if branch == "feature" else f"{run_id:040x}",
        "head_branch": branch,
        "created_at": created,
        "status": "completed",
        "conclusion": "success",
        "run_attempt": 1,
        "pull_requests": [
            {
                "number": 42,
                "base": {"ref": "main", "repo": {"id": timings.REPOSITORY_ID}},
            }
        ],
    }


def pull():
    return {
        "number": 42,
        "state": "open",
        "base": {"repo": {"id": timings.REPOSITORY_ID}, "ref": "main"},
        "head": {"repo": {"id": timings.REPOSITORY_ID}, "ref": "feature", "sha": SHA},
    }


def report(seconds=240, samples=(120, 120, 120, 120, 120)):
    baselines = [
        (run(index, f"baseline-{index}"), timings.measurements([job(value)]))
        for index, value in enumerate(samples, 1)
    ]
    return timings.compare(run(), pull(), [job(seconds)], baselines)


def comment(value):
    return {
        "id": 55,
        "user": {"id": timings.BOT_ID},
        "performed_via_github_app": {"slug": "astral-automations-bot"},
        "body": timings.render(value),
    }


class FakeAPI:
    def __init__(self):
        self.objects = {"actions/runs/100": run(), "pulls/42": pull()}
        self.collections = {"issues/42/comments": [], "actions/workflows/10/runs": []}
        self.reads = []
        self.writes = []

    def request(self, path, *, body=None, method="GET"):
        if method != "GET":
            self.writes.append((method, path, body))
            return {}
        self.reads.append(path)
        return copy.deepcopy(self.objects[path])

    def items(self, path, key=None):
        self.reads.append(path)
        yield from copy.deepcopy(self.collections[path.split("?")[0]])


class TimingTests(unittest.TestCase):
    def test_reports_large_increase_with_step_details(self):
        result = report()
        finding = timings.regressions(result)[0]
        self.assertEqual(finding["comparison"]["increase"], 120)
        self.assertEqual(finding["steps"][0]["name"], "Run benchmarks")
        self.assertIn("+2m 00s (+100%)", timings.render(result))

    def test_requires_absolute_and_relative_thresholds(self):
        self.assertEqual(timings.regressions(report(150)), [])
        self.assertEqual(timings.regressions(report(720, (600,) * 5)), [])

    def test_requires_enough_samples(self):
        result = report(600, (120,) * 4)
        self.assertEqual(timings.regressions(result), [])
        self.assertIn("No jobs could be compared", timings.render(result))

    def test_normal_variation_suppresses_alert(self):
        self.assertEqual(
            timings.regressions(report(600, (100, 200, 300, 400, 500))), []
        )

    def test_single_baseline_outlier_does_not_hide_regression(self):
        self.assertEqual(
            len(timings.regressions(report(240, (120, 120, 121, 122, 2400)))), 1
        )

    def test_runner_change_has_no_comparable_baseline(self):
        baseline = [(run(index), timings.measurements([job()])) for index in range(5)]
        result = timings.compare(
            run(), pull(), [job(600, labels=["runner-macos"])], baseline
        )
        self.assertEqual(timings.regressions(result), [])
        self.assertIsNone(result["jobs"][0]["comparison"])

    def test_queue_and_job_timestamp_skew_do_not_change_runtime(self):
        source = job()
        source["started_at"] = "2026-10-14T00:00:00Z"
        source["completed_at"] = "2026-10-16T00:00:00Z"
        self.assertEqual(timings.measure_job(source)["seconds"], 120)

    def test_failed_or_incomplete_measurements_are_excluded(self):
        self.assertIsNone(timings.measure_job(job(conclusion="failure")))
        source = job()
        source["steps"][0]["completed_at"] = None
        self.assertIsNone(timings.measure_job(source))

    def test_continue_on_error_step_does_not_produce_successful_sample(self):
        source = job()
        source["steps"][0]["conclusion"] = "failure"
        self.assertIsNone(timings.measure_job(source))

    def test_duplicate_job_names_are_excluded(self):
        self.assertEqual(timings.measurements([job(), job(600)]), {})

    def test_baseline_steps_must_finish_before_source_run(self):
        self.assertIsNone(
            timings.measure_job(job(), before=timings.timestamp("2026-10-15T00:01:00Z"))
        )

    def test_repeated_step_names_remain_separate(self):
        source = job()
        source["steps"].append(copy.deepcopy(source["steps"][0]))
        self.assertEqual(len(timings.measure_job(source)["steps"]), 2)

    def test_workflow_names_cannot_inject_markdown_or_mentions(self):
        value = timings.inline(
            "</code><img src=x> @maintainer |\n[link](https://bad.example)"
        )
        self.assertNotIn("<img", value)
        self.assertNotIn("@maintainer", value)
        self.assertNotIn("|", value)
        self.assertNotIn("\n", value)
        self.assertNotIn("[link]", value)


class ContextTests(unittest.TestCase):
    def test_partial_retry_keeps_successful_jobs_from_earlier_attempt(self):
        api = FakeAPI()
        api.collections["actions/runs/100/attempts/1/jobs"] = [
            job(600),
            job(name="test", conclusion="failure"),
        ]
        api.collections["actions/runs/100/attempts/2/jobs"] = [job(name="test")]
        measured = timings.measurements(timings.source_jobs(api, 100, 2))
        benchmark = measured[timings.job_key(job())]
        self.assertEqual((benchmark["seconds"], benchmark["attempt"]), (600, 1))
        self.assertEqual(measured[timings.job_key(job(name="test"))]["attempt"], 2)

    def test_retry_replaces_previous_success_even_when_it_fails(self):
        api = FakeAPI()
        api.collections["actions/runs/100/attempts/1/jobs"] = [job(600)]
        api.collections["actions/runs/100/attempts/2/jobs"] = [
            job(conclusion="failure")
        ]
        self.assertEqual(timings.measurements(timings.source_jobs(api, 100, 2)), {})

    def test_rejects_upstream_repository(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["repository"]["id"] = 699532645
        with self.assertRaisesRegex(ValueError, "uv-dev"):
            timings.source_context(api, 100, SHA, 1)

    def test_rejects_untrusted_head_repository(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["head_repository"]["id"] = 123
        with self.assertRaises(ValueError):
            timings.source_context(api, 100, SHA, 1)

    def test_superseded_commit_is_skipped(self):
        api = FakeAPI()
        api.objects["pulls/42"]["head"]["sha"] = "b" * 40
        self.assertIsNone(timings.source_context(api, 100, SHA, 1))

    def test_stale_attempt_is_skipped(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["run_attempt"] = 2
        self.assertIsNone(timings.source_context(api, 100, SHA, 1))

    def test_retargeted_pr_is_skipped(self):
        api = FakeAPI()
        api.objects["pulls/42"]["base"]["ref"] = "another-branch"
        self.assertIsNone(timings.source_context(api, 100, SHA, 1))

    def test_missing_run_association_uses_commit_pull_requests(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["pull_requests"] = []
        api.collections[f"commits/{SHA}/pulls"] = [pull()]
        self.assertEqual(timings.source_context(api, 100, SHA, 1)[1]["number"], 42)

    def test_upstream_commit_association_is_not_read_as_local_pr(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["pull_requests"] = []
        upstream = pull()
        upstream["number"] = 22541
        upstream["base"]["repo"]["id"] = 699532645
        api.collections[f"commits/{SHA}/pulls"] = [upstream, pull()]
        self.assertEqual(timings.source_context(api, 100, SHA, 1)[1]["number"], 42)
        self.assertNotIn("pulls/22541", api.reads)

    def test_explicit_pr_is_available_only_for_historical_analysis(self):
        api = FakeAPI()
        api.objects["pulls/42"]["state"] = "closed"
        with self.assertRaisesRegex(ValueError, "historical"):
            timings.source_context(api, 100, SHA, 1, pull_number=42)
        self.assertEqual(
            timings.source_context(api, 100, SHA, 1, historical=True, pull_number=42)[
                1
            ]["number"],
            42,
        )

    def test_baseline_excludes_same_pr_and_deduplicates_branches(self):
        api = FakeAPI()
        own = run(90, "feature", created="2026-10-15T00:00:00Z")
        first = run(89, "other", created="2026-10-15T00:00:00Z")
        first["pull_requests"] = [{"number": 43}]
        first["run_attempt"] = 3
        duplicate = copy.deepcopy(first)
        duplicate["id"] = 88
        api.collections["actions/workflows/10/runs"] = [own, first, duplicate]
        api.collections["actions/runs/89/attempts/1/jobs"] = [job()]
        selected = timings.baseline_runs(api, run(), 42)
        self.assertEqual([source["id"] for source, _ in selected], [89])
        self.assertNotIn("actions/runs/88/attempts/1/jobs", api.reads)


@patch.dict("os.environ", WORKFLOW_ENV)
class PublishingTests(unittest.TestCase):
    def test_healthy_run_does_not_create_comment(self):
        api = FakeAPI()
        timings.publish(api, report(120))
        self.assertEqual(api.writes, [])

    def test_regression_creates_comment(self):
        api = FakeAPI()
        timings.publish(api, report())
        self.assertEqual(api.writes[0][:2], ("POST", "issues/42/comments"))

    def test_updates_only_own_comment(self):
        api = FakeAPI()
        forged = comment(report())
        forged["user"]["id"] = 123
        api.collections["issues/42/comments"] = [forged]
        timings.publish(api, report())
        self.assertEqual(api.writes[0][0], "POST")

    def test_requires_matching_app_identity(self):
        api = FakeAPI()
        forged = comment(report())
        forged["performed_via_github_app"] = None
        api.collections["issues/42/comments"] = [forged]
        timings.publish(api, report())
        self.assertEqual(api.writes[0][0], "POST")

    def test_duplicate_delivery_is_no_op(self):
        api = FakeAPI()
        api.collections["issues/42/comments"] = [comment(report())]
        timings.publish(api, report())
        self.assertEqual(api.writes, [])

    def test_remeasured_warning_is_updated(self):
        api = FakeAPI()
        api.collections["issues/42/comments"] = [comment(report())]
        timings.publish(api, report(120))
        self.assertEqual(api.writes[0][:2], ("PATCH", "issues/comments/55"))
        self.assertIn("No jobs with comparable samples met", api.writes[0][2]["body"])

    def test_partial_rerun_does_not_clear_unmeasured_warning(self):
        api = FakeAPI()
        api.collections["issues/42/comments"] = [comment(report())]
        current = report(120)
        current["jobs"] = []
        current["attempt"] = 2
        api.objects["actions/runs/100"]["run_attempt"] = 2
        timings.publish(api, current)
        self.assertEqual(api.writes[0][0], "PATCH")
        self.assertIn(
            "Earlier warnings at this commit were not reassessed",
            api.writes[0][2]["body"],
        )
        self.assertIn("4m 00s against a 2m 00s median", api.writes[0][2]["body"])

    def test_new_warning_does_not_hide_unmeasured_earlier_warning(self):
        api = FakeAPI()
        api.collections["issues/42/comments"] = [comment(report())]
        current = report()
        current["jobs"][0]["key"] = "another-job"
        current["jobs"][0]["name"] = "test / linux"
        timings.publish(api, current)
        body = api.writes[0][2]["body"]
        self.assertIn("test / linux", body)
        self.assertIn("Earlier warnings at this commit were not reassessed", body)
        self.assertIn("bench / walltime", body)

    def test_new_commit_prevents_write(self):
        api = FakeAPI()
        api.objects["pulls/42"]["head"]["sha"] = "b" * 40
        timings.publish(api, report())
        self.assertEqual(api.writes, [])

    def test_closed_pr_prevents_write(self):
        api = FakeAPI()
        api.objects["pulls/42"]["state"] = "closed"
        timings.publish(api, report())
        self.assertEqual(api.writes, [])

    def test_newer_run_prevents_write(self):
        api = FakeAPI()
        newer = run(101)
        newer["status"] = "in_progress"
        newer["conclusion"] = None
        api.collections["actions/workflows/10/runs"] = [newer]
        timings.publish(api, report())
        self.assertEqual(api.writes, [])

    def test_retry_started_during_analysis_prevents_write(self):
        api = FakeAPI()
        api.objects["actions/runs/100"]["run_attempt"] = 2
        timings.publish(api, report())
        self.assertEqual(api.writes, [])

    def test_historical_report_cannot_be_published(self):
        result = report()
        result["historical"] = True
        with self.assertRaisesRegex(ValueError, "main-branch"):
            timings.publish(FakeAPI(), result)

    def test_upstream_and_feature_branch_cannot_publish(self):
        with (
            patch.dict("os.environ", {"GITHUB_REPOSITORY": "astral-sh/uv"}),
            self.assertRaises(ValueError),
        ):
            timings.publish(FakeAPI(), report())
        with (
            patch.dict("os.environ", {"GITHUB_REF": "refs/heads/feature"}),
            self.assertRaises(ValueError),
        ):
            timings.publish(FakeAPI(), report())


class GitHubTests(unittest.TestCase):
    def test_transient_read_failure_is_retried(self):
        responses = [
            subprocess.CompletedProcess([], 1, "", "gh: HTTP 502"),
            subprocess.CompletedProcess([], 0, "{}"),
        ]
        with (
            patch.object(subprocess, "run", side_effect=responses) as execute,
            patch.object(timings.time, "sleep"),
        ):
            self.assertEqual(timings.GitHub().request("actions/runs/100"), {})
        self.assertEqual(execute.call_count, 2)

    def test_comment_write_is_not_retried_after_server_error(self):
        with (
            patch.dict("os.environ", {"COMMENT_TOKEN": "write"}),
            patch.object(
                subprocess,
                "run",
                return_value=subprocess.CompletedProcess([], 1, "", "gh: HTTP 502"),
            ) as execute,
            self.assertRaisesRegex(RuntimeError, "HTTP 502"),
        ):
            timings.GitHub().request(
                "issues/42/comments", method="POST", body={"body": "report"}
            )
        self.assertEqual(execute.call_count, 1)

    def test_jobs_are_paginated(self):
        first = [{"id": index} for index in range(100)]
        api = timings.GitHub()
        with patch.object(
            api, "request", side_effect=[{"jobs": first}, {"jobs": [{"id": 100}]}]
        ) as request:
            self.assertEqual(
                len(list(api.items("actions/runs/1/attempts/1/jobs", "jobs"))), 101
            )
        self.assertIn("page=2", request.call_args_list[1].args[0])

    def test_writes_use_comment_token_and_json_stdin(self):
        with (
            patch.dict("os.environ", {"GH_TOKEN": "read", "COMMENT_TOKEN": "write"}),
            patch.object(
                subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "{}")
            ) as execute,
        ):
            timings.GitHub().request(
                "issues/42/comments",
                method="POST",
                body={"body": "literal `text`\nnext"},
            )
        self.assertEqual(execute.call_args.kwargs["env"]["GH_TOKEN"], "write")
        self.assertEqual(
            json.loads(execute.call_args.kwargs["input"]),
            {"body": "literal `text`\nnext"},
        )


if __name__ == "__main__":
    unittest.main()
