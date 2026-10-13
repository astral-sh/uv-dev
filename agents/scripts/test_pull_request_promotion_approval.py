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

    def add_artifact(
        self, record=None, *, artifact_id=10, extra_file=False, candidate=False
    ):
        record = copy.deepcopy(self.record if record is None else record)
        content = io.BytesIO()
        with zipfile.ZipFile(content, "w") as archive:
            archive.writestr(
                "candidate.json" if candidate else "approval.json", json.dumps(record)
            )
            if extra_file:
                archive.writestr("unexpected.json", "{}")
        data = content.getvalue()
        name = (
            APPROVAL.candidate_artifact_name(
                REPOSITORY_ID, 101, HEAD, record["candidate"]["head_sha"]
            )
            if candidate
            else APPROVAL.artifact_name(REPOSITORY_ID, 101, 11)
        )
        artifact = {
            "id": artifact_id,
            "name": name,
            "size_in_bytes": len(data),
            "expired": False,
            "expires_at": "2099-01-01T00:00:00Z",
            "created_at": "2026-01-01T12:00:03Z",
            "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
            "workflow_run": {
                "id": record["run"]["id"],
                "repository_id": REPOSITORY_ID,
                "head_repository_id": REPOSITORY_ID,
                "head_branch": "main",
                "head_sha": WORKFLOW_HEAD,
            },
        }
        self.archives[f"repos/{REPOSITORY}/actions/artifacts/{artifact_id}/zip"] = data
        path = f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100"
        self.api_values.setdefault(path, [{"artifacts": []}])[0]["artifacts"].append(
            artifact
        )
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

    def check_source(self, **changes):
        expected = {
            "repository-id": str(REPOSITORY_ID),
            "head-sha": HEAD,
            "head-ref": "feature",
            "approval-id": "11",
            "approval-event": "labeled",
            "promotion-label": "bot:automerge",
            "promoter": "maintainer",
        } | changes
        return self.invoke(
            "source",
            *(value for key, item in expected.items() for value in ("--" + key, item)),
        )

    def test_source_rechecks_label_approved_draft_without_loading_artifacts(self):
        self.assertEqual(self.check_source(), self.source)
        self.assertFalse(any("/actions/" in endpoint for endpoint, _ in self.requests))

    def test_source_rejects_replaced_repository_identity(self):
        with self.assertRaisesRegex(
            APPROVAL.ApprovalError, "repository identity changed"
        ):
            self.check_source(**{"repository-id": "999"})

    def test_source_rejects_promotion_branches(self):
        self.source["head"]["ref"] = "automations/promote/101"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "cannot be promoted again"):
            self.check_source(**{"head-ref": "automations/promote/101"})

    def test_source_rejects_changed_approval_identity(self):
        for key, value in {
            "approval-id": "12",
            "approval-event": "ready_for_review",
            "promotion-label": "bot:promote",
            "promoter": "another-maintainer",
        }.items():
            with (
                self.subTest(field=key),
                self.assertRaisesRegex(APPROVAL.ApprovalError, "event changed"),
            ):
                self.check_source(**{key: value})

    def test_source_rejects_changed_revision_or_branch(self):
        for key, value, message in (
            ("head-sha", "b" * 40, "head changed"),
            ("head-ref", "renamed", "branch changed"),
        ):
            with (
                self.subTest(field=key),
                self.assertRaisesRegex(APPROVAL.ApprovalError, message),
            ):
                self.check_source(**{key: value})

    def test_source_checks_repository_identity_and_current_label(self):
        for side in ("base", "head"):
            for key, value in (("id", 999), ("full_name", "another/repository")):
                with self.subTest(side=side, field=key):
                    changed = copy.deepcopy(self.source)
                    changed[side]["repo"] = changed[side]["repo"] | {key: value}
                    with (
                        patch.dict(
                            self.api_values, {f"repos/{REPOSITORY}/pulls/101": changed}
                        ),
                        self.assertRaisesRegex(APPROVAL.ApprovalError, "repositories"),
                    ):
                        self.check_source()
        self.source["labels"] = []
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "label was removed"):
            self.check_source()

    def test_source_refetches_state_at_each_boundary(self):
        self.assertEqual(self.check_source()["state"], "open")
        self.source["state"] = "closed"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "no longer open"):
            self.check_source()
        self.assertEqual(
            self.requests.count((f"repos/{REPOSITORY}/pulls/101", False)), 2
        )

    def test_source_rechecks_ready_approval_draft_eligibility(self):
        self.event["event"] = "ready_for_review"
        self.source["draft"] = False
        self.assertFalse(
            self.check_source(
                **{
                    "approval-event": "ready_for_review",
                    "promotion-label": "bot:promote",
                }
            )["draft"]
        )
        self.source["draft"] = True
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "no longer approved"):
            self.check_source(
                **{
                    "approval-event": "ready_for_review",
                    "promotion-label": "bot:promote",
                }
            )

    def candidate_setup(self):
        self.source["base"]["ref"] = "stack/parent"
        self.source["base"]["sha"] = "b" * 40
        self.plan = {
            "number": 101,
            "promoter": "maintainer",
            "approval_id": "11",
            "approval_event": "labeled",
            "promotion_label": "bot:automerge",
            "head_ref": "feature",
            "head_sha": HEAD,
            "existing_head_sha": "",
            "source_base": "stack/parent",
            "source_base_sha": "b" * 40,
            "base_ref": "main",
            "base_sha": "d" * 40,
            "previous_sha": "b" * 40,
            "merge_sha": "c" * 40,
            "merged_parent_number": "1000",
            "action": "rebase",
        }
        self.candidate_record = copy.deepcopy(self.record) | {
            "schema": 2,
            "candidate": {
                "head_sha": "6" * 40,
                **{key: self.plan[key] for key in APPROVAL.CANDIDATE_CONTEXT},
            },
        }
        self.job.update(
            name=f"Promote {REPOSITORY}#101",
            conclusion="failure",
            completed_at="2026-01-01T12:00:08Z",
        )
        self.job["steps"] = [
            {
                "name": name,
                "number": number,
                "status": "completed",
                "conclusion": "success",
                "started_at": f"2026-01-01T12:00:0{number}Z",
                "completed_at": f"2026-01-01T12:00:0{number + 1}Z",
            }
            for number, name in enumerate(
                (
                    "Verify promotion revision",
                    "Recheck promotion plan",
                    "Capture verified promotion candidate",
                    "Persist verified promotion candidate",
                ),
                2,
            )
        ]

        candidate_name = APPROVAL.candidate_artifact_name(
            REPOSITORY_ID, 101, HEAD, "6" * 40
        )
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={candidate_name}&per_page=100"
        ] = [{"artifacts": []}]

    def add_candidate(self, record=None, **kwargs):
        artifact = self.add_artifact(
            self.candidate_record if record is None else record,
            candidate=True,
            **kwargs,
        )
        artifact["created_at"] = "2026-01-01T12:00:05Z"
        return artifact

    def load_candidate(self, *, candidate_sha="6" * 40):
        name = APPROVAL.candidate_artifact_name(REPOSITORY_ID, 101, HEAD, candidate_sha)
        self.api_values.setdefault(
            f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100",
            [{"artifacts": []}],
        )
        return self.invoke(
            "load-candidate",
            "--repository-id",
            str(REPOSITORY_ID),
            "--plan",
            json.dumps(self.plan),
            "--candidate-sha",
            candidate_sha,
        )

    def test_verified_candidate_survives_later_writer_failure(self):
        self.candidate_setup()
        self.add_candidate()
        self.assertEqual(self.load_candidate(), self.candidate_record)
        self.assertIn((self.jobs_path, True), self.requests)

    def test_candidate_can_originate_from_a_trusted_push_sweep(self):
        self.candidate_setup()
        self.add_candidate()
        self.run["event"] = "push"
        self.assertEqual(self.load_candidate(), self.candidate_record)

    def test_new_approval_mode_can_reuse_only_the_old_transformation(self):
        self.candidate_setup()
        self.add_candidate()
        self.event["id"] = 12
        self.event["label"]["name"] = "bot:promote"
        self.event["actor"]["login"] = "another-maintainer"
        self.event["actor"]["id"] = 456
        self.source["labels"] = [{"name": "bot:promote"}]
        self.plan.update(
            approval_id="12",
            promotion_label="bot:promote",
            promoter="another-maintainer",
        )
        result = self.load_candidate()
        self.assertEqual(result["candidate"]["head_sha"], "6" * 40)
        self.assertEqual(result["approval"]["id"], 11)
        self.assertEqual(result["approval"]["promotion_label"], "bot:automerge")

    def test_candidate_does_not_supply_current_publication_authority(self):
        self.candidate_setup()
        self.add_candidate()
        self.event["actor"]["type"] = "Bot"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No current human"):
            self.load_candidate()

    def test_candidate_rejects_changed_source_revision_and_branch(self):
        self.candidate_setup()
        self.add_candidate()
        original = copy.deepcopy(self.source)
        for key, value in (("sha", "9" * 40), ("ref", "renamed")):
            changed = copy.deepcopy(original)
            changed["head"][key] = value
            with (
                self.subTest(field=key),
                patch.dict(self.api_values, {f"repos/{REPOSITORY}/pulls/101": changed}),
                self.assertRaisesRegex(APPROVAL.ApprovalError, "changed"),
            ):
                self.load_candidate()

    def test_candidate_rejects_moved_source_parent(self):
        self.candidate_setup()
        self.add_candidate()
        self.source["base"]["sha"] = "9" * 40
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "source parent changed"):
            self.load_candidate()

    def test_candidate_rejects_changed_parent_merge_context(self):
        self.candidate_setup()
        self.add_candidate()
        for key, value in (
            ("previous_sha", "9" * 40),
            ("merge_sha", "9" * 40),
            ("merged_parent_number", "1002"),
            ("base_ref", "another-base"),
        ):
            original = self.plan[key]
            self.plan[key] = value
            with (
                self.subTest(field=key),
                self.assertRaisesRegex(
                    APPROVAL.ApprovalError, "parent or target branch changed"
                ),
            ):
                self.load_candidate()
            self.plan[key] = original

    def test_candidate_allows_only_verified_target_base_advancement(self):
        self.candidate_setup()
        self.add_candidate()
        self.plan["base_sha"] = "e" * 40
        self.api_values[f"repos/{REPOSITORY}/compare/{'d' * 40}...{'e' * 40}"] = {
            "status": "ahead"
        }
        self.assertEqual(self.load_candidate()["candidate"]["base_sha"], "d" * 40)

    def test_candidate_rejects_rewritten_target_base(self):
        self.candidate_setup()
        self.add_candidate()
        self.plan["base_sha"] = "e" * 40
        self.api_values[f"repos/{REPOSITORY}/compare/{'d' * 40}...{'e' * 40}"] = {
            "status": "diverged"
        }
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "not an ancestor"):
            self.load_candidate()

    def test_candidate_rejects_a_different_remote_branch_revision(self):
        self.candidate_setup()
        self.add_candidate()
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No trusted candidate"):
            self.load_candidate(candidate_sha="7" * 40)

    def test_missing_candidate_proof_does_not_allow_branch_reuse(self):
        self.candidate_setup()
        name = APPROVAL.candidate_artifact_name(REPOSITORY_ID, 101, HEAD, "6" * 40)
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100"
        ] = [{"artifacts": []}]
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No trusted candidate"):
            self.load_candidate()

    def test_expired_candidate_proof_does_not_allow_branch_reuse(self):
        self.candidate_setup()
        self.add_candidate()["expired"] = True
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No trusted candidate"):
            self.load_candidate()

    def test_conflicting_candidate_proof_is_rejected(self):
        self.candidate_setup()
        self.add_candidate()
        changed = copy.deepcopy(self.candidate_record)
        changed["candidate"]["base_sha"] = "e" * 40
        self.add_candidate(changed, artifact_id=20)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "Conflicting candidate"):
            self.load_candidate()

    def test_untrusted_candidate_artifact_cannot_block_valid_proof(self):
        self.candidate_setup()
        self.add_candidate()
        untrusted = self.add_candidate(artifact_id=20, extra_file=True)
        untrusted["workflow_run"]["id"] = 2000
        self.api_values[f"repos/{REPOSITORY}/actions/runs/2000"] = self.run | {
            "id": 2000,
            "workflow_id": 333,
        }
        self.assertEqual(self.load_candidate(), self.candidate_record)

    def test_candidate_requires_all_proof_steps_to_succeed(self):
        self.candidate_setup()
        self.add_candidate()
        for step in self.job["steps"]:
            step["conclusion"] = "failure"
            with (
                self.subTest(step=step["name"]),
                self.assertRaisesRegex(
                    APPROVAL.ApprovalError,
                    "No trusted candidate"
                    if step["name"] == "Persist verified promotion candidate"
                    else "must all succeed",
                ),
            ):
                self.load_candidate()
            step["conclusion"] = "success"

    def test_candidate_requires_artifact_inside_its_upload_step(self):
        self.candidate_setup()
        self.add_candidate()["created_at"] = "2026-01-01T12:00:07Z"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "during its upload step"):
            self.load_candidate()

    def test_candidate_requires_proof_steps_in_order(self):
        self.candidate_setup()
        self.add_candidate()
        self.job["steps"][-1]["number"] = 1
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "unexpected order"):
            self.load_candidate()

    def test_candidate_rejects_extra_archive_payload(self):
        self.candidate_setup()
        self.add_candidate(extra_file=True)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "only candidate.json"):
            self.load_candidate()

    def test_candidate_rejects_proof_that_permitted_generated_edits(self):
        self.candidate_setup()
        self.candidate_record["schema"] = 1
        self.add_candidate()
        with self.assertRaisesRegex(
            APPROVAL.ApprovalError, "Unsupported approval record"
        ):
            self.load_candidate()

    def test_clean_candidate_namespace_does_not_load_legacy_proof(self):
        self.candidate_setup()
        self.candidate_record["schema"] = 1
        artifact = self.add_candidate()
        current_name = artifact["name"]
        legacy_name = current_name.replace(
            "promotion-clean-candidate-", "promotion-candidate-"
        )
        artifact["name"] = legacy_name
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={legacy_name}&per_page=100"
        ] = [{"artifacts": [artifact]}]
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={current_name}&per_page=100"
        ] = [{"artifacts": []}]
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No trusted candidate"):
            self.load_candidate()
        self.assertFalse(any(legacy_name in endpoint for endpoint, _ in self.requests))

    def test_candidate_rejects_invalid_historical_approval_actor(self):
        self.candidate_setup()
        self.candidate_record["approval"]["actor"]["type"] = "Bot"
        self.add_candidate()
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "approval actor"):
            self.load_candidate()

    def capture_candidate(self, candidate_sha="6" * 40):
        real_run = APPROVAL.subprocess.run

        def run(command, **kwargs):
            if command[:2] == ["git", "merge-base"]:
                return None
            return real_run(command, **kwargs)

        with patch.object(APPROVAL.subprocess, "run", side_effect=run) as calls:
            result = self.invoke(
                "capture-candidate",
                "--repository-id",
                str(REPOSITORY_ID),
                "--plan",
                json.dumps(self.plan),
                "--candidate-sha",
                candidate_sha,
            )
        return result, calls.call_args_list

    def test_candidate_capture_checks_both_git_ancestry_boundaries(self):
        self.candidate_setup()
        result, calls = self.capture_candidate()
        self.assertEqual(result["record"], self.candidate_record)
        self.assertTrue(result["upload"])
        ancestry = [
            call.args[0] for call in calls if call.args[0][:2] == ["git", "merge-base"]
        ]
        self.assertEqual(
            ancestry,
            [
                ["git", "merge-base", "--is-ancestor", "d" * 40, "6" * 40],
                ["git", "merge-base", "--is-ancestor", "c" * 40, "d" * 40],
            ],
        )

    def test_candidate_capture_reuses_a_verified_prior_attempt(self):
        self.candidate_setup()
        self.add_candidate()
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        self.run["run_attempt"] = 2
        result, _ = self.capture_candidate()
        self.assertFalse(result["upload"])
        self.assertFalse(result["overwrite"])
        self.assertEqual(result["record"]["run"]["attempt"], 1)
        self.assertEqual(result["record"], self.candidate_record)

    def test_incomplete_other_candidate_cannot_block_the_remote_revision(self):
        self.candidate_setup()
        self.add_candidate()
        other = copy.deepcopy(self.candidate_record)
        other["candidate"]["head_sha"] = "7" * 40
        other["run"]["id"] = 2000
        self.add_candidate(other, artifact_id=20)
        self.api_values[f"repos/{REPOSITORY}/actions/runs/2000"] = self.run | {
            "id": 2000
        }
        incomplete = copy.deepcopy(self.job)
        incomplete["steps"][-1]["conclusion"] = "cancelled"
        self.api_values[
            f"repos/{REPOSITORY}/actions/runs/2000/attempts/1/jobs?per_page=100"
        ] = [{"jobs": [incomplete]}]
        self.assertEqual(self.load_candidate(), self.candidate_record)
        self.assertFalse(any("/runs/2000" in endpoint for endpoint, _ in self.requests))

    def test_different_candidate_rerun_uses_a_distinct_artifact_name(self):
        self.candidate_setup()
        original = self.add_candidate()
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        self.run["run_attempt"] = 2
        name = APPROVAL.candidate_artifact_name(REPOSITORY_ID, 101, HEAD, "7" * 40)
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100"
        ] = [{"artifacts": []}]
        result, _ = self.capture_candidate("7" * 40)
        self.assertTrue(result["upload"])
        self.assertEqual(result["artifact_name"], name)
        self.assertNotEqual(result["artifact_name"], original["name"])
        self.assertEqual(result["record"]["run"]["attempt"], 2)

    def test_current_candidate_name_cannot_hide_a_different_payload_revision(self):
        self.candidate_setup()
        other = copy.deepcopy(self.candidate_record)
        other["candidate"]["head_sha"] = "7" * 40
        artifact = self.add_candidate(other)
        name = APPROVAL.candidate_artifact_name(REPOSITORY_ID, 101, HEAD, "6" * 40)
        artifact["name"] = name
        self.api_values[
            f"repos/{REPOSITORY}/actions/artifacts?name={name}&per_page=100"
        ] = [{"artifacts": [artifact]}]
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "another revision"):
            self.load_candidate()

    def test_fresh_capture_replaces_incomplete_upload_from_earlier_attempt(self):
        self.candidate_setup()
        self.add_candidate()
        self.job["steps"][-1]["conclusion"] = "cancelled"
        self.run["run_attempt"] = 2
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        result, _ = self.capture_candidate()
        self.assertTrue(result["upload"])
        self.assertTrue(result["overwrite"])
        self.assertEqual(result["record"]["run"]["attempt"], 2)

    def test_incomplete_upload_cannot_authorize_candidate_loading(self):
        self.candidate_setup()
        self.add_candidate()
        self.job["steps"][-1]["conclusion"] = "cancelled"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "No trusted candidate"):
            self.load_candidate()

    def test_new_valid_upload_can_coexist_with_an_incomplete_old_run(self):
        self.candidate_setup()
        old = copy.deepcopy(self.candidate_record)
        old["run"]["id"] = 2000
        self.add_candidate(old)
        self.api_values[f"repos/{REPOSITORY}/actions/runs/2000"] = self.run | {
            "id": 2000
        }
        incomplete = copy.deepcopy(self.job)
        incomplete["steps"][-1]["conclusion"] = "cancelled"
        self.api_values[
            f"repos/{REPOSITORY}/actions/runs/2000/attempts/1/jobs?per_page=100"
        ] = [{"jobs": [incomplete]}]
        result, _ = self.capture_candidate()
        self.assertTrue(result["upload"])
        self.assertFalse(result["overwrite"])
        self.add_candidate(result["record"], artifact_id=20)
        self.assertEqual(self.load_candidate(), result["record"])

    def test_fresh_capture_cannot_replace_an_incomplete_current_attempt(self):
        self.candidate_setup()
        self.add_candidate()
        self.job["steps"][-1]["conclusion"] = "cancelled"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "earlier attempt"):
            self.capture_candidate()

    def test_fresh_capture_cannot_replace_a_different_incomplete_transformation(self):
        self.candidate_setup()
        old = copy.deepcopy(self.candidate_record)
        old["candidate"]["base_sha"] = "8" * 40
        self.add_candidate(old)
        self.job["steps"][-1]["conclusion"] = "cancelled"
        self.run["run_attempt"] = 2
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        with self.assertRaisesRegex(
            APPROVAL.ApprovalError, "identical incomplete candidate"
        ):
            self.capture_candidate()

    def test_incomplete_upload_with_wrong_timing_is_not_replaceable(self):
        self.candidate_setup()
        self.add_candidate()["created_at"] = "2026-01-01T12:00:07Z"
        self.job["steps"][-1]["conclusion"] = "cancelled"
        self.run["run_attempt"] = 2
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "during its upload step"):
            self.capture_candidate()

    def test_fresh_capture_does_not_replace_failed_verification_proof(self):
        self.candidate_setup()
        self.add_candidate()
        self.job["steps"][0]["conclusion"] = "failure"
        self.run["run_attempt"] = 2
        os.environ["GITHUB_RUN_ATTEMPT"] = "2"
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "must all succeed"):
            self.capture_candidate()

    def test_candidate_capture_cannot_relabel_a_reused_revision(self):
        self.candidate_setup()
        self.plan.update(action="promote", existing_head_sha="6" * 40)
        with self.assertRaisesRegex(APPROVAL.ApprovalError, "newly verified rebases"):
            self.invoke(
                "capture-candidate",
                "--repository-id",
                str(REPOSITORY_ID),
                "--plan",
                json.dumps(self.plan),
                "--candidate-sha",
                "6" * 40,
            )

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
