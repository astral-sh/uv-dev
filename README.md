# uv check --no-install-project

Issue: astral-sh/uv#21083

Classification: enhancement

## Summary

The reporter has a project with compiled extension modules that are expensive to build and require
external dependencies. They can prepare a dependency-only environment with
`uv sync --no-install-project` and then run `ty check`, but the `ty-pre-commit` hook invokes
`uv check`, which synchronizes and installs the project. They request `uv check
--no-install-project` so the hook can maintain the project's Python dependencies without building
or installing the project itself.

The current source confirms the mechanism: `uv check` discovers and locks the project, identifies
the installation target, and calls project synchronization with `InstallOptions::default()`.
`CheckArgs` includes extras, dependency groups, package selection, and `--no-sync`, but it does not
include `--no-install-project`. This is a missing command-line capability rather than a failure of
documented behavior.

Maintainer discussion now questions whether installing the local project should happen at all for
`uv check`. One maintainer considers the proposed flag acceptable to add while a broader design is
considered. Another proposes making project omission the default, on the basis that local C
extensions are not needed for type checking: when stubs exist, ty can resolve the import from them;
without stubs, ty currently reports an error. This is a design proposal, not yet an agreed default.

No existing issue or pull request was found for this exact `uv check` option. astral-sh/uv#6578 is
the closest open conceptual request, but it explicitly covers `add`, `remove`, and `run` and predates
`uv check`. The historical and implementation precedents are astral-sh/uv#4028, which introduced
dependency-only synchronization, and astral-sh/uv#20233 with astral-sh/uv#20628, which added other
sync-derived selection flags to `uv check` for a `ty-pre-commit` workflow.

## Maintainer direction

A maintainer initially suggested a configuration-level way for a project to declare that its stubs
make installation unnecessary, then indicated that adding `--no-install-project` is acceptable in
the meantime. A second maintainer suggested a broader behavior change: omit the project by default
for `uv check`, because building local C extensions provides no type information that ty needs.
The choice among an opt-in flag, stub-aware configuration, and a new default remains unresolved.

## Type-checking behavior

According to a ty maintainer, local C extensions do not need to be built for type checking:

- If the extension has stubs, ty resolves the import using those stubs.
- If it has no stubs, ty currently emits an error rather than gaining useful type information from
  the built extension.
- A future goal is to recognize an installed extension without stubs as an opaque module whose
  members resolve to `Any`, while still emitting a diagnostic. That behavior is not implemented.

Consequently, building a large extension would at most enable a somewhat more specific diagnostic
under that future behavior, which the maintainer does not consider worth the build cost.

## Current workaround

`ty-pre-commit` forwards its arguments to `uv check`, so the environment can first be prepared with
`uv sync --no-install-project` and the hook can be given `--no-sync`. This preserves the
dependency-only environment, but the hook will not update it.

## Classification

Classify this as an `enhancement`. The requested flag is not present on `uv check`, and the command
currently passes default installation options to its synchronization path. The report asks to add a
dependency-only synchronization mode already available on `uv sync`; it does not establish that
`uv check` violates documented behavior. `uv check` is also explicitly experimental. Maintainers
agree that avoiding unnecessary local extension builds merits a change, but have not decided whether
the result should be an opt-in flag, configuration, or the command's default behavior.

This is not a duplicate. astral-sh/uv#6578 has the same broad motivation—avoid installing a costly
compiled local project when another command synchronizes the environment—but its stated scope is
`add`, `remove`, and `run`. No open issue or pull request was found that tracks
`uv check --no-install-project` specifically.

## Related

- astral-sh/uv#6578 — Open issue requesting `--no-install-*` on `add`, `remove`, and `run`, with the
  same expensive Rust-project-build motivation. It is the strongest adjacent result, but its command
  scope differs from `uv check`.
- astral-sh/uv#20233 — Closed issue from a `ty-pre-commit` user requesting `--package` and
  `--all-packages` on `uv check`. A maintainer confirms that the hook forwards arguments to
  `uv check`; the requested selection behavior differs from omitting project installation.
- astral-sh/uv#20628 — Merged pull request implementing astral-sh/uv#20233 by extending the
  `uv check` CLI/settings and package-selection synchronization path. It is implementation precedent
  for sync-derived controls but does not add `--no-install-project`.
- astral-sh/uv#4028 — Closed canonical discussion that introduced installing project dependencies
  without the project itself for `uv sync`, particularly to avoid costly project builds. It defines
  the desired semantics but does not cover `uv check`.

## Search evidence

Exact searches combined `uv check` with `no-install-project`, `install project`, and related flag
spellings; only astral-sh/uv#21083 matched the exact request. Conceptual searches covered skipping or
avoiding project installation, dependency-only environments, heavy builds, `ty-pre-commit`, and
commands that synchronize environments. Historical and fix-oriented searches covered closed issues
and merged pull requests for the origin of `--no-install-project` and for prior additions to
`uv check`.

astral-sh/uv#19790 and astral-sh/uv#19791 were inspected but ruled out because they concern a broken
Python environment and custom ty configuration, respectively. astral-sh/uv#19601 was also ruled out:
it concerns composing existing `uv sync` flags, not extending `uv check`.
