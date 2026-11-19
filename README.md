# Avoid invalidating consumer lockfiles when a local path dependency's unrequested extras change

Issue: astral-sh/uv#23298

Classification: enhancement

## Summary

A consumer with an independent `uv.lock` depends on a sibling library through `tool.uv.sources`. Changing the library's unrequested `test` extra from `pytest>=8` to `pytest>=9` makes `uv lock --check` exit 1. Regenerating the lock changes only the library's `[package.metadata].requires-dist` entry; the two resolved packages are unchanged and `pytest` is not locked. In a monorepo, this can force unrelated consumer lockfile updates in CI.

astral-sh/uv#19081 tracks the analogous behavior for dependency groups, not extras. astral-sh/uv#21163 provides a preview mode that avoids the reported metadata-only invalidation.

## Preview workaround

A maintainer asked the reporter to try `UV_PREVIEW_FEATURES=lock-without-metadata`. Set it when regenerating the consumer lockfile with `uv lock` and when running `uv lock --check` in CI. This is the environment-variable equivalent of the flag verified in the offline reproduction below. The reporter has not yet confirmed whether it works in their setup.

## Draft response

The default `uv lock --check` compares the local library's recorded requirement declarations, including unrequested extras, not just the resolved packages. That's why this edit needs a lockfile refresh even though `pytest` isn't locked. I verified on uv 0.13.9 that the check passes after the same edit when the lockfile is generated and checked with `--preview-features lock-without-metadata`. If you can use a preview feature, regenerate the consumer lockfile with `uv lock --preview-features lock-without-metadata` and pass the same flag to `uv lock --check` in CI. For default validation, the requested change is to ignore only extras no dependency in the graph requests, while still checking base requirements and extras requested directly or transitively.

## Classification

The default check detects a real change in the declaration metadata stored in the lockfile; it is not reporting a difference in the resolved graph. Ignoring declarations for unrequested extras in default mode is a change to the freshness policy, not an established correctness regression. The dependency-group issue covers different inputs, and the merged preview work does not change default validation.

## Related

- astral-sh/uv#19081 (open issue) — The same downstream `uv lock --check` churn after an edit to a local path dependency, but for `[dependency-groups]`, which are development metadata. `[project.optional-dependencies]` declares package extras that other packages can request transitively, so this issue needs a separate activation check.
- astral-sh/uv#21163 (merged pull request) — Reimplemented the `lock-without-metadata` preview, which can omit local package declarations and validate resolved dependency edges instead. An offline reproduction on uv 0.13.9 confirmed that a lock generated with this flag stays valid after the unrequested-extra edit. This does not change default lockfiles.

## Supporting evidence

- On uv 0.13.9 (released November 17, 2026), an offline reproduction with CPython 3.12.3 passed its initial check. The default check exited 1 after changing only `pytest>=8` to `pytest>=9` in the unrequested extra. Relocking produced a one-line metadata diff and no change in package entries. A lock generated with the preview passed the same post-edit check and remained byte-for-byte unchanged.
- In a second offline preview fixture, adding a direct request for the previously unused extra required a relock and added its local dependency. Removing that edge from the requested extra or adding a base dependency also required a relock. These checks exercise changes to resolved edges, not every possible extra or version-constraint combination.
- `crates/uv-lock/src/lock/mod.rs:3944` (`satisfies_requires_dist`) compares normalized current and recorded `requires-dist` declarations when metadata is present. In the metadata-free path, `crates/uv-lock/src/lock/mod.rs:4164` skips a non-workspace package's extra when no incoming edge requests it. `docs/concepts/projects/sync.md:35` describes `--check` as checking freshness against metadata.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with `gh` for the freshness error, `uv lock --check`, `requires-dist`, `provides-extras`, and literal extra and path-source wording. Searched separately for unused versus transitively activated extras, newly requested extras, base requirements, downstream lock churn, dependency groups, and metadata-free validation. Used PR listings, changelogs, and linked records to supplement rate-limited PR searches.

Ruled out astral-sh/uv#22046 and its fix astral-sh/uv#22055: they concern rechecking an unchanged preview lock with conflicting dependency groups. Also ruled out astral-sh/uv#20672 and astral-sh/uv#23203: they address URL lookahead for a transitively requested extra during resolution, not metadata-only changes to an unrequested extra after locking. The cross-reference from astral-sh/uv#19081 to astral-sh/uv-dev#116 concerns missed revalidation of mutable transitive sources, not ignoring unused extras.
