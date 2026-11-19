# uv lock --check reports multiple matches when a dependency has no package entry

Issue: astral-sh/uv#23295

Classification: bug

## Summary

On uv 0.13.9, `uv lock --check --offline --verbose` rejects a `uv.lock` in which `demo` depends on `demo-util` by name but no `[[package]]` entry exists for `demo-util`. Rejecting the file is correct; saying that the dependency has more than one matching package is not. The reporter encountered this after a merge and supplied a minimal schema-v1 fixture independent of the merge. The closest historical report had genuine ambiguity, a post-merge report has the inverse graph problem, and a recent `--check` diagnostic change operates after parsing.

## Draft response

Thanks for the reproducer. `uv lock --check` should still reject this lockfile, but it should say that `demo-util` has no corresponding package entry, not that there are multiple matches. A focused PR that distinguishes those cases and adds a regression test using your example would be welcome. To repair the merge now, start from an intact lockfile from the base branch and run `uv lock` with the merged project metadata.

## Classification

This is a confirmed diagnostic bug, not a request to accept or automatically repair invalid lockfiles. The parser reports multiple matches when there are zero. No existing tracking item or prior fix for that distinction was found. The fixture establishes the diagnostic failure, not how the original merge lost the package entry.

## Related

- astral-sh/uv#11701 (closed issue): Same missing-`source` message from the lockfile parser, but the attached lockfile has two `numpy` package entries and a name-only reference. A maintainer reproduced the parse failure with the existing lockfile, could not reproduce it from scratch, and closed the issue without a fix. This is true ambiguity, not a missing package entry.
- astral-sh/uv#18788 (open issue): The same check mode and a post-merge lockfile inconsistency, but the graph problem is the inverse: an unreferenced transitive package remains and the check succeeds. Here a reference remains without a package, and the check already fails.
- astral-sh/uv#23251 (merged pull request): Improves `--locked`/`--check` diagnostics when a parsed lockfile is stale and needs updating. It does not change deserialization or the missing-`source` error. It merged on November 17, 2026, after uv 0.13.9 was published that day.

## Supporting evidence

In `crates/uv-lock/src/lock/mod.rs:6888`, `Lock::try_from` builds a lookup containing only names with exactly one package entry. It removes names with multiple entries; names with no entries never enter the lookup. `PackageIdForDependency::unwire` at `crates/uv-lock/src/lock/mod.rs:8041` emits `MissingDependencySource` on a lookup miss for a dependency without `source`, without distinguishing why the name is absent. This happens before the later check for dependency IDs with no locked package at `crates/uv-lock/src/lock/mod.rs:2791`. The same lookup and error branch are present in the 0.13.9 tag.

The `lock_unsupported_version` test in `crates/uv/tests/lock/lock.rs:31242` already snapshots the misleading inner cause for an absent `iniconfig` package, but uses `--frozen` with unsupported schema v2. The reported schema-v1 `--check` case gives a focused regression scenario without the unsupported-version error.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated GitHub CLI searches. Literal searches used the parse error, `more than one matching package`, the missing `source` wording, `uv lock --check`, `demo-util`, and `0.13.9`. Conceptual searches covered missing and dangling package entries, ambiguous or unrecognized lockfile dependencies, and invalid lockfiles after merges. Fix-oriented checks covered parser and lock diagnostics, recent and merged pull requests, and candidate discussion and closure chains. No matching fix or duplicate surfaced.

The earlier report astral-sh/uv#11384 and its fix, astral-sh/uv#11400, concern optional versions for dynamic packages with conflicting extras, not zero package entries. The requests in astral-sh/uv#11185 and astral-sh/uv#16348 concern automatic merge resolution or explicit replacement of invalid lockfiles, and astral-sh/uv#5633 requests merge guidance; none tracks the incorrect diagnostic. The similar title of astral-sh/uv#19196 refers to merging configuration for dependency cooldowns, not to a missing lockfile graph node.
