"""Offline checks for the authority carried by promotion approval artifacts."""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import sys
import unittest
import zipfile
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "promotion_approval",
    pathlib.Path(__file__).with_name("pull-request-promotion-approval.py"),
)
APPROVAL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(APPROVAL)

REPOSITORY = "astral-sh/uv-dev"
REPOSITORY_ID = 1302176231
WORKFLOW_ID = 222
HEAD = "a" * 40
WORKFLOW_HEAD = "f" * 40


class PromotionApprovalTests(unittest.TestCase):
    def setUp(self):
        self.repository = {"id": REPOSITORY_ID, "full_name": REPOSITORY}
        self.event = {
            "id": 11,
            "event": "labeled",
            "label": {"name": "bot:automerge"},
            "actor": {"id": 123, "login": "maintainer", "type": "User"},
            "created_at": "2026-01-01T12:00:00Z",
        }
        self.approval = {
            key: copy.deepcopy(value)
            for key, value in self.event.items()
            if key != "label"
        } | {"promotion_label": "bot:automerge"}
        self.source = {
            "number": 101,
            "state": "open",
            "draft": True,
            "labels": [{"name": "bot:automerge"}],
            "head": {"ref": "feature", "sha": HEAD, "repo": self.repository},
            "base": {"ref": "main", "sha": WORKFLOW_HEAD, "repo": self.repository},
        }
        self.run = {
            "id": 1000,
            "run_attempt": 1,
            "workflow_id": WORKFLOW_ID,
            "path": APPROVAL.WORKFLOW_PATH,
            "event": "workflow_dispatch",
            "head_branch": "main",
            "head_sha": WORKFLOW_HEAD,
            "repository": self.repository,
            "head_repository": self.repository,
            "created_at": "2026-01-01T12:00:01Z",
        }
        self.record = {
            "schema": 1,
            "repository": self.repository,
            "number": 101,
            "head_ref": "feature",
            "head_sha": HEAD,
            "approval": self.approval,
            "run": {
                "id": 1000,
                "attempt": 1,
                "workflow_id": WORKFLOW_ID,
                "head_sha": WORKFLOW_HEAD,
            },
        }
        self.events_path = f"repos/{REPOSITORY}/issues/101/events?per_page=100"
        name = APPROVAL.artifact_name(REPOSITORY_ID, 101, 11)
        self.artifacts_path = (
            f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100"
        )
        self.jobs_path = (
            f"repos/{REPOSITORY}/actions/runs/1000/attempts/1/jobs?per_page=100"
        )
        self.job = {
            "name": APPROVAL.CAPTURE_JOB,
            "status": "completed",
            "conclusion": "success",
            "head_sha": WORKFLOW_HEAD,
            "started_at": "2026-01-01T12:00:02Z",
            "completed_at": "2026-01-01T12:00:04Z",
        }
        self.api_values = {
            self.events_path: [[self.event]],
            self.artifacts_path: [{"artifacts": []}],
            self.jobs_path: [{"jobs": [self.job]}],
            f"repos/{REPOSITORY}": self.repository,
            f"repos/{REPOSITORY}/pulls/101": self.source,
            f"repos/{REPOSITORY}/actions/workflows/promote-pull-request.yml": {
                "id": WORKFLOW_ID
            },
            f"repos/{REPOSITORY}/actions/runs/1000": self.run,
        }
        self.archives = {}
        self.requests = []
        self.environment = {
            "GITHUB_EVENT_NAME": "workflow_dispatch",
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_REPOSITORY": REPOSITORY,
            "GITHUB_REPOSITORY_ID": str(REPOSITORY_ID),
            "GITHUB_RUN_ID": "1000",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_SHA": WORKFLOW_HEAD,
            "PATH": os.environ.get("PATH", ""),
        }
        self.addCleanup(patch.stopall)
        patch.object(APPROVAL, "gh_json", side_effect=self.api).start()
        patch.object(
            APPROVAL.subprocess, "check_output", side_effect=self.download
        ).start()
        patch.dict(os.environ, self.environment, clear=True).start()

    def api(self, endpoint, *, paginate=False):
        self.requests.append((endpoint, paginate))
        self.assertIn(
            endpoint,
            self.api_values,
            "Unexpected API call; network access is forbidden",
        )
        return copy.deepcopy(self.api_values[endpoint])

    def download(self, command):
        self.assertEqual(command[:2], ["gh", "api"])
        self.assertEqual(len(command), 3)
        self.assertIn(
            command[2],
            self.archives,
            "Unexpected download; network access is forbidden",
        )
        return self.archives[command[2]]

    def add_artifact(self, record=None, *, artifact_id=10, extra_file=False):
        record = copy.deepcopy(self.record if record is None else record)
        content = io.BytesIO()
        with zipfile.ZipFile(content, "w") as archive:
            archive.writestr("approval.json", json.dumps(record))
            if extra_file:
                archive.writestr("unexpected.json", "{}")
        data = content.getvalue()
        artifact = {
            "id": artifact_id,
            "name": APPROVAL.artifact_name(REPOSITORY_ID, 101, 11),
            "size_in_bytes": len(data),
            "expired": False,
            "expires_at": "2099-01-01T00:00:00Z",
            "created_at": "2026-01-01T12:00:03Z",
            "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
            "workflow_run": {
                "id": 1000,
                "repository_id": REPOSITORY_ID,
                "head_repository_id": REPOSITORY_ID,
                "head_branch": "main",
                "head_sha": WORKFLOW_HEAD,
            },
        }
        self.archives[f"repos/{REPOSITORY}/actions/artifacts/{artifact_id}/zip"] = data
        self.api_values[self.artifacts_path][0]["artifacts"].append(artifact)
        return artifact

    def stored(self):
        return APPROVAL.recorded_approval(
            REPOSITORY, REPOSITORY_ID, WORKFLOW_ID, 101, self.approval
        )

    def invoke(self, command, *args):
        output = io.StringIO()
        with (
            patch.object(
                sys, "argv", ["approval.py", command, REPOSITORY, "101", *args]
            ),
            contextlib.redirect_stdout(output),
        ):
            APPROVAL.main()
        return json.loads(output.getvalue())

    def test_capture_retains_explicit_revision_and_automerge_mode(self):
        result = self.invoke(
            "capture", "--head-sha", HEAD, "--promotion-label", "bot:automerge"
        )
        self.assertEqual(result["record"], self.record)
        self.assertTrue(result["upload"])

    def test_capture_cannot_run_during_push_sweep(self):
        os.environ["GITHUB_EVENT_NAME"] = "push"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "explicit dispatch"):
            self.invoke(
                "capture", "--head-sha", HEAD, "--promotion-label", "bot:automerge"
            )

    def test_capture_rejects_newer_approval_after_dispatch(self):
        self.event["created_at"] = "2026-01-01T12:00:05Z"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "newer approval"):
            self.invoke(
                "capture", "--head-sha", HEAD, "--promotion-label", "bot:automerge"
            )

    def test_capture_rejects_changed_dispatch_head(self):
        self.source["head"]["sha"] = "b" * 40
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "head changed"):
            self.invoke(
                "capture", "--head-sha", HEAD, "--promotion-label", "bot:automerge"
            )

    def test_capture_rejects_superseded_mode(self):
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "mode was superseded"):
            self.invoke(
                "capture", "--head-sha", HEAD, "--promotion-label", "bot:promote"
            )

    def test_capture_reuses_identical_record(self):
        self.add_artifact()
        self.assertFalse(
            self.invoke(
                "capture", "--head-sha", HEAD, "--promotion-label", "bot:automerge"
            )["upload"]
        )

    def test_capture_cannot_rebind_an_existing_approval_to_another_head(self):
        self.add_artifact()
        self.source["head"]["sha"] = "b" * 40
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "another revision"):
            self.invoke(
                "capture", "--head-sha", "b" * 40, "--promotion-label", "bot:automerge"
            )

    def test_sweep_rejects_head_changed_since_approval(self):
        self.add_artifact()
        self.source["head"]["sha"] = "b" * 40
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "head changed"):
            self.invoke("load")

    def test_sweep_rejects_renamed_source_branch(self):
        self.add_artifact()
        self.source["head"]["ref"] = "renamed"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "branch changed"):
            self.invoke("load")

    def test_missing_record_requires_explicit_approval(self):
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "fresh explicit"):
            self.invoke("load")

    def test_expired_record_requires_explicit_approval(self):
        self.add_artifact()["expired"] = True
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "fresh explicit"):
            self.invoke("load")

    def test_elapsed_expiry_is_not_accepted_as_live(self):
        self.add_artifact()["expires_at"] = "2020-01-01T00:00:00Z"
        self.assertIsNone(self.stored())

    def test_conflicting_live_records_fail_closed(self):
        self.add_artifact()
        changed = copy.deepcopy(self.record)
        changed["head_sha"] = "b" * 40
        self.add_artifact(changed, artifact_id=20)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "Conflicting"):
            self.stored()

    def test_artifacts_and_events_are_paginated(self):
        earlier = copy.deepcopy(self.event)
        earlier.update(
            id=10, event="ready_for_review", created_at="2026-01-01T11:59:00Z"
        )
        self.api_values[self.events_path] = [[earlier], [self.event]]
        first = self.add_artifact()
        second = self.add_artifact(artifact_id=20)
        self.api_values[self.artifacts_path] = [
            {"artifacts": [first]},
            {"artifacts": [second]},
        ]
        result = self.invoke("load")
        self.assertEqual(result["approval"]["promotion_label"], "bot:automerge")
        self.assertIn((self.events_path, True), self.requests)
        self.assertIn((self.artifacts_path, True), self.requests)
        self.assertIn((self.jobs_path, True), self.requests)

    def test_removed_label_does_not_resurrect_older_ready_approval(self):
        ready = copy.deepcopy(self.event)
        ready.update(id=10, event="ready_for_review", created_at="2026-01-01T11:59:00Z")
        removed = copy.deepcopy(self.event)
        removed.update(id=12, event="unlabeled", created_at="2026-01-01T12:00:05Z")
        self.api_values[self.events_path] = [[ready, self.event, removed]]
        self.assertIsNone(APPROVAL.current_approval(REPOSITORY, 101))

    def test_removing_another_mode_label_keeps_the_current_approval(self):
        removed = copy.deepcopy(self.event)
        removed.update(id=12, event="unlabeled", created_at="2026-01-01T12:00:05Z")
        removed["label"]["name"] = "bot:promote"
        self.api_values[self.events_path] = [[self.event, removed]]
        self.assertEqual(APPROVAL.current_approval(REPOSITORY, 101), self.approval)

    def test_recovery_draft_transition_retains_label_approval_identity(self):
        returned = copy.deepcopy(self.event)
        returned.update(
            id=12, event="converted_to_draft", created_at="2026-01-01T12:00:05Z"
        )
        self.api_values[self.events_path] = [[self.event, returned]]
        self.assertEqual(APPROVAL.current_approval(REPOSITORY, 101), self.approval)

    def test_ready_approval_does_not_authorize_a_current_draft(self):
        approval = self.approval | {
            "event": "ready_for_review",
            "promotion_label": "bot:promote",
        }
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "no longer approved"):
            APPROVAL.eligible_source(REPOSITORY, REPOSITORY_ID, 101, approval, HEAD)

    def test_bot_event_is_not_a_human_approval(self):
        self.event["actor"]["type"] = "Bot"
        self.assertIsNone(APPROVAL.current_approval(REPOSITORY, 101))

    def test_artifact_from_another_workflow_is_rejected(self):
        self.add_artifact()
        self.run["workflow_id"] = 333
        self.assertIsNone(self.stored())

    def test_artifact_from_push_run_is_rejected(self):
        self.add_artifact()
        self.run["event"] = "push"
        self.assertIsNone(self.stored())

    def test_artifact_from_non_main_branch_is_rejected(self):
        self.add_artifact()
        self.run["head_branch"] = "feature"
        self.assertIsNone(self.stored())

    def test_artifact_from_another_repository_is_rejected(self):
        artifact = self.add_artifact()
        artifact["workflow_run"]["repository_id"] = 1234
        self.assertIsNone(self.stored())

    def test_unrelated_workflow_cannot_block_a_valid_same_name_record(self):
        self.add_artifact()
        unrelated = self.add_artifact(artifact_id=20, extra_file=True)
        unrelated["workflow_run"]["id"] = 2000
        self.api_values[f"repos/{REPOSITORY}/actions/runs/2000"] = self.run | {
            "id": 2000,
            "workflow_id": 333,
            "path": ".github/workflows/ci.yml",
        }
        self.assertEqual(self.stored(), self.record)

    def test_pr_artifact_cannot_block_a_valid_same_name_record(self):
        self.add_artifact()
        unrelated = self.add_artifact(artifact_id=20, extra_file=True)
        unrelated["workflow_run"]["head_branch"] = "feature"
        self.assertEqual(self.stored(), self.record)

    def test_failed_capture_job_is_not_authority(self):
        self.add_artifact()
        self.job["conclusion"] = "failure"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "capture job"):
            self.stored()

    def test_artifact_created_outside_capture_job_is_rejected(self):
        self.add_artifact()["created_at"] = "2026-01-01T12:00:06Z"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "during its capture job"):
            self.stored()

    def test_digest_mismatch_is_rejected(self):
        self.add_artifact()["digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "digest"):
            self.stored()

    def test_extra_archive_payload_is_rejected(self):
        self.add_artifact(extra_file=True)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "only approval.json"):
            self.stored()

    def test_wrong_source_tuple_is_rejected(self):
        record = copy.deepcopy(self.record)
        record["number"] = 102
        self.add_artifact(record)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "another source"):
            self.stored()

    def test_wrong_approval_mode_is_rejected(self):
        record = copy.deepcopy(self.record)
        record["approval"]["promotion_label"] = "bot:promote"
        self.add_artifact(record)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "current human approval"):
            self.stored()

    def test_non_integer_identity_is_rejected(self):
        record = copy.deepcopy(self.record)
        record["approval"]["id"] = 11.0
        self.add_artifact(record)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "current human approval"):
            self.stored()

    def test_unknown_record_schema_is_rejected(self):
        record = copy.deepcopy(self.record)
        record["schema"] = 2
        self.add_artifact(record)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "record version"):
            self.stored()


if __name__ == "__main__":
    unittest.main()
