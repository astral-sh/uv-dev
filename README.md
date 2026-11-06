# Strict `fork-strategy=fewest` - resolve to a single version for each package

Issue: astral-sh/uv#23128

Classification: enhancement

## Summary

The reporter uses uv 0.13.7 and wants universal resolution to select exactly one version of each package across supported Python versions, or fail when that cannot be achieved. They understand that `fork-strategy = "fewest"` does not provide this guarantee and request a feature if no existing setting does so. Their current workaround checks repeated package names in `uv.lock` using `grep "^name =" uv.lock | sort | uniq -d`, with a failing CI variant alongside `uv lock --check`.

No exact duplicate found. astral-sh/uv#12782 covers related fewest minimization, astral-sh/uv#18337 explains legitimate multi-version locks and global constraints, and merged astral-sh/uv#21000 fixes environment ordering without adding a strict mode.

The issue has no reproduction demonstrating an incorrect resolution and no comments at review time. Its current GitHub label is `question`; the proposed classification is `enhancement`.

## Draft response

There isn't currently a single-version-or-fail fork strategy. `fewest` prefers fewer versions but still permits different versions for different environments; `uv lock --check` checks whether the lockfile is up to date, rather than enforcing that policy. This would be an enhancement.

For now, keep your separate CI check. For specific packages, an exact pin without environment markers in `tool.uv.constraint-dependencies` in the workspace-root `pyproject.toml` can require the same version wherever that package is used. astral-sh/uv#12782 tracks a related minimization bug, but does not cover rejecting all multi-version solutions.

## Classification

The request adds a hard one-version-per-package constraint and failure behavior to universal resolution. Documentation and the ForkStrategy enum establish that fewest is an optimization strategy and only fewest/requires-python are available. Multiple versions across disjoint environments are supported behavior; no incorrect resolution or regression is established here. Related minimization bugs do not cover rejecting solutions that require multiple versions, and no existing issue tracking that same strict capability was found.

The distinction is between preferring fewer versions and making a multi-version solution an error. A failure to minimize when a shared version exists can be a bug, as in astral-sh/uv#12782. This report instead requests a universal policy, including failure when no shared version is available. The historical environment-ordering fix predates this report, but there is no matching reproduction or claim establishing a regression.

## Related

- astral-sh/uv#12782 — `fork-strategy` `fewest` does not limit versions when combined with `required-environments` (open). Requests consistent versions across environments: fewest selects two open3d versions although one satisfies all required targets, and a maintainer acknowledged the report. This concerns improving minimization with required-environments, whereas the new request would reject every multi-version solution, including cases where splitting is necessary.
- astral-sh/uv#18337 — constraint-dependencies ignored for transitive dep, leading to multiple versions of package in uv.lock (closed). Also describes a one-version policy and duplicate package versions in uv.lock. Maintainer discussion established that its marker-conditioned constraint legitimately permits different versions elsewhere and suggested omitting the marker to constrain all environments. It provides a targeted workaround, rather than a general single-version-or-fail strategy.
- astral-sh/uv#21000 — Respect `fork-strategy` for `environments` (merged). Merged on 2026-08-08 to fix astral-sh/uv#20999 by ordering initial Python environment forks according to the selected strategy. It fixes unnecessary versions caused by environment ordering, but adds no strict rejection mode. The new report supplies no evidence that this historical bug has regressed.

## Supporting evidence

- `docs/concepts/resolution.md:401` explains that universal locks may contain multiple versions or URLs for different Python versions or platforms. Its multi-version resolution section describes `fewest` as minimizing selected versions, preferring releases compatible with more targets.
- `crates/uv-configuration/src/fork_strategy.rs:5` defines only `Fewest` and `RequiresPython`. Neither is a single-version-or-fail strategy. The setting documentation at `crates/uv-settings/src/settings.rs:1004` describes the same optimization tradeoff.
- `docs/concepts/projects/sync.md:35` and `crates/uv-cli/src/lib.rs:3813` establish that `uv lock --check` checks lockfile freshness and is equivalent to locked mode. It does not express the requested additional policy.
- `crates/uv-workspace/src/pyproject.rs:602` documents dependency constraints as restricting selected versions without adding a package to the resolution. It specifies that project commands read these constraints from the workspace-root `pyproject.toml`. The maintainer's final comment on astral-sh/uv#18337 explains why removing a constraint's environment marker applies it across environments.
- The maintainer acknowledged the expectation in astral-sh/uv#12782 on 2025-04-10. Its reproduction uses `required-environments` and shows two versions even though an older version covers all required targets; it does not request failure when splitting is unavoidable.
- The body and review of astral-sh/uv#21000 confirm a change to initial fork ordering, following astral-sh/uv#10007. The latter fixed astral-sh/uv#9998 by honoring fork strategy when ordering dependency-created Python splits. These changes concern resolution preferences rather than enforcing a new hard constraint.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using fork-strategy/fewest, single version, one version, single-version, one-version, strict resolution, no forks, disabling forking, disallow fork, uniform versions, multiple versions with fail, duplicate versions, duplicate lock checks, and the exact uniq -d workaround. Used area:resolver, area:lock, and enhancement terminology, and inspected candidate bodies, maintainer comments, review comments, and reference chains. PR keyword searches returned no hits; referenced merged astral-sh/uv#21000 and astral-sh/uv#10007 were inspected directly. Historical ordering fixes were distinguished from the requested policy. Ruled out astral-sh/uv#5598, which prevents simultaneous conflicting versions within one marker environment, and astral-sh/uv#21484, which concerns lockfile integrity rather than cross-environment version uniformity. Also inspected astral-sh/uv#14588, astral-sh/uv#5161, astral-sh/uv#4926, astral-sh/uv#9735, and astral-sh/uv#17866; none tracks a general single-version-or-fail mode.

In particular, astral-sh/uv#5598 is a plausible wording match because it discusses rejecting locks containing multiple versions, but its invariant is one version within a single marker environment. It allows different versions across disjoint environments, which the new request would forbid. astral-sh/uv#14588 requests the opposite preference: newer versions per Python implementation even when that creates more versions.

## Handoff status

Only this issue-context README and a temporary structured result were authored outside the checkout. No GitHub changes were made. No builds or tests were needed for this capability triage; the findings rely on repository documentation, source declarations, and the discussions above.
