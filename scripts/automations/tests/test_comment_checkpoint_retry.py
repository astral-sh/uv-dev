import subprocess
import unittest
from dataclasses import dataclass, field, replace
from typing import override
from unittest.mock import patch

from test_comments import (
    CURRENT_RUN,
    HEAD,
    LATER,
    PRIOR_RUN,
    REPOSITORY,
    SCOPE,
    WORKFLOW_SHA,
    workflow_run,
)
from test_comments_index import (
    IndexGitHub,
    artifact_base,
    discover,
    request_for,
    retain_index,
    retain_marker,
    state_for,
)

from uv_automations.github_actions import (
    ActionsRun,
    ArtifactIdentity,
    ManifestArtifact,
    WorkflowRun,
)
from uv_automations.workflows.comments_index import (
    CHECKPOINT_DISCOVERY_ARTIFACTS,
    CheckpointSelection,
    SupersededFeedback,
    continue_feedback,
    discovery_name,
    next_continuation,
    prepare_index_aliases,
)


def read_failure() -> subprocess.CalledProcessError:
    return subprocess.CalledProcessError(
        1, ["gh", "api", "--method", "GET", "actions/artifacts"], stderr="HTTP 503"
    )


@dataclass
class FailingIndexGitHub(IndexGitHub):
    failures: dict[tuple[str, int], list[Exception]] = field(default_factory=dict)
    reads: list[tuple[str, int]] = field(default_factory=list)

    def before_read(self, kind: str, identifier: int) -> None:
        key = kind, identifier
        self.reads.append(key)
        if failures := self.failures.get(key):
            raise failures.pop(0)

    @override
    def read_json_manifest(self, artifact: ManifestArtifact) -> object:
        self.before_read("manifest", artifact.identifier)
        return super().read_json_manifest(artifact)

    @override
    def get_workflow_run(self, source: ActionsRun) -> WorkflowRun:
        self.before_read("workflow", source.identifier)
        return super().get_workflow_run(source)

    @override
    def get_artifact(
        self, source: ActionsRun, identifier: int, name: str
    ) -> ArtifactIdentity:
        self.before_read("artifact", identifier)
        return super().get_artifact(source, identifier, name)


def github_for(current: ActionsRun = CURRENT_RUN) -> FailingIndexGitHub:
    return FailingIndexGitHub(
        workflow_runs={current: workflow_run(current, successful=False)}
    )


class CommentCheckpointRetryTests(unittest.TestCase):
    def test_transient_alias_inspection_keeps_the_newest_checkpoint(self) -> None:
        for step in ("alias", "index", "workflow", "state", "session"):
            with self.subTest(step=step):
                github = github_for()
                retain_index(github, state_for())
                source = ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA)
                newest = retain_index(
                    github, state_for(source, through=LATER), created_at=LATER
                )
                alias = artifact_base(source) + 6
                match step:
                    case "alias":
                        key = "manifest", alias
                    case "index":
                        key = "manifest", newest.artifact.identifier
                    case "workflow":
                        key = "workflow", source.identifier
                    case "state":
                        key = "artifact", newest.value.state.identifier
                    case "session":
                        key = "artifact", newest.value.session.identifier
                    case _:
                        raise AssertionError("Unexpected inspection step")
                github.failures[key] = [read_failure()]
                with patch(
                    "uv_automations.workflows.comments_index.time", create=True
                ) as clock:
                    selected = discover(github)
                self.assertEqual(
                    selected, CheckpointSelection(newest.value.state, newest.artifact)
                )
                self.assertEqual(github.reads.count(("manifest", alias)), 2)
                self.assertEqual(github.failures[key], [])
                self.assertEqual(
                    github.manifest_queries,
                    [
                        (
                            REPOSITORY,
                            discovery_name(SCOPE),
                            CHECKPOINT_DISCOVERY_ARTIFACTS,
                        )
                    ],
                )
                self.assertEqual(github.run_queries, [])
                self.assertEqual(github.dispatches, [])
                self.assertEqual(
                    [call.args for call in clock.sleep.call_args_list], [(5,)]
                )

    def test_exhausted_alias_reads_keep_the_older_checkpoint_fallback(self) -> None:
        github = github_for()
        older = retain_index(github, state_for())
        source = ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA)
        newest = retain_index(
            github, state_for(source, through=LATER), created_at=LATER
        )
        alias = artifact_base(source) + 6
        github.failures["manifest", newest.artifact.identifier] = [
            read_failure() for _ in range(3)
        ]
        with patch(
            "uv_automations.workflows.comments_index.time", create=True
        ) as clock:
            selected = discover(github)
        self.assertEqual(
            selected, CheckpointSelection(older.value.state, older.artifact)
        )
        self.assertEqual(github.reads.count(("manifest", alias)), 3)
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )

    def test_invalid_or_unfinished_alias_reads_are_not_retried(self) -> None:
        for failure in (
            ValueError("The immutable manifest digest changed"),
            subprocess.TimeoutExpired(["gh", "api"], 60),
            OSError("Could not start gh"),
        ):
            with self.subTest(failure=type(failure).__name__):
                github = github_for()
                older = retain_index(github, state_for())
                source = ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA)
                newest = retain_index(
                    github, state_for(source, through=LATER), created_at=LATER
                )
                github.failures["manifest", newest.artifact.identifier] = [failure]
                with patch(
                    "uv_automations.workflows.comments_index.time", create=True
                ) as clock:
                    selected = discover(github)
                self.assertEqual(
                    selected, CheckpointSelection(older.value.state, older.artifact)
                )
                self.assertEqual(
                    github.reads.count(("manifest", artifact_base(source) + 6)), 1
                )
                clock.sleep.assert_not_called()

    def test_consumed_continuation_alias_recovers_without_dispatching(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        incoming = next_continuation(parent)
        if incoming is None:
            raise AssertionError("Expected a pending continuation")
        sibling = retain_index(
            github,
            state_for(
                ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA),
                through=LATER,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            created_at=LATER,
        )
        marker = retain_marker(github, sibling)
        github.failures["manifest", marker.identifier] = [read_failure()]
        with patch(
            "uv_automations.workflows.comments_index.time", create=True
        ) as clock:
            selected = discover(
                github,
                explicit=request_for(incoming),
                budget=incoming.remaining,
                key=incoming.key,
            )
        self.assertEqual(selected, SupersededFeedback(sibling.value.state))
        self.assertEqual(github.reads.count(("manifest", marker.identifier)), 2)
        self.assertEqual(github.dispatches, [])
        self.assertEqual([call.args for call in clock.sleep.call_args_list], [(5,)])

    def test_exhausted_consumption_reads_still_fail_closed(self) -> None:
        github = github_for()
        parent = retain_index(github, state_for())
        incoming = next_continuation(parent)
        if incoming is None:
            raise AssertionError("Expected a pending continuation")
        sibling = retain_index(
            github,
            state_for(
                ActionsRun(REPOSITORY, 150, 1, WORKFLOW_SHA),
                through=LATER,
                budget=incoming.remaining,
                key=incoming.key,
            ),
            created_at=LATER,
        )
        marker = retain_marker(github, sibling)
        github.failures["manifest", marker.identifier] = [
            read_failure() for _ in range(3)
        ]
        with (
            patch("uv_automations.workflows.comments_index.time", create=True) as clock,
            self.assertRaisesRegex(ValueError, "Cannot prove continuation completion"),
        ):
            discover(
                github,
                explicit=request_for(incoming),
                budget=incoming.remaining,
                key=incoming.key,
            )
        self.assertEqual(github.reads.count(("manifest", marker.identifier)), 3)
        self.assertEqual(github.dispatches, [])
        self.assertEqual(
            [call.args for call in clock.sleep.call_args_list], [(5,), (10,)]
        )

    def test_publisher_retry_keeps_its_last_successful_alias(self) -> None:
        github = github_for()
        previous = retain_index(github, state_for())
        retry = replace(PRIOR_RUN, attempt=2)
        github.workflow_runs[retry] = workflow_run(retry, successful=False)
        state = state_for(retry, through=LATER)
        current = retain_index(github, state, created_at=LATER, discoverable=False)
        alias = artifact_base(PRIOR_RUN) + 6
        github.failures["manifest", alias] = [read_failure()]
        manifests = github.manifests.copy()
        with patch(
            "uv_automations.workflows.comments_index.time", create=True
        ) as clock:
            aliases = prepare_index_aliases(
                github,
                retry,
                state,
                current.value.state.identifier,
                current.artifact.identifier,
            )
        self.assertEqual(aliases.discovery.previous, previous.artifact)
        self.assertIsNone(aliases.consumption)
        self.assertEqual(github.reads.count(("manifest", alias)), 2)
        self.assertEqual(github.manifests, manifests)
        self.assertEqual([call.args for call in clock.sleep.call_args_list], [(5,)])

    def test_continuation_dispatch_is_outside_the_alias_read_retry(self) -> None:
        github = github_for()
        state = state_for(CURRENT_RUN)
        current = retain_index(github, state)
        failure = read_failure()
        with (
            patch.object(github, "dispatch_feedback", side_effect=failure) as dispatch,
            patch("uv_automations.workflows.comments_index.time", create=True) as clock,
            self.assertRaises(subprocess.CalledProcessError) as caught,
        ):
            continue_feedback(
                github,
                github,
                CURRENT_RUN,
                state,
                current.value.state.identifier,
                current.artifact.identifier,
                expected_head=HEAD,
            )
        self.assertIs(caught.exception, failure)
        dispatch.assert_called_once()
        clock.sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
