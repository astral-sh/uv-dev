# Regression: conflicts definition in root project are not inherited

Issue: astral-sh/uv#21424

Classification: enhancement

## Summary

The reporter has a multi-package workspace whose root `pyproject.toml` sets `dev` and
`local-spark` as default dependency groups and declares `local-spark` mutually exclusive with a
`databricks` extra. The unqualified conflict declaration was accepted by uv 0.9, but uv 0.11 and
0.12 reject it with `Expected package field in conflicting entry: { group = "local-spark" }`.
They interpret this as root conflict configuration no longer being inherited by the workspace.

The exact error is emitted when uv reads conflicts from a non-project (virtual) workspace root and
cannot infer a package name for an entry. Current source and tests deliberately require
`package = ...` for conflicts at such a root. This behavior was introduced by astral-sh/uv#18886:
before that fix, uv silently ignored all conflicts declared on a virtual root. Consequently, uv
0.9 accepting this file does not establish that the conflict was enforced.

The missing capability is a way to represent a conflict involving a dependency group owned by a
virtual root, which has no project name to use as the package scope. No open issue or pull request
was found that already tracks that exact capability.

## Draft response

Thanks for the report. The exact error indicates this is being read as a virtual workspace root,
where uv has no root project name to use as the implicit package. Before astral-sh/uv#18886, uv
silently ignored `[tool.uv].conflicts` on virtual roots, so uv 0.9 was not enforcing this
declaration. Since that fix, conflicts at a virtual root are read, but each item must include
`package`; a dependency group owned directly by the virtual root cannot currently be represented
this way.

Could you provide a minimal `pyproject.toml` showing the workspace members and where the
`databricks` extra is declared? That will clarify the necessary package scoping and give us a
concrete case for supporting virtual-root groups.

## Classification

Classify as `enhancement`. The observable behavior change is real, but repository history shows
that uv 0.9 ignored virtual-root conflicts rather than successfully inheriting and enforcing them.
astral-sh/uv#18886 fixed that correctness bug by reading the declarations and intentionally
rejecting any virtual-root conflict item without an explicit package. The integration test
`lock_non_project_member_conflicts_missing_package` expects the reporter's exact class of error,
and the lock-scenario test `group_virtual` documents that conflicting groups in a project without
`[project]` are not currently supported because the internal representation requires a package
name.

Supporting an unqualified `local-spark` group owned by the virtual root would therefore add
currently absent behavior. The report does not include the complete root manifest or show where
the `databricks` extra is declared, so a minimal example is still needed to determine the intended
cross-package scope. If the root actually has an effective project identity, the example could
instead expose a bug in workspace discovery; the exact diagnostic alone indicates that uv did not
have such an identity while collecting these conflicts.

## Related

- astral-sh/uv#18879 — Closed bug reporting that `[tool.uv].conflicts` was entirely ignored on a
  virtual workspace root. It is the canonical historical report explaining why uv 0.9 could
  accept this configuration without enforcing it. Its reproduction used explicit package-level
  member conflicts, unlike astral-sh/uv#21424's unqualified root group and extra.
- astral-sh/uv#18886 — Merged pull request that fixed astral-sh/uv#18879 by collecting virtual-root
  conflicts. It also added explicit coverage that rejects entries without `package = ...` because
  a non-project root supplies no package name to infer. The fix merged on 2026-04-06 and first
  appeared in the uv 0.11 release line, matching the reported 0.9-to-0.11 change.

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
