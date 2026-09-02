# Regression: conflicts definition in root project are not inherited

Issue: astral-sh/uv#21424

Classification: question

## Summary

The reporter has a multi-package workspace whose root `pyproject.toml` sets `dev` and
`local-spark` as default dependency groups and declares `local-spark` mutually exclusive with a
`databricks` extra. The unqualified conflict declaration was accepted by uv 0.9, but uv 0.11 and
0.12 reject it with `Expected package field in conflicting entry: { group = "local-spark" }`.
They interpret this as root conflict configuration no longer being inherited by the workspace.

The exact error is emitted when uv reads conflicts from a non-project (virtual) workspace root and
cannot infer a package name for an entry. Current source and tests deliberately require
`package = ...` for conflicts at such a root. This behavior was introduced in uv 0.11.4 by
astral-sh/uv#18886: before that fix, uv silently ignored all conflicts declared on a virtual root.
Consequently, uv 0.9 accepting this file does not establish that the conflict was enforced.

A maintainer confirmed that the diagnostic is correct and that the supported configuration is to
add the owning `package` to each conflict entry. This resolves the report as a configuration and
scoping clarification rather than missing inheritance behavior.

## Maintainer guidance

For a root `pyproject.toml` without a `[project]` table, uv has no implicit package name for an
unqualified `group` or `extra`. Each conflict item should therefore identify its owner explicitly,
for example by adding `package = "<owning-package>"` alongside `group = "local-spark"` and
`extra = "databricks"` as appropriate. The issue does not yet identify those owning packages, so
the handoff cannot provide the exact corrected declaration.

## Classification

Classify as `question`. The observable behavior change is real, but repository history and the
maintainer response establish that uv 0.9 ignored virtual-root conflicts rather than successfully
inheriting and enforcing them. astral-sh/uv#18886 fixed that correctness bug in uv 0.11.4 by
reading the declarations and intentionally rejecting any virtual-root conflict item without an
explicit package. The integration test
`lock_non_project_member_conflicts_missing_package` expects the reporter's exact class of error,
and the maintainer confirmed that adding the owning package is the supported fix. No incorrect
current behavior or missing capability remains established by the available report.

## Related

- astral-sh/uv#18879 — Closed bug reporting that `[tool.uv].conflicts` was entirely ignored on a
  virtual workspace root. It is the canonical historical report explaining why uv 0.9 could
  accept this configuration without enforcing it. Its reproduction used explicit package-level
  member conflicts, unlike astral-sh/uv#21424's unqualified root group and extra.
- astral-sh/uv#18886 — Merged pull request that fixed astral-sh/uv#18879 by collecting virtual-root
  conflicts. It also added explicit coverage that rejects entries without `package = ...` because
  a non-project root supplies no package name to infer. The fix merged on 2026-04-06 and appeared
  in uv 0.11.4, matching the reported 0.9-to-0.11 change.

## Search evidence

Literal searches covered the full error, `package field`, `conflicting entry`, `default-groups`,
and mixed `{ group = ... }` / `{ extra = ... }` declarations. Conceptual searches covered virtual
and non-project workspace roots, root conflict inheritance, workspace conflict ownership,
package-scoped extras and groups, and conflict propagation. Fix-oriented searches covered closed
issues and merged pull requests across the uv 0.9 through 0.12 interval.

astral-sh/uv#10405 was inspected but concerns declaring conflicts for a workspace dependency's
extras and unconditional activation, not package-less conflicts owned by a virtual root.
astral-sh/uv#18015 and its fix astral-sh/uv#18096 concern propagation of project-level package
conflicts to that package's extras and groups; they assume named projects and do not address this
missing root identity. astral-sh/uv#18317 concerns lockfile size and resolver-fork growth when
members repeat conflict declarations, so it is adjacent operationally but not the same behavior.
