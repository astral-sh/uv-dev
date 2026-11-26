# `uv lock --upgrade` writes a dead edge (`python_version < '0'`) that a lock-guided `uv lock` drops, so uv.lock flips

Issue: astral-sh/uv#23410

Classification: bug

## Summary

A fresh `uv lock` and `uv lock --upgrade` write an unreachable dependency edge in a project whose resolution forks by Python version. After a project version bump forces a lock-guided re-resolution, plain `uv lock` omits that edge. For the same final project inputs and selected packages, the two paths produce lockfiles that differ by one dependency entry. The closest precedents are fixed bugs in false-marker output and impossible-marker lock oscillation, but neither covers this edge. The open fork-pruning and marker-order discussions are adjacent, not duplicates.

## Draft response

Your four-wheel example reproduces on uv 0.13.0 and 0.13.10. The only difference after the version bump is that `uv lock --upgrade` puts back an `alpha` → `beta 2.0` edge that can never apply. Plain `uv lock --dry-run` accepts either already-valid lock, but `uv lock --upgrade --dry-run` against the lock without the edge does report `Lockfile changes detected`. The next step is a regression test that checks that fresh, full-upgrade, and lock-guided resolutions all omit the impossible edge.

## Classification

This is a correctness bug: equivalent resolutions should not differ by an unreachable dependency entry. The earlier fixes covered impossible forks and cross-variable marker normalization, not an impossible edge between reachable packages. No existing issue or pull request was found that tracks this specific behavior, and the evidence does not establish a regression of either earlier fix.

## Related

- astral-sh/uv#8676 (closed) — `uv pip compile --universal` emitted the same false-marker sentinel as a requirements line. It did not involve an edge in `uv.lock` or compare fresh and lock-guided resolutions.
- astral-sh/uv#8759 (merged) — Fixed astral-sh/uv#8676 by skipping forks whose combined markers were disjoint from the Python requirement. The new fixture needs both Python forks and both `beta` versions; only `alpha`’s edge to `beta 2.0` is impossible.
- astral-sh/uv#20368 (closed) — Repeated plain locks alternated between two impossible top-level platform dependencies, changing which package was selected and failing `--check`. The new report retains the same packages and alternates resolution modes.
- astral-sh/uv#20369 (merged) — Fixed astral-sh/uv#20368 by separating unchecked marker-algebra operations from the regular cache. Here the marker is correctly normalized to false; the issue is that the resulting edge is written.
- astral-sh/uv#6176 (open) — Proposes applying fork markers at fork time so the solver does not process dependencies disjoint from a fork. Maintainers explicitly discuss the difference between discovered and pre-seeded forks, but the issue’s example discards the unreachable output and does not track persisted false edges.
- astral-sh/uv#22040 (open) — Tracks forked-lock churn when existing forks are reused, usually a reordering of `resolution-markers`. A follow-up also shows equivalent marker rewrites, but neither report shows an always-false dependency edge being added.
- astral-sh/uv#23403 (closed, unmerged) — Attempted to fix astral-sh/uv#22040 by preserving the order of reused forks. It explicitly limited its scope to ordering and did not change dependency-edge filtering.

## Verification

Recreated the reported fixture offline with four generated wheels, using the official Linux x86_64 builds of uv 0.13.0 and 0.13.10 and CPython 3.12.3. The project supports Python 3.10 and later. `alpha` requires `beta` only below Python 3.10.2; `gamma` requires `beta>=2` on Python 3.11 and later; `beta 2.0` itself requires Python 3.11 or later.

1. A fresh `uv lock` writes one always-false edge from `alpha` to `beta 2.0`.
2. Bumping only the project version from `1.0.0` to `1.0.1` and running plain `uv lock` removes the edge.
3. `uv lock --upgrade` restores it. On 0.13.10, the before/after diff is exactly that added edge. Plain locking without changing the project again leaves the upgraded lock intact.

On both releases, upgrade dry-run against the guided lock reports `Lockfile changes detected`. Plain dry-run reports no changes for either already-valid lock (both forms checked on 0.13.0). This verifies the lockfile churn, not a separate failure of dry-run to report the upgrade diff.

## Source evidence

- `crates/uv-lock-operations/src/validated_lock.rs:120` marks an existing lock unusable for a full upgrade. `crates/uv-lock-operations/src/lock.rs:860` returns a satisfying lock unchanged, while `crates/uv-lock-operations/src/lock.rs:888` reuses eligible pins and fork markers when a re-resolution is needed.
- `crates/uv-resolver/src/resolution/output.rs:252` intersects each dependency marker with its fork marker and can add the resulting edge even if it is false. Final graph cleanup at `crates/uv-resolver/src/resolution/output.rs:193` prunes unreachable nodes, not false edges between independently reachable nodes.
- `crates/uv-lock/src/lock/mod.rs:7094` passes graph edges to the lock dependency builder, which simplifies the marker and constructs the dependency at `crates/uv-lock/src/lock/mod.rs:1499`. False markers are rendered as `python_version < '0'` by `crates/uv-pep508/src/marker/tree.rs:1888`. These paths explain how a false edge can reach the lockfile without an unreachable package being selected.
- `crates/uv-project-commands/src/lock.rs:193` prints a generic change notice for a changed lock even if there are no version events to list, consistent with the upgrade dry-run result.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated `gh`. Literal searches covered `python_version < '0'`, `dead edge`, `lock --upgrade`, `No lockfile changes detected`, and the example package and Python bounds. Symptom searches covered lock idempotence, lockfile churn, dry-run output, and fresh versus guided locks. Fix-oriented searches covered impossible or disjoint markers, reachability, fork preferences, and marker ordering; checked linked fixes, comments, and recent PR titles by state.

Also ruled out astral-sh/uv#15810 and its fix astral-sh/uv#15817 (same-version Git SHA reporting), astral-sh/uv#23394 (ignores impossible input pins when deciding whether to allow yanked versions in pip compile, not lockfile output), and astral-sh/uv#23142 (a repeat targeted upgrade fails resolution instead of successfully writing an equivalent graph).
