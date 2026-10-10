# Setting to error on "constraint-dependencies" not being used

Issue: astral-sh/uv#22457

Classification: enhancement

## Summary

A constraint added to work around a transitive dependency's bug can remain in `pyproject.toml` after upgrading or removing the direct dependency eliminates that transitive dependency. The reporter proposes an opt-in setting, tentatively `require-use-dependencies = true`, to make `uv lock` fail for unused entries in `constraint-dependencies`, `build-constraint-dependencies`, or `override-dependencies`. A lint through `uv check` or Ruff is suggested as an alternative interface.

No duplicate found. The closest discussions cover ineffective sources, stale cooldown exceptions, pyproject.toml validation, and Ruff integration. The report supplies no reproduction, uv version, platform, or error output; it describes a requested capability rather than a failed resolution.

## Draft response

Your proposed opt-in error or lint would be new functionality: constraints and overrides currently allow entries for packages that are not otherwise required. Related requests cover ineffective sources in astral-sh/uv#13829 and stale cooldown exceptions in astral-sh/uv#18792, but neither covers these settings.

The next step is to define “unused” across workspace members, environment markers, resolver backtracking, and build dependencies before choosing a flag or lint interface. Absence from the final runtime lockfile alone is insufficient: a setting can still affect resolution. `uv check` currently runs ty; broader lint integration is discussed in astral-sh/uv#16314.

## Classification

The report requests new opt-in validation for stale configuration. Constraints and unscoped overrides intentionally do not require their packages to be present, and existing tests accept constraints left after dependency removal. No incorrect resolution or regression is reported. The related discussions address different settings or broader lint infrastructure, so none establishes a duplicate.

The source supports the distinction between allowing unused configuration and introducing diagnostics for it. The current checkout also contains preview support for pruning resolution inputs from `uv.lock`; that behavior does not implement the requested error or remove stale entries from `pyproject.toml`. There is no evidence of a previously fixed validation feature regressing.

## Related

- astral-sh/uv#13829 (open) — `[tool.uv.sources]` should warn if it has no effect on any platform. Requests diagnostics for ineffective dependency configuration, including absent dependencies and disjoint markers. Maintainer comments require considering all workspace members. Closely related validation design, but it concerns sources rather than unused constraints, build constraints, or overrides.
- astral-sh/uv#18792 (open) — Prune stale `exclude-newer-package` entries on `uv lock`. Describes temporary dependency workarounds accumulating as dead configuration and requests detection or removal during locking. Its trigger is an expired cooldown exception, and its requested action is pruning; this report requests errors for constraint or override entries after dependencies disappear.
- astral-sh/uv#15006 (open) — Feature request: `pyproject.toml` formatting and validation. Requests configuration validation, including tool-specific semantics, through a possible check interface. Maintainers describe it as unlikely in the near term and mention Ruff's existing TOML validation. It does not specifically request resolution-aware detection of unused dependency policies.
- astral-sh/uv#16314 (open) — `ruff check` equivalent to `uv format` for `ruff format`. Tracks Ruff lint integration. A maintainer closed astral-sh/uv#21392 into this discussion while expressing a preference for combining Ruff and ty through uv check. Relevant to the proposed lint interface, but it does not define an unused-constraint rule.

## Supporting evidence

- `crates/uv-workspace/src/pyproject.rs:537` documents that an unscoped override does not install its package unless it is requested elsewhere. The corresponding runtime-constraint documentation begins at line 599 and build-constraint documentation at line 630. These settings restrict requested dependencies rather than require every configured package to appear.
- `crates/uv/tests/lock/lock.rs:46712`, test `lock_resolution_inputs_dynamic_constraints`, includes removing a dependency while retaining its version constraint; the final lock invocation succeeds. This directly covers the reported trigger under the resolution-inputs preview.
- `crates/uv/tests/lock/lock.rs:46866`, test `lock_resolution_inputs_prune_unused_inputs`, explicitly expects successful locking with unused constraints and overrides, and verifies their omission from the lockfile under the preview feature.
- `crates/uv-preview/src/lib.rs:431` describes the `resolution-inputs` preview. `crates/uv-lock/src/lock/inputs.rs:16` implements pruning of recorded runtime settings and explains that settings consulted during backtracking can matter even when their packages are absent from the final graph. `crates/uv-lock-operations/src/lock.rs:1022` applies that pruning to the lock object.
- `crates/uv-cli/src/lib.rs:1027` documents `uv check` as currently checking Python code with ty. It does not advertise dependency-policy linting.
- In astral-sh/uv#13774, maintainers distinguished configuration that is temporarily unused during installation from sources that can never apply, and directed the warning request to astral-sh/uv#13829. The latter explicitly calls for considering dependencies in every workspace member.
- In astral-sh/uv#15006, maintainers said broader configuration validation was unlikely in the near term and pointed to Ruff's existing TOML validation. That comment does not establish an existing lint for unused uv constraints.
- In astral-sh/uv#21392, a maintainer preferred extending `uv check` to combine Ruff and ty and centralized the lint-wrapper discussion in astral-sh/uv#16314. astral-sh/uv#19768 independently requests broader checks, without an unused-constraint rule.

These are documentation, source, test-snapshot, and discussion findings. No executable reproduction or tests were run, and no released version is asserted to contain the checkout's preview behavior.

## Scope and design questions

The report was decomposed before searching into:

1. Runtime constraints left behind after a direct dependency is upgraded or removed.
2. The equivalent requested diagnostic for build constraints, whose dependency graph differs from the runtime graph.
3. The equivalent requested diagnostic for overrides.
4. The proposed interfaces: opt-in failure during locking or a separate configuration lint.

A design needs to distinguish a package absent from the final graph, a declaration inapplicable under particular markers, a setting consulted during backtracking, and a bound that is satisfied but still intentionally restricts future resolution. Build constraints also need an explicit definition of use when a lock operation does not build a package. These are design considerations, not confirmed bugs or implementation commitments.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separately searched constraint-dependencies, build-constraint-dependencies, override-dependencies, and require-use-dependencies; expanded to unused, stale, obsolete, orphan, redundant, warnings, no effect, pruning, configuration validation, and uv check/Ruff integration. Fix-oriented PR searches covered the setting names, unused constraints/overrides, pruning, resolution-inputs, and astral-sh/uv#13829. Inspected candidate bodies and comments and followed discussion chains to astral-sh/uv#13829 and astral-sh/uv#16314. Ruled out astral-sh/uv#13904 (source-code usage analysis), astral-sh/uv#19261 (constraints ignored depending on working directory), astral-sh/uv#12276 (invalid manually edited lockfiles), and astral-sh/uv#18921 (cooldown bypass implementation). No matching implementation or regression fix was found. Some supplemental queries were rate-limited; subsequent explicit PR status searches succeeded.

Additional inspected chains included astral-sh/uv#12097 leading to astral-sh/uv#13904; astral-sh/uv#15006 referencing astral-sh/uv#6308; astral-sh/uv#13829 referencing astral-sh/uv#8253; and astral-sh/uv#18792 referencing astral-sh/uv#17999 and astral-sh/uv#19864, with the latter linking astral-sh/uv#18921. These concern import-based dependency cleanup, general TOML tooling, transitive source selection, or cooldown bypasses. None establishes a canonical discussion for errors on the three settings in this report.

## Handoff status

The README is the only file authored for this task. No checkout files were modified and no GitHub comments, labels, or other state were changed. The checkout already contains a modification to `agents/codex/config.toml` and an untracked `.issue-triage-event.json`; both were left untouched.
