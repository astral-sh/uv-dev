# `uv lock --upgrade` writes a dead edge (`python_version < '0'`) that a lock-guided `uv lock` drops, so uv.lock flips

Issue: astral-sh/uv#23410

Classification: bug

## Summary

A fresh `uv lock` and `uv lock --upgrade` write an unreachable dependency edge in a project whose resolution forks by Python version. After a project version bump forces a lock-guided re-resolution, plain `uv lock` omits that edge. Given the same final project inputs and selected packages, the two paths produce lockfiles that differ by one dependency entry. Alternating commands alone on an unchanged project does not flip the file: plain locking leaves the valid upgraded lock intact. Upgrade dry-run does detect the edge-only diff when starting from the guided lock.

## Classification

This is a lockfile stability bug: equivalent resolutions differ by an unreachable dependency entry. The reproduction does not show an incorrect package selection or a missed diff in upgrade dry-run. The earlier fixes covered impossible forks and cross-variable marker normalization, not an impossible edge between reachable packages. No existing issue or pull request was found that tracks this specific behavior, and the evidence does not establish when it was introduced or that it is a regression of an earlier fix.

## Related

- astral-sh/uv#8676 (closed) — `uv pip compile --universal` emitted the same false-marker sentinel as a requirements line. It did not involve an edge in `uv.lock` or compare fresh and lock-guided resolutions.
- astral-sh/uv#8759 (merged) — Fixed astral-sh/uv#8676 by skipping forks whose combined markers were disjoint from the Python requirement. The new fixture needs both Python forks and both `beta` versions; only `alpha`’s edge to `beta 2.0` is impossible.
- astral-sh/uv#20368 (closed) — Repeated plain locks alternated between two impossible top-level platform dependencies, changing which package was selected and failing `--check`. The new report retains the same packages and alternates resolution modes.
- astral-sh/uv#20369 (merged) — Fixed astral-sh/uv#20368 by separating unchecked marker-algebra operations from the regular cache. Here the marker is correctly normalized to false; the issue is that the resulting edge is written.
- astral-sh/uv#6176 (open) — Proposes applying fork markers at fork time so the solver does not process dependencies disjoint from a fork. Maintainers explicitly discuss the difference between discovered and pre-seeded forks, but the issue’s example discards the unreachable output and does not track persisted false edges.
- astral-sh/uv#22040 (open) — Tracks forked-lock churn when existing forks are reused, usually a reordering of `resolution-markers`. A follow-up also shows equivalent marker rewrites, but neither report shows an always-false dependency edge being added.
- astral-sh/uv#23403 (closed, unmerged) — Attempted to fix astral-sh/uv#22040 by preserving the order of reused forks. It explicitly limited its scope to ordering and did not change dependency-edge filtering.

## Reproduction

**Reproducible.** Independently generated four local `py3-none-any` wheels and reproduced the behavior on Linux x86_64 with CPython 3.12.3, using uv 0.13.9 from `PATH` and the official uv 0.13.0 and 0.13.10 binaries. The projects, wheelhouses, and caches were isolated in temporary directories; all lock commands ran offline.

The project is a standalone `demo` (initially `1.0.0`) with `requires-python = ">=3.10"`, `dependencies = ["alpha", "gamma"]`, and `[tool.uv]` settings `find-links = ["wheels"]` and `no-index = true`. The wheels have the following metadata:

| Wheel | Relevant metadata |
| --- | --- |
| `alpha 1.0` | `Requires-Dist: beta; python_full_version < '3.10.2'` |
| `gamma 1.0` | `Requires-Dist: beta>=2; python_version >= '3.11'` |
| `beta 1.0` | No dependencies or Python restriction |
| `beta 2.0` | `Requires-Python: >=3.11`; no dependencies |

1. Run `uv lock --offline` with no `uv.lock`. The lock forks at Python 3.11 and includes an `alpha` → `beta 2.0` dependency with `marker = "python_version < '0'"`.
2. Change only the project version from `1.0.0` to `1.0.1` and run `uv lock --offline` again. The lock-guided re-resolution omits that edge.
3. Run `uv lock --offline --upgrade`. The edge returns. On all three versions, the diff against step 2 is exactly one added dependency entry; the selected packages and fork markers are identical. On uv 0.13.10, bumping the version again to `1.0.2` and repeating the plain lock and upgrade produced the same one-line diff.

`uv lock --offline --dry-run` reports `No lockfile changes detected` for either already-valid lock. In contrast, `uv lock --offline --upgrade --dry-run` reports `Lockfile changes detected` against the guided lock and leaves the file unchanged; once the edge is present, it reports no changes. A subsequent plain lock without a project edit leaves the upgraded file byte-for-byte unchanged.

Nearby coverage in `crates/uv/tests/lock/lock.rs`: `lock_impossible_platform_markers` checks that contradictory top-level platform requirements are retained only in metadata, not as resolved edges, and that `--check` accepts the lock. `lock_dry_run_noop` checks both plain and upgrade dry-run against an unchanged Python-forked lock. Neither test compares edges between independently reachable packages across fresh and lock-guided re-resolutions.

## Source evidence

- `crates/uv-lock-operations/src/validated_lock.rs:120` marks an existing lock unusable for a full upgrade. `crates/uv-lock-operations/src/lock.rs:860` returns a satisfying lock unchanged; re-resolution can reuse eligible pins and fork markers.
- `crates/uv-resolver/src/resolution/output.rs:252` intersects each dependency marker with its fork marker and can add the resulting edge even if it is false. Final graph cleanup at `crates/uv-resolver/src/resolution/output.rs:193` prunes unreachable nodes, not false edges between independently reachable nodes.
- `crates/uv-lock/src/lock/mod.rs:7094` passes graph edges to the lock dependency builder, which simplifies the marker and constructs the dependency at `crates/uv-lock/src/lock/mod.rs:1499`. False markers are rendered as `python_version < '0'` by `crates/uv-pep508/src/marker/tree.rs:1888`. These are candidate investigation points; the reproduction did not trace where the two resolution paths first diverge.
- `crates/uv-project-commands/src/lock.rs:193` prints a generic change notice for a changed lock even if there are no version events to list, consistent with the upgrade dry-run result.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated `gh`. Literal searches covered `python_version < '0'`, `dead edge`, `lock --upgrade`, `No lockfile changes detected`, and the example package and Python bounds. Symptom searches covered lock idempotence, lockfile churn, dry-run output, and fresh versus guided locks. Fix-oriented searches covered impossible or disjoint markers, reachability, fork preferences, and marker ordering; checked linked fixes, comments, and recent PR titles by state.

Also ruled out astral-sh/uv#15810 and its fix astral-sh/uv#15817 (same-version Git SHA reporting), astral-sh/uv#23394 (ignores impossible input pins when deciding whether to allow yanked versions in pip compile, not lockfile output), and astral-sh/uv#23142 (a repeat targeted upgrade fails resolution instead of successfully writing an equivalent graph).
