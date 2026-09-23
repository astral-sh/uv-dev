# uv automations

This internal Python 3.14+ package moves workflow logic out of shell and `jq` without replacing
GitHub Actions as the scheduler. Actions still owns triggers, permissions, concurrency, runners, job
dependencies, and credential acquisition.

The first consumers are pull-request labeling, conflicted-pull-request discovery, and commit
transport for the regression-test and bug-fix workflows:

```console
uv run --project scripts/automations --locked --no-dev uv-automations labels validate --allowed .github/allowed-pull-request-labels.json
uv run --project scripts/automations --locked --no-dev uv-automations pull-requests conflicts --repo astral-sh/uv
```

Label validation reads the recommendation from stdin. Pass `--github-output "$GITHUB_OUTPUT"` to
write the validated `labels` output. Conflict discovery preserves the JSON consumed by
`.github/workflows/pull-request-conflicts.yml`, including the head repository and the full head SHA.
The label workflow loads its allowlist from `main`, independently of the revision providing the
automation implementation.

## Python and Actions boundary

Python owns input and result validation, GitHub API calls, Git operations, policy decisions, matrix
and request payloads, summaries, and publication. The CLI exposes named workflow stages; it does not
expose arbitrary GitHub requests. Internally, `gh` and `git` remain useful transports and are
invoked with argument arrays rather than shell commands.

Actions owns triggers, the job graph, runner selection, concurrency, environments, permissions,
credential exchange, checkout/tool setup, artifact upload/download, and the Codex action. Small
`run` steps invoke one Python stage. Moving an operation into Python must not move it into a
less-trusted job or make its credentials available to an agent.

`pull-request-labels.yml` now calls `labels prepare`, `labels report`, `labels validate`, and
`labels apply`. Preparation records the inspected head, and publication checks that the pull request
is still open at that head. `pull-request-conflicts.yml` calls `pull-requests identify` and
`pull-requests remove-rebase-label`; Python owns dispatch verification, repository-ID checks, the
writable-head filter, the matrix limit, and idempotent cleanup. The actual rebase remains a separate
reusable workflow until its artifact and Git operations are migrated.

## Library shape

- `models` contains validated identities, immutable dataclasses, and closed enums.
- `github` provides typed reads and narrow writes through `gh`. Credentials come from the caller's
  environment; the library does not acquire or persist tokens. A publisher can use
  `GitHub(token_variable="GH_READ_TOKEN")` for provenance and freshness reads while `GH_TOKEN`
  remains a separate, narrowly scoped writer credential.
- `git` provides operations on repositories with trusted Git metadata.
- `checkouts` creates an isolated view of a candidate whose Git metadata was writable by an agent.
- `artifacts` persists and imports exact commit ranges without checking out received code.
- `actions` adapts typed results to Actions' file-based interfaces.
- `workflows` contains workflow-specific decisions. Functions accept explicit inputs and narrow
  interfaces, so tests do not need GitHub access.
- `cli` only parses arguments, loads inputs, invokes the library, and renders results.

Use exhaustive `match` statements followed by `assert_never` for finite states. When outcomes have
different payloads, use unions of frozen dataclasses instead of several optional fields and flags.
Decode external JSON at the boundary rather than passing untyped dictionaries through the workflow.
The shared decoder rejects duplicate object fields and non-finite numbers.

For example, workflow code can make an explicit, typed GitHub query:

```python
from uv_automations.github import GitHub, OpenPullRequestQuery
from uv_automations.models import RepositoryName

query = OpenPullRequestQuery(
    repository=RepositoryName("astral-sh/uv"),
    base="main",
    author="app/astral-automations-bot",
)
pull_requests = GitHub().list_open_pull_requests(query)
```

## Trusted runtime and commit artifacts

Workflows that switch to candidate code first check out `github.workflow_sha` and call
`.github/actions/setup-automations`. The action installs a non-editable copy of this package in a
separate Python 3.14 environment. Invoke its `python` output with `-I -m uv_automations` so later
checkouts, `PYTHONPATH`, and local modules cannot replace the implementation. It leaves the job's
Python selection alone; only this runtime is pinned to Python 3.14. The bootstrap also installs a
pinned `uv` on `PATH`. Workflows that use `uv` as the product under test or as their project tool
install their intended version separately afterward. Agent steps receive a separate writable
temporary directory, with their caches and editable context inside it; the installed runtime stays
outside that directory. These profiles do not grant writes to all of `/tmp`, which can also contain
the runner's trusted temporary state.

The shared Git adapter disables filesystem monitors, traditional hooks, grafts, replacement objects,
and inherited repository-selection state. These settings do not make arbitrary repository-local
configuration safe. If an agent can write `.git`, inspect it through a private transport clone of a
protected object snapshot before running Git operations outside the agent's sandbox:

```python
from uv_automations.artifacts import CommitRange, persist_commit
from uv_automations.checkouts import inspect_candidate

with inspect_candidate(source, base=base_sha, scratch=trusted_scratch) as candidate:
    candidate.require_clean()
    commits = CommitRange(base_sha, candidate.head)
    bundle = persist_commit(candidate.repository, commits, destination)
```

Only the captured `HEAD` and independently copied loose objects and pack/index pairs enter the
snapshot's fresh SHA-1/files-ref metadata. Git never reads the source configuration, hooks,
alternates, grafts, or replacement refs. The source index is checked separately from a fresh
worktree-check index, so staged leftovers and index flags cannot hide changes. Attribute lookup uses
the trusted base. `require_clean` is a consistency check of the live working tree, not an atomic
worktree capture; transported commits come only from the protected object snapshot.

The first implementation requires Git 2.50.1 or newer, native no-follow directory descriptors, and a
full SHA-1 Actions checkout with ordinary files-based refs. It rejects submodules, linked worktrees,
reftable, shallow repositories, promisor packs, alternate object stores, and symlinked or hardlinked
source metadata. Copies are limited to 100,000 source files and 2 GiB, and incomplete or corrupt
object stores fail closed. The scratch directory must be outside the agent-writable checkout and
temporary directories. Git reads can select a separate credential with
`repository.with_token("GH_READ_TOKEN")`; the original repository object retains the writer's
environment.

Commit transport has a small, concrete API:

```python
from pathlib import Path

from uv_automations.artifacts import CommitRange, load_commit, persist_commit
from uv_automations.git import Git
from uv_automations.models import CommitSha

commits = CommitRange(base=CommitSha(base_sha), head=CommitSha(head_sha))
bundle = persist_commit(Git(Path("producer")), commits, Path("commit.bundle"))
verified_head = load_commit(Git(Path("publisher")), bundle.path, commits)
```

The base must be a strict ancestor of the head. The bundle advertises exactly
`refs/uv-automations/commits/<head-sha>`; import verifies that ref and the expected commit before
checking ancestry. It preserves the publisher's checkout, refs, and `FETCH_HEAD`. Empty results
belong to the caller's outcome type, not to an ambiguous empty bundle.

Use `commits persist` and `commits load` for the corresponding CLI stages. Actions transfers the
bundle using the upload action's immutable `artifact-id`; pass that ID and the producer's exact
`head-sha` to the publisher. A valid bundle is not authorization to publish it: the caller must
still check its own allowed paths, authorship, current target, and other preconditions before
acquiring write credentials.

## Pull request feedback

The feedback adapters add immutable contracts without coupling them to one workflow:

- `comment_models` decodes conversation comments, review summaries, complete review threads, and
  bounded agent recommendations. Target revisions are content fingerprints, and the Codex output
  schema is generated from the same enums and limits as the Python decoder.
- `github_comments.CommentGitHub` provides bounded comment pages, review-thread cursors, direct
  selected-thread ownership checks, and narrow reply/resolve operations. It preserves GitHub's
  64-bit comment and review identities.
- `github_actions.ActionsRun` separates immutable workflow-run identity from its retry attempt.
  `ArtifactIdentity` records the repository ID, run, attempt, artifact ID, name, and digest.
  `ActionsGitHub` checks the available REST metadata. The
  [artifact response](https://docs.github.com/en/rest/actions/artifacts#get-an-artifact) does not
  identify the retry attempt: consumers bind it through a trusted producer name or manifest and
  verify that exact workflow attempt before trusting downloaded contents. `ActionsRun.workflow_sha`
  is deliberately limited to the verified root `workflow_dispatch` on `main`; REST `head_sha` is not
  generic workflow-file provenance for other event types.
- `sessions.snapshot_sessions(path, workspace, trusted_root=runner_temp)` validates one root Codex
  Action session and its parent-linked subagents. The root must be outside all agent-writable
  ancestors; the reader walks below it through no-follow directory descriptors.
  `write_sessions(snapshot, destination)` creates a fresh sessions tree, normalizing compressed
  rollouts without merging another Codex home.

## Subsequent migrations

Migrate complete deterministic workflow stages rather than extracting isolated `jq` expressions.
Keep the existing wire formats and job-level credential boundaries during each migration.

The outstanding automation drafts provide useful tests of that boundary:

| Proposal                                                   | Shared mechanism                                                                | Policy that remains with the consumer                                                            |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| [uv-dev#766](https://github.com/astral-sh/uv-dev/pull/766) | Exact commit ranges, bundle verification, and immutable artifact IDs            | Allowed paths, authorship, parent selection, and permission to publish                           |
| [uv-dev#546](https://github.com/astral-sh/uv-dev/pull/546) | Typed issue reads and safe context-file creation                                | Which issue to collect and how a workflow uses its context                                       |
| [uv-dev#942](https://github.com/astral-sh/uv-dev/pull/942) | Candidate inspection, explicit empty/nonempty outcomes, and leased pushes       | Independently proving that the original changes are already in the base before closing a PR      |
| [uv-dev#305](https://github.com/astral-sh/uv-dev/pull/305) | Bounded feedback collection, typed results, and verified run/session provenance | Eligible feedback, checkpoint advancement, commit accounting, and reply/resolve decisions        |
| [uv-dev#894](https://github.com/astral-sh/uv-dev/pull/894) | Bounded event histories, typed promotion records, and exact workflow dispatch   | The queued head and human approval are still current, and the parent merge reached source `main` |
| [uv-dev#922](https://github.com/astral-sh/uv-dev/pull/922) | Repository identities, parent history, ancestry, and narrow base updates        | Unique promoted-parent evidence, synchronized-`main` checks, and child freshness                 |
| [uv-dev#986](https://github.com/astral-sh/uv-dev/pull/986) | Typed human approval events and publication preconditions                       | Private-to-public approval, label ordering, and recovery policy                                  |

The promotion consumers should use a `PromotionApproval` that retains the source repository
identity, pull request, approved head, human event ID, and approval kind. A `PromotionRecord` should
identify the upstream pull request only after verifying the issuing bot's database ID and GitHub App
identity. A `RetargetPlan` retains the synchronized `main` SHA, exact child base/head, and verified
parent merge; publication rechecks those preconditions before each narrow mutation. These are
publication authority, not properties of a valid Git bundle.

| Existing workflows                                                                     | Python responsibility                                                                                                |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `issue-triage`, `update-issue-context`, `reproduce-bug`, `fix-bug`                     | Issue context, thread selection, typed agent results, candidate commits, and context/PR publication                  |
| `diagnose-workflow-failure`                                                            | Run snapshots, retry budget and backoff, stale-run checks, duplicate detection, and issue reporting                  |
| `promote-pull-request`, `update-pull-request-parent`, `rebase-conflicted-pull-request` | Approval and repository identity, parent state, ancestry, bundles, leased pushes, and recovery                       |
| `pull-request-security-review`                                                         | Finding conversion, review anchors, current-head checks, and publication                                             |
| `plan`, `sync-uv-dev`, `sync-uv-security`, `sync-python-releases`                      | Changed-file decisions, revision plans, metadata updates, and synchronization                                        |
| Release preparation, signing, verification, and publication                            | Release policy, artifact inventories, checksums, and publication plans; native build/signing tools remain primitives |

The intended API for the next consumers is approximately:

```python
# github.py
def get_issue(reference: IssueRef) -> Issue: ...
def get_workflow_run(reference: WorkflowRunRef) -> WorkflowRun: ...


# context.py
def load(issue: IssueRef) -> IssueContext: ...
def persist(
    git: Git, update: IssueContextUpdate, *, expected_head: CommitSha
) -> PersistResult: ...


# workflows/failures.py
def plan_retry(
    run: WorkflowRun, diagnosis: Diagnosis, policy: RetryPolicy
) -> RetryPlan: ...
def retry(github_writer: GitHubWriter, plan: RetryPlan) -> RetryResult: ...


# workflows/promotion.py
def plan(
    source: PullRequest,
    approval: PromotionApproval,
    upstream: RepositoryState,
    parent: ParentState,
) -> PromotionPlan: ...
def publish(
    github_writer: GitHubWriter,
    git: Git,
    plan: PromotionPlan,
    head: CommitSha,
) -> PromotionResult: ...
def plan_replay(
    queued: QueuedPromotion,
    approval: PromotionApproval,
    parent: PromotedParent,
    source_main: CommitSha,
) -> ReplayPlan | StalePromotion: ...
def plan_retarget(
    parent: PromotedParent,
    children: tuple[PullRequestDetails, ...],
    source_main: CommitSha,
) -> tuple[RetargetPlan, ...]: ...
def retarget(
    reader: PromotionReader, writer: PullRequestBaseWriter, plan: RetargetPlan
) -> RetargetOutcome: ...
```

`RebaseResult`, for example, should be a union of `CleanRebase`, `ConflictedRebase`, `EmptyRebase`,
and `RejectedRebase`. A plan must carry the repository identity, approved head SHA, run attempt, or
other preconditions needed to detect stale state. Git writes use an exact lease.

Keep analysis and publication in separate jobs. A publisher loads its code and configuration from
the trusted workflow revision, validates incoming artifacts again, and obtains only the permissions
needed for its specific operation. A Python type or `Protocol` is not a credential boundary.

Release and synchronization workflows can later share Git revision handling, artifact inventories,
checksums, and publication results without sharing repository-specific release policy.

## Development

```console
uv run --directory scripts/automations --locked python -m unittest discover -s tests
uv run --only-group check ty check --project scripts/automations
uv run --only-group check ruff check scripts/automations
uv run --only-group check ruff format --check scripts/automations
```
