import argparse
import io
import json
import os
import re
import subprocess
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from test_promotion_retarget import (
    BOT,
    HEAD,
    MAIN,
    PARENT_HEAD,
    FakeWriter,
    RetargetFixture,
)

from uv_automations import cli
from uv_automations.github_promotion import PromotionReadError
from uv_automations.github_promotion_retarget import PromotionRetargetGitHub
from uv_automations.models import RepositoryIdentity
from uv_automations.promotion_models import (
    UV_DEV_REPOSITORY,
    UV_REPOSITORY,
    UV_SECURITY_REPOSITORY,
    PromotionScope,
)
from uv_automations.promotion_retarget_cli import (
    ApplyRetargets,
    IdentifyRetargets,
    add_commands,
    parse_command,
    run,
)


def repository_payload(repository: RepositoryIdentity) -> dict[str, object]:
    return {"full_name": str(repository.name), "id": repository.database_id}


def updated_payload(repository: RepositoryIdentity) -> dict[str, object]:
    return {
        "number": 124,
        "state": "open",
        "html_url": f"https://github.com/{repository.name}/pull/124",
        "draft": True,
        "user": {"login": BOT.login, "id": BOT.database_id, "type": BOT.kind.value},
        "title": "Sensitive source title",
        "body": "Sensitive source body",
        "merged_at": None,
        "merge_commit_sha": None,
        "base": {
            "repo": repository_payload(repository),
            "ref": "main",
            "sha": str(MAIN),
        },
        "head": {
            "repo": repository_payload(repository),
            "ref": "child",
            "sha": str(HEAD),
        },
        "labels": [],
    }


class PromotionRetargetGitHubTests(unittest.TestCase):
    def test_base_update_has_one_fixed_json_field(self) -> None:
        scope = PromotionScope(UV_DEV_REPOSITORY, 124)
        with (
            patch.dict(os.environ, {"GH_TOKEN": "write-token"}),
            patch("uv_automations.github_promotion.subprocess.run") as command,
        ):
            command.return_value = subprocess.CompletedProcess(
                [], 0, json.dumps(updated_payload(UV_DEV_REPOSITORY)), ""
            )
            updated = PromotionRetargetGitHub(
                token_variable="GH_TOKEN"
            ).retarget_to_main(scope)
        self.assertEqual(updated.scope, scope)
        self.assertEqual(updated.details.base.ref, "main")
        self.assertEqual(updated.details.head.sha, HEAD)
        requests = [call for call in command.call_args_list if call.args[0][0] == "gh"]
        self.assertEqual(len(requests), 1)
        request = requests[0]
        self.assertEqual(
            request.args[0],
            [
                "gh",
                "api",
                "--method",
                "PATCH",
                "repos/astral-sh/uv-dev/pulls/124",
                "--input",
                "-",
            ],
        )
        self.assertEqual(json.loads(request.kwargs["input"]), {"base": "main"})
        self.assertEqual(request.kwargs["env"]["GH_TOKEN"], "write-token")
        self.assertTrue(request.kwargs["capture_output"])

    def test_target_is_restricted_to_managed_source_repositories(self) -> None:
        with (
            patch.object(PromotionRetargetGitHub, "_api") as api,
            self.assertRaisesRegex(ValueError, "source repository"),
        ):
            PromotionRetargetGitHub().retarget_to_main(
                PromotionScope(UV_REPOSITORY, 124)
            )
        api.assert_not_called()

    def test_missing_write_token_fails_closed(self) -> None:
        with (
            patch.dict(os.environ, {"GH_TOKEN": ""}),
            patch("uv_automations.github_promotion.subprocess.run") as command,
            self.assertRaisesRegex(ValueError, "Missing GitHub token environment"),
        ):
            PromotionRetargetGitHub(token_variable="GH_TOKEN").retarget_to_main(
                PromotionScope(UV_SECURITY_REPOSITORY, 124)
            )
        command.assert_not_called()

    def test_private_subprocess_and_decoder_errors_are_redacted(self) -> None:
        scope = PromotionScope(UV_SECURITY_REPOSITORY, 124)
        output = io.StringIO()
        with (
            redirect_stdout(output),
            redirect_stderr(output),
            patch("uv_automations.github_promotion.subprocess.run") as command,
            self.assertRaisesRegex(
                PromotionReadError, "^GitHub promotion request failed$"
            ) as raised,
        ):
            command.return_value = subprocess.CompletedProcess(
                [], 1, "Sensitive source body", f"private-ref-{PARENT_HEAD}"
            )
            PromotionRetargetGitHub().retarget_to_main(scope)
        self.assertEqual(str(raised.exception), "GitHub promotion request failed")
        self.assertEqual(output.getvalue(), "")

        with (
            patch.object(
                PromotionRetargetGitHub,
                "_api",
                return_value={"private": "Sensitive source body"},
            ),
            self.assertRaisesRegex(
                PromotionReadError, "^Invalid GitHub retarget response$"
            ),
        ):
            PromotionRetargetGitHub().retarget_to_main(scope)


class PromotionRetargetCliTests(unittest.TestCase):
    def test_top_level_cli_parses_and_runs_both_stages(self) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        writer = FakeWriter(fixture.source)
        source_arguments = [
            "--repo",
            str(UV_SECURITY_REPOSITORY.name),
            "--repository-id",
            str(UV_SECURITY_REPOSITORY.database_id),
            "--main-sha",
            str(MAIN),
        ]
        with TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            summary = Path(directory) / "summary"
            identify = [
                "promotions",
                "retarget",
                "identify",
                *source_arguments,
                "--github-output",
                str(output),
                "--summary",
                str(summary),
            ]
            apply = [
                "promotions",
                "retarget",
                "apply",
                *source_arguments,
                "--summary",
                str(summary),
            ]
            self.assertEqual(
                cli.parse_command(cli.create_parser(), identify),
                IdentifyRetargets(
                    repository=UV_SECURITY_REPOSITORY,
                    main_sha=MAIN,
                    github_output=output,
                    summary=summary,
                ),
            )
            self.assertEqual(
                cli.parse_command(cli.create_parser(), apply),
                ApplyRetargets(
                    repository=UV_SECURITY_REPOSITORY, main_sha=MAIN, summary=summary
                ),
            )
            with (
                patch(
                    "uv_automations.promotion_retarget_cli.PromotionGitHub",
                    side_effect=[
                        fixture.source,
                        fixture.upstream,
                        fixture.source,
                        fixture.upstream,
                    ],
                ),
                patch(
                    "uv_automations.promotion_retarget_cli.PromotionRetargetGitHub",
                    return_value=writer,
                ) as writer_factory,
            ):
                cli.main(identify)
                writer_factory.assert_not_called()
                cli.main(apply)
            self.assertEqual(output.read_text(), "count=1\n")
            self.assertEqual(
                summary.read_text(),
                "Found 1 draft pull requests to retarget in `astral-sh/uv-security`. "
                "Left 0 ready children with promotion and skipped 0 unverifiable children.\n"
                "Retargeted 1 draft pull requests in `astral-sh/uv-security`. "
                "0 were already on main. Left 0 ready children with promotion and skipped "
                "0 unverifiable or changed children.\n",
            )
        writer_factory.assert_called_once_with(token_variable="GH_TOKEN")
        self.assertEqual(writer.writes, [fixture.child.scope])

    def test_top_level_cli_redacts_private_failures(self) -> None:
        output = io.StringIO()
        with (
            redirect_stderr(output),
            patch(
                "uv_automations.promotion_retarget_cli.plan_retargets",
                side_effect=RuntimeError(f"private-ref-{PARENT_HEAD}"),
            ),
            self.assertRaises(SystemExit) as raised,
        ):
            cli.main(
                [
                    "promotions",
                    "retarget",
                    "apply",
                    "--repo",
                    str(UV_SECURITY_REPOSITORY.name),
                    "--repository-id",
                    str(UV_SECURITY_REPOSITORY.database_id),
                    "--main-sha",
                    str(MAIN),
                ]
            )
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(
            output.getvalue(), "uv-automations: Private promotion retargeting failed\n"
        )

    def test_parser_uses_exact_source_identity_and_public_sha(self) -> None:
        parser = argparse.ArgumentParser()
        add_commands(parser)
        command = parse_command(
            parser.parse_args(
                [
                    "identify",
                    "--repo",
                    str(UV_DEV_REPOSITORY.name),
                    "--repository-id",
                    str(UV_DEV_REPOSITORY.database_id),
                    "--main-sha",
                    str(MAIN),
                ]
            )
        )
        self.assertEqual(
            command,
            IdentifyRetargets(
                repository=UV_DEV_REPOSITORY,
                main_sha=MAIN,
                github_output=None,
                summary=None,
            ),
        )
        with self.assertRaises(ValueError):
            parse_command(
                parser.parse_args(
                    [
                        "apply",
                        "--repo",
                        str(UV_DEV_REPOSITORY.name),
                        "--repository-id",
                        "1",
                        "--main-sha",
                        str(MAIN),
                    ]
                )
            )

    def test_identification_emits_only_counts_and_never_constructs_a_writer(
        self,
    ) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        with TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            summary = Path(directory) / "summary"
            with (
                patch(
                    "uv_automations.promotion_retarget_cli.PromotionGitHub",
                    side_effect=[fixture.source, fixture.upstream],
                ) as reader,
                patch(
                    "uv_automations.promotion_retarget_cli.PromotionRetargetGitHub"
                ) as writer,
            ):
                run(
                    IdentifyRetargets(
                        repository=UV_SECURITY_REPOSITORY,
                        main_sha=MAIN,
                        github_output=output,
                        summary=summary,
                    )
                )
            self.assertEqual(output.read_text(), "count=1\n")
            self.assertEqual(
                summary.read_text(),
                "Found 1 draft pull requests to retarget in `astral-sh/uv-security`. "
                "Left 0 ready children with promotion and skipped 0 unverifiable children.\n",
            )
        self.assertEqual(
            [call.kwargs for call in reader.call_args_list],
            [
                {"token_variable": "GH_READ_TOKEN"},
                {"token_variable": "GH_UPSTREAM_TOKEN"},
            ],
        )
        writer.assert_not_called()

    def test_publisher_recomputes_its_plan_with_separate_credentials(self) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        writer = FakeWriter(fixture.source)
        output = io.StringIO()
        with (
            redirect_stdout(output),
            patch(
                "uv_automations.promotion_retarget_cli.PromotionGitHub",
                side_effect=[fixture.source, fixture.upstream],
            ),
            patch(
                "uv_automations.promotion_retarget_cli.PromotionRetargetGitHub",
                return_value=writer,
            ) as writer_factory,
        ):
            run(
                ApplyRetargets(
                    repository=UV_SECURITY_REPOSITORY, main_sha=MAIN, summary=None
                )
            )
        writer_factory.assert_called_once_with(token_variable="GH_TOKEN")
        self.assertEqual(writer.writes, [fixture.child.scope])
        self.assertEqual(
            output.getvalue(),
            "Retargeted 1 draft pull requests in `astral-sh/uv-security`. "
            "0 were already on main. Left 0 ready children with promotion and skipped "
            "0 unverifiable or changed children.\n",
        )

    def test_private_errors_never_include_snapshot_or_subprocess_details(self) -> None:
        command = ApplyRetargets(
            repository=UV_SECURITY_REPOSITORY, main_sha=MAIN, summary=None
        )
        for error in (
            ValueError(f"Sensitive source title private-ref-{PARENT_HEAD}"),
            RuntimeError("Sensitive source body #124"),
            PromotionReadError(f"private-ref-{PARENT_HEAD}"),
        ):
            with (
                self.subTest(kind=type(error).__name__),
                patch(
                    "uv_automations.promotion_retarget_cli.plan_retargets",
                    side_effect=error,
                ),
                self.assertRaisesRegex(
                    ValueError, "^Private promotion retargeting failed$"
                ) as raised,
            ):
                run(command)
            self.assertTrue(raised.exception.__suppress_context__)

    def test_stale_sync_is_count_only_and_does_not_construct_a_writer(self) -> None:
        fixture = RetargetFixture(UV_SECURITY_REPOSITORY)
        fixture.source.refs[UV_SECURITY_REPOSITORY, "main"] = HEAD
        output = io.StringIO()
        with (
            redirect_stdout(output),
            patch(
                "uv_automations.promotion_retarget_cli.PromotionGitHub",
                side_effect=[fixture.source, fixture.upstream],
            ),
            patch(
                "uv_automations.promotion_retarget_cli.PromotionRetargetGitHub"
            ) as writer,
        ):
            run(
                ApplyRetargets(
                    repository=UV_SECURITY_REPOSITORY, main_sha=MAIN, summary=None
                )
            )
        writer.assert_not_called()
        self.assertEqual(
            output.getvalue(),
            "The synchronized main of `astral-sh/uv-security` changed; "
            "leaving its stacks unchanged.\n",
        )


class PromotionRetargetWorkflowTests(unittest.TestCase):
    def test_sync_callers_pass_only_the_verified_main_revision(self) -> None:
        root = Path(__file__).resolve().parents[3]
        for repository in (UV_DEV_REPOSITORY, UV_SECURITY_REPOSITORY):
            name = str(repository.name).removeprefix("astral-sh/")
            workflow = (root / f".github/workflows/sync-{name}.yml").read_text()
            with self.subTest(repository=repository.name):
                self.assertIn(
                    "    outputs:\n"
                    "      main-sha: ${{ steps.sync.outputs.main-sha }}\n",
                    workflow,
                )
                self.assertIn(
                    "  retarget-promoted-pull-request-children:\n"
                    "    name: Retarget children of promoted pull requests\n"
                    "    needs: sync\n"
                    "    if: github.repository == 'astral-sh/uv' && "
                    "github.ref == 'refs/heads/main'\n"
                    "    permissions:\n"
                    "      contents: read\n"
                    "      id-token: write\n"
                    "      pull-requests: read\n"
                    "    uses: $/.github/workflows/retarget-promoted-pull-request-children.yml\n"
                    "    with:\n"
                    f"      repository: {repository.name}\n"
                    f'      repository-id: "{repository.database_id}"\n'
                    "      main-sha: ${{ needs.sync.outputs.main-sha }}\n"
                    "    secrets: inherit\n",
                    workflow,
                )

    def test_private_sync_exports_only_publicly_checked_revisions(self) -> None:
        root = Path(__file__).resolve().parents[3]
        workflow = (root / ".github/workflows/sync-uv-security.yml").read_text()
        self.assertEqual(
            re.findall(r'echo "main-sha=\$([A-Za-z_]+)"', workflow),
            ["destination_sha", "SOURCE_SHA"],
        )
        self.assertLess(
            workflow.index('"$destination_sha" refs/heads/main 2>/dev/null'),
            workflow.index('echo "main-sha=$destination_sha"'),
        )
        self.assertLess(
            workflow.index('"$SOURCE_SHA:refs/heads/main"'),
            workflow.index('echo "main-sha=$SOURCE_SHA"'),
        )

    def test_sts_allows_only_matching_main_sync_callers_and_pr_writes(self) -> None:
        root = Path(__file__).resolve().parents[3]
        policy = json.loads((root / ".github/ost-simple-sts.json").read_text())
        rules = [
            rule
            for rule in policy["rules"]
            if rule.get("reusable_workflow")
            == "retarget-promoted-pull-request-children.yml"
        ]
        self.assertEqual(
            rules,
            [
                {
                    "caller": "uv",
                    "environment": "automations",
                    "caller_ref": "refs/heads/main",
                    "caller_workflow": f"sync-{repository}.yml",
                    "reusable_workflow": "retarget-promoted-pull-request-children.yml",
                    "on": ["push", "workflow_dispatch"],
                    "permissions": {"contents": "read", "pull_requests": "write"},
                    "target": repository,
                }
                for repository in ("uv-dev", "uv-security")
            ],
        )

    def test_reusable_workflow_has_count_only_outputs_and_separate_tokens(self) -> None:
        root = Path(__file__).resolve().parents[3]
        workflow = (
            root / ".github/workflows/retarget-promoted-pull-request-children.yml"
        ).read_text()
        self.assertEqual(
            re.findall(r"^    outputs:\n((?:      .*\n)+)", workflow, re.MULTILINE),
            ["      count: ${{ steps.identify.outputs.count }}\n"],
        )
        self.assertEqual(
            re.findall(
                r"^          permissions: \|\n((?:            .*\n)+)",
                workflow,
                re.MULTILINE,
            ),
            [
                "            contents: read\n            pull_requests: read\n",
                "            contents: read\n            pull_requests: read\n",
                "            pull_requests: write\n",
            ],
        )
        self.assertEqual(workflow.count("ref: ${{ github.workflow_sha }}"), 2)
        self.assertNotIn("gh api", workflow)
        self.assertNotIn("pull_request_target", workflow)
