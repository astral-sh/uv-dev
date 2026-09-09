# uv automations

This internal Python 3.14+ package moves workflow logic out of shell and `jq` without replacing
GitHub Actions as the scheduler. Actions still owns triggers, permissions, concurrency, runners, job
dependencies, and credential acquisition.

The first consumers are pull-request label validation and conflicted-pull-request discovery:

```console
uv run --project scripts/automations --locked --no-dev uv-automations labels validate --allowed .github/allowed-pull-request-labels.json
uv run --project scripts/automations --locked --no-dev uv-automations pull-requests conflicts --repo astral-sh/uv
```

Label validation reads the recommendation from stdin. Pass `--github-output "$GITHUB_OUTPUT"` to
write the validated `labels` output. Conflict discovery preserves the JSON consumed by
`.github/workflows/pull-request-conflicts.yml`, including the head repository and the full head SHA.

## Library shape

- `models` contains validated identities, immutable dataclasses, and closed enums.
- `github` provides typed reads through `gh`. Credentials come from the caller's environment; the
  library does not acquire or persist tokens.
- `actions` adapts typed results to Actions' file-based interfaces.
- `workflows` contains workflow-specific decisions. Functions accept explicit inputs and narrow
  interfaces, so tests do not need GitHub access.
- `cli` only parses arguments, loads inputs, invokes the library, and renders results.

Use exhaustive `match` statements followed by `assert_never` for finite states. When outcomes have
different payloads, use unions of frozen dataclasses instead of several optional fields and flags.
Decode external JSON at the boundary rather than passing untyped dictionaries through the workflow.

## Subsequent migrations

Keep the existing wire formats while moving one consumer at a time. The intended API for the next
consumers is approximately:

```python
# Read and validate.
github.get_issue(reference: IssueRef) -> Issue
github.get_workflow_run(reference: WorkflowRunRef) -> WorkflowRun
context.load(issue: IssueRef) -> IssueContext
artifacts.verify_bundle(path: Path, policy: BundlePolicy) -> VerifiedBundle

# Plan without writing.
failures.plan_retry(run, diagnosis, policy) -> RetryPlan
promotion.plan(source, approval, upstream, parent) -> PromotionPlan
rebase.plan(source, destination, previous_base) -> RebasePlan

# Apply explicitly, checking that the expected revisions still match.
context.persist(git, update, *, expected_head: CommitSha) -> PersistResult
failures.retry(github_writer, plan: RetryPlan) -> RetryResult
promotion.publish(github_writer, git, plan, bundle) -> PromotionResult
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
