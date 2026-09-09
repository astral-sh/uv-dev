import json
import subprocess
import unittest
from unittest.mock import patch

from uv_automations.github_actions import (
    DISPATCH_API_VERSION,
    ActionsGitHub,
    WorkflowDispatch,
)
from uv_automations.models import RepositoryIdentity, RepositoryName

REPOSITORY = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
REPOSITORY_RESPONSE = {
    "full_name": str(REPOSITORY.name),
    "id": REPOSITORY.database_id,
}


def response(identifier: int = 123) -> dict[str, object]:
    return {
        "workflow_run_id": identifier,
        "run_url": f"https://api.github.com/repos/{REPOSITORY.name}/actions/runs/{identifier}",
        "html_url": f"https://github.com/{REPOSITORY.name}/actions/runs/{identifier}",
    }


class WorkflowDispatchTests(unittest.TestCase):
    def test_main_dispatch_retains_run_identity(self) -> None:
        inputs = {"pull_request": "7", "head_sha": "a" * 40, "approval_id": "99"}
        with patch(
            "uv_automations.github.subprocess.run",
            side_effect=(
                subprocess.CompletedProcess([], 0, json.dumps(REPOSITORY_RESPONSE)),
                subprocess.CompletedProcess([], 0, json.dumps(response())),
            ),
        ) as command:
            result = ActionsGitHub().dispatch_main_workflow(
                REPOSITORY, "promote-pull-request.yml", inputs
            )
        self.assertEqual(result, WorkflowDispatch(REPOSITORY, 123))
        self.assertEqual(
            result.url, "https://github.com/astral-sh/uv-dev/actions/runs/123"
        )
        self.assertEqual(
            command.call_args_list[0].args[0],
            ["gh", "api", "--method", "GET", "repos/astral-sh/uv-dev"],
        )
        self.assertEqual(
            command.call_args.args[0],
            [
                "gh",
                "api",
                "--method",
                "POST",
                "--header",
                f"X-GitHub-Api-Version: {DISPATCH_API_VERSION}",
                "repos/astral-sh/uv-dev/actions/workflows/promote-pull-request.yml/dispatches",
                "--input",
                "-",
            ],
        )
        self.assertEqual(
            json.loads(command.call_args.kwargs["input"]),
            {"ref": "main", "inputs": inputs},
        )

    def test_dispatch_response_is_not_a_redirect(self) -> None:
        for changed in (
            {"workflow_run_id": True},
            {"run_url": "https://api.github.com/repos/other/repo/actions/runs/123"},
            {"html_url": "https://github.com/other/repo/actions/runs/123"},
        ):
            with (
                self.subTest(changed=changed),
                patch(
                    "uv_automations.github.subprocess.run",
                    side_effect=(
                        subprocess.CompletedProcess(
                            [], 0, json.dumps(REPOSITORY_RESPONSE)
                        ),
                        subprocess.CompletedProcess(
                            [], 0, json.dumps(response() | changed)
                        ),
                    ),
                ),
                self.assertRaises((TypeError, ValueError)),
            ):
                ActionsGitHub().dispatch_main_workflow(REPOSITORY, "promote.yml", {})

    def test_repository_name_cannot_resolve_to_a_different_id(self) -> None:
        with (
            patch(
                "uv_automations.github.subprocess.run",
                return_value=subprocess.CompletedProcess(
                    [], 0, json.dumps(REPOSITORY_RESPONSE | {"id": 1})
                ),
            ) as command,
            self.assertRaisesRegex(ValueError, "repository identity changed"),
        ):
            ActionsGitHub().dispatch_main_workflow(REPOSITORY, "promote.yml", {})
        self.assertEqual(command.call_count, 1)

    def test_only_named_workflows_are_accepted(self) -> None:
        for workflow in (
            "../promote.yml",
            "promote.yml/dispatches",
            "main",
            "x\ny.yml",
        ):
            with self.subTest(workflow=workflow), self.assertRaises(ValueError):
                ActionsGitHub().dispatch_main_workflow(REPOSITORY, workflow, {})


if __name__ == "__main__":
    unittest.main()
