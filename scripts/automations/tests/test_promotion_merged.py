import copy
import json
import shlex
import shutil
import subprocess
import unittest
from dataclasses import replace
from typing import override

import test_promotion as planning
from test_promotion_completion import (
    COMPLETION,
    HEAD,
    OTHER,
    SOURCE,
    UPSTREAM,
    FaultInjection,
    pull_request,
    repository,
)
from test_promotion_workflow_boundaries import job

from uv_automations.json import as_object
from uv_automations.models import PullRequestState
from uv_automations.promotion_models import (
    UV_SECURITY_REPOSITORY,
    PromotionApprovalKind,
    PromotionScope,
)
from uv_automations.workflows.promotion import (
    AlreadyPublished,
    PromotionRequest,
    Rejected,
    plan_promotion,
)
from uv_automations.workflows.promotion_completion import (
    PublicationAction,
    SkippedCompletion,
)

MERGE = "d" * 40


def merge(upstream: dict[str, object]) -> None:
    upstream.update(
        state="closed",
        merged_at="2026-09-09T12:00:30Z",
        merge_commit_sha=MERGE,
    )


class MergeDuringCompletion(FaultInjection):
    def __init__(self, merge_after: str) -> None:
        super().__init__()
        self.merge_after = merge_after

    @override
    def run(
        self, arguments: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        response = super().run(arguments, **kwargs)
        if arguments[:3] == ["gh", "api", "--method"]:
            operation = f"{arguments[3]} {arguments[4].split('?', 1)[0]}"
            if operation == self.merge_after:
                merge(self.upstream)
        return response


class MergedPromotionTests(unittest.TestCase):
    def test_planner_recognizes_only_exact_open_or_merged_publications(self) -> None:
        reader = planning.planner_reader()
        original = planning.pull_request(planning.UPSTREAM, author=planning.BOT)
        merged = replace(
            original,
            details=replace(original.details, state=PullRequestState.CLOSED),
            merge=planning.MERGED,
        )
        request = PromotionRequest(planning.SOURCE, planning.HEAD)
        for upstream in (original, merged):
            with self.subTest(upstream=upstream):
                reader.pull_requests[planning.UPSTREAM] = upstream
                result = plan_promotion(reader, request)
                self.assertIsInstance(result, AlreadyPublished)
                if isinstance(result, AlreadyPublished):
                    self.assertEqual(result.upstream, upstream)
        for upstream in (
            replace(merged, merge=None),
            replace(
                merged,
                details=replace(
                    merged.details,
                    base=replace(merged.details.base, ref="other"),
                ),
            ),
            replace(
                merged,
                details=replace(
                    merged.details,
                    head=replace(merged.details.head, sha=planning.UPDATED),
                ),
            ),
        ):
            with self.subTest(upstream=upstream):
                reader.pull_requests[planning.UPSTREAM] = upstream
                self.assertIsInstance(plan_promotion(reader, request), Rejected)

    def test_completion_finishes_an_exact_merged_publication(self) -> None:
        fixture = FaultInjection()
        merge(fixture.upstream)
        self.assertIsNone(fixture.execute())
        self.assertIsNone(fixture.execute(close=True))
        self.assertEqual(fixture.source["state"], "closed")
        for method, resource in (
            ("POST", "labels"),
            ("POST", "assignees"),
            ("POST", "comments"),
            ("PATCH", "close"),
        ):
            self.assertEqual(fixture.calls[fixture.operation(method, resource)], 1)

    def test_merge_between_completion_writes_does_not_interrupt_recovery(self) -> None:
        for resource in ("labels", "comments"):
            with self.subTest(resource=resource):
                fixture = MergeDuringCompletion(
                    FaultInjection().operation("POST", resource)
                )
                self.assertIsNone(fixture.execute())
                self.assertIsNone(fixture.execute(close=True))
                self.assertEqual(fixture.source["state"], "closed")
                self.assertEqual(
                    fixture.calls[fixture.operation("POST", "comments")], 1
                )
                self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)

    def test_merged_parent_update_keeps_its_distinct_published_head(self) -> None:
        completion = replace(
            COMPLETION, action=PublicationAction.UPDATE_PARENT, promoted_head=OTHER
        )
        fixture = FaultInjection(completion)
        merge(fixture.upstream)
        fixture.upstream["head"] = {
            **as_object(fixture.upstream["head"]),
            "sha": str(OTHER),
        }
        self.assertIsNone(fixture.execute())
        self.assertIsNone(fixture.execute(close=True))
        self.assertEqual(fixture.calls[fixture.operation("PATCH", "close")], 1)

    def test_merged_private_publication_requires_current_writer_approval(self) -> None:
        completion = replace(
            COMPLETION,
            source=PromotionScope(UV_SECURITY_REPOSITORY, SOURCE.number),
            source_base=replace(
                COMPLETION.source_base, repository=UV_SECURITY_REPOSITORY
            ),
            source_head=replace(
                COMPLETION.source_head, repository=UV_SECURITY_REPOSITORY
            ),
            approval_kind=PromotionApprovalKind.LABELED,
        )
        for permission in ("write", "read"):
            with self.subTest(permission=permission):
                fixture = FaultInjection(completion)
                merge(fixture.upstream)
                fixture.events[0].update(event="labeled", label={"name": "bot:promote"})
                fixture.permission = permission
                if permission == "write":
                    self.assertIsNone(fixture.execute())
                    self.assertIsNone(fixture.execute(close=True))
                    self.assertEqual(
                        fixture.calls[fixture.operation("PATCH", "close")], 1
                    )
                else:
                    self.assertIsInstance(fixture.execute(), SkippedCompletion)
                    self.assertIsInstance(
                        fixture.execute(close=True), SkippedCompletion
                    )
                    self.assertFalse(
                        any(
                            key.startswith(("POST ", "PATCH ")) for key in fixture.calls
                        )
                    )

    def test_merged_publication_does_not_relax_completion_preconditions(self) -> None:
        for target, revision, field, value in (
            ("source", "base", "ref", "other"),
            ("source", "head", "sha", str(OTHER)),
            ("upstream", "base", "ref", "other"),
            ("upstream", "head", "sha", str(OTHER)),
            ("upstream", "head", "repo", repository(SOURCE)),
        ):
            for close in (False, True):
                with self.subTest(target=target, field=field, close=close):
                    fixture = FaultInjection()
                    merge(fixture.upstream)
                    data = getattr(fixture, target)
                    data[revision] = {**as_object(data[revision]), field: value}
                    self.assertIsInstance(
                        fixture.execute(close=close), SkippedCompletion
                    )
                    self.assertFalse(
                        any(
                            key.startswith(("POST ", "PATCH ")) for key in fixture.calls
                        )
                    )
        for close in (False, True):
            fixture = FaultInjection()
            fixture.upstream["state"] = "closed"
            self.assertIsInstance(fixture.execute(close=close), SkippedCompletion)
            self.assertFalse(
                any(key.startswith(("POST ", "PATCH ")) for key in fixture.calls)
            )

    def test_workflow_accepts_only_exact_open_or_merged_publications(self) -> None:
        jq = shutil.which("jq")
        if jq is None:
            self.skipTest("jq is required to check the workflow publication guard")
        section = job("promote-pull-request.yml", "promote", "replay-promoted-children")
        section = section.split("if (( existing_count == 1 )); then\n", 1)[1]
        section = section.split("upstream_pull_request=$(jq --compact-output", 1)[0]
        lines = [
            line.strip()
            for line in section.splitlines()
            if line.strip().startswith("'")
        ]
        self.assertEqual(len(lines), 1)
        (predicate,) = shlex.split(lines[0].removesuffix(" \\").strip())
        upstream = pull_request(UPSTREAM)
        merged = copy.deepcopy(upstream)
        merge(merged)
        closed = {**upstream, "state": "closed"}
        changed_head = copy.deepcopy(merged)
        changed_head["head"] = {**as_object(changed_head["head"]), "sha": str(OTHER)}
        changed_base = copy.deepcopy(merged)
        changed_base["base"] = {**as_object(changed_base["base"]), "ref": "other"}
        for value, accepted in (
            (upstream, True),
            (merged, True),
            (closed, False),
            ({**merged, "merged_at": None}, False),
            ({**merged, "merge_commit_sha": None}, False),
            ({**merged, "merge_commit_sha": "invalid"}, False),
            (changed_head, False),
            (changed_base, False),
        ):
            with self.subTest(value=value):
                result = subprocess.run(
                    (
                        jq,
                        "--exit-status",
                        "--arg",
                        "base_ref",
                        "main",
                        "--arg",
                        "head_ref",
                        "fixture/promotion",
                        "--arg",
                        "head_sha",
                        str(HEAD),
                        "--argjson",
                        "repository_id",
                        str(UPSTREAM.repository.database_id),
                        predicate,
                    ),
                    input=json.dumps([value]),
                    text=True,
                    capture_output=True,
                    check=False,
                    timeout=30,
                )
                self.assertEqual(result.returncode == 0, accepted, result.stderr)


if __name__ == "__main__":
    unittest.main()
