import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def job(workflow: str, name: str, next_name: str | None = None) -> str:
    contents = (ROOT / ".github/workflows" / workflow).read_text()
    section = contents.split(f"  {name}:\n", 1)[1]
    return section.split(f"  {next_name}:\n", 1)[0] if next_name else section


class PromotionWorkflowBoundaryTests(unittest.TestCase):
    def test_prepare_is_a_complete_read_only_python_stage(self) -> None:
        prepare = job("promote-pull-request.yml", "prepare", "queue")
        self.assertIn("promotions prepare", prepare)
        self.assertIn('--approval-id "$EXPECTED_APPROVAL_ID"', prepare)
        self.assertIn("contents: read\n            pull_requests: read", prepare)
        self.assertNotIn("gh api", prepare)
        self.assertNotIn("pull_requests: write", prepare)
        self.assertNotIn("contents: write", prepare)

    def test_queue_record_and_dispatch_have_separate_writers(self) -> None:
        queue = job("promote-pull-request.yml", "queue", "replay-queued-promotion")
        replay = job(
            "promote-pull-request.yml",
            "replay-queued-promotion",
            "update-pull-request-parent",
        )
        self.assertIn("pull_requests: write", queue)
        self.assertNotIn("actions: write", queue)
        self.assertNotIn("contents: write", queue)
        self.assertIn("promotions record-queue", queue)
        self.assertIn("needs.queue.outputs.recorded == 'true'", replay)
        self.assertIn("actions: write", replay)
        self.assertIn("GH_TOKEN: ${{ github.token }}", replay)
        self.assertNotIn("Get promotion replay token", replay)
        self.assertNotIn("pull_requests: write", replay)
        self.assertNotIn("contents: write", replay)
        self.assertIn("promotions replay-one", replay)

    def test_sync_replays_only_after_a_successful_sync(self) -> None:
        sync = job("sync-uv-dev.yml", "sync", "replay-queued-promotions")
        replay = job("sync-uv-dev.yml", "replay-queued-promotions")
        worker = job("replay-queued-promotions.yml", "replay")
        self.assertIn("promotions sync", sync)
        self.assertIn("main-sha: ${{ steps.sync.outputs.main-sha }}", sync)
        self.assertNotIn("gh api", sync)
        self.assertIn("GH_UPSTREAM_TOKEN: ${{ github.token }}", sync)
        self.assertIn("needs: sync", replay)
        self.assertIn("${{ needs.sync.outputs.main-sha }}", replay)
        self.assertIn("uses: $/.github/workflows/replay-queued-promotions.yml", replay)
        self.assertIn("secrets: inherit", replay)
        self.assertIn("GH_SOURCE_TOKEN:", worker)
        self.assertIn("GH_UPSTREAM_TOKEN: ${{ github.token }}", worker)
        self.assertIn("promotions replay", worker)
        self.assertNotIn("pull_requests: write", worker)
        self.assertNotIn("contents: write", worker)

    def test_cross_repository_dispatch_has_a_distinct_reusable_rule(self) -> None:
        policy = json.loads((ROOT / ".github/ost-simple-sts.json").read_text())
        rules = [
            rule
            for rule in policy["rules"]
            if rule.get("reusable_workflow") == "replay-queued-promotions.yml"
        ]
        self.assertEqual(len(rules), 1)
        self.assertEqual(
            rules[0],
            {
                "caller": "uv",
                "environment": "automations",
                "caller_ref": "refs/heads/main",
                "caller_workflow": "sync-uv-dev.yml",
                "reusable_workflow": "replay-queued-promotions.yml",
                "on": ["push", "workflow_dispatch"],
                "permissions": {
                    "actions": "write",
                    "contents": "read",
                    "pull_requests": "read",
                },
                "target": "uv-dev",
                "installation": "automations",
            },
        )
        # STS chooses the first matching caller/callee rule, before checking
        # requested permissions. Direct publisher/sync rules must stay distinct.
        for rule in policy["rules"]:
            if (
                rule.get("caller_workflow")
                in {"promote-pull-request.yml", "sync-uv-dev.yml"}
                and "reusable_workflow" not in rule
            ):
                self.assertNotIn("actions", rule["permissions"])

    def test_successful_source_close_wakes_only_its_queued_children(self) -> None:
        publisher = job(
            "promote-pull-request.yml", "promote", "replay-promoted-children"
        )
        replay = job("promote-pull-request.yml", "replay-promoted-children", "recover")
        self.assertIn("closed: ${{ steps.close.outputs.closed }}", publisher)
        self.assertIn('echo "closed=true" >> "$GITHUB_OUTPUT"', publisher)
        self.assertIn("needs.promote.outputs.closed == 'true'", replay)
        self.assertIn("promotions replay-children", replay)
        self.assertIn('--parent "$PARENT_PULL_REQUEST"', replay)
        self.assertIn("GH_TOKEN: ${{ github.token }}", replay)
        self.assertNotIn("pull_requests: write", replay)
        self.assertNotIn("contents: write", replay)

    def test_replay_approval_is_rechecked_at_existing_write_boundaries(self) -> None:
        publisher = job(
            "promote-pull-request.yml", "promote", "replay-promoted-children"
        )
        recover = job("promote-pull-request.yml", "recover")
        self.assertEqual(publisher.count("promotions current-approval"), 3)
        self.assertIn("promotions current-approval", recover)
        for section in (publisher, recover):
            self.assertIn("REPLAY_APPROVAL_ID: ${{ inputs.approval_id }}", section)
            self.assertIn('--approval-id "$REPLAY_APPROVAL_ID"', section)
            self.assertNotIn('select(.event == "ready_for_review"', section)

    def test_base_copy_runs_only_in_the_existing_publisher(self) -> None:
        publisher = job(
            "promote-pull-request.yml", "promote", "replay-promoted-children"
        )
        self.assertIn("promotions ensure-base", publisher)
        self.assertIn("steps.base.outputs.ready != 'false'", publisher)
        self.assertIn("GH_READ_TOKEN: ${{ steps.read-token.outputs.token }}", publisher)
        self.assertIn("ref: ${{ github.workflow_sha }}", publisher)


if __name__ == "__main__":
    unittest.main()
