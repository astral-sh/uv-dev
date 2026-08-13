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

No existing issue or pull request was found for this exact `uv check` option. astral-sh/uv#6578 is
the closest open conceptual request, but it explicitly covers `add`, `remove`, and `run` and predates
`uv check`. The historical and implementation precedents are astral-sh/uv#4028, which introduced
dependency-only synchronization, and astral-sh/uv#20233 with astral-sh/uv#20628, which added other
sync-derived selection flags to `uv check` for a `ty-pre-commit` workflow.

## Draft response

Thanks — this is a distinct gap in the current `uv check` interface. `uv check` synchronizes the
selected project with the default install options, but it does not currently expose
`--no-install-project`; `--no-sync` skips synchronization entirely, so it is not equivalent when the
hook should update dependencies.

As a workaround, `ty-pre-commit` forwards its arguments to `uv check`, so you can run
`uv sync --no-install-project` first and pass `--no-sync` to the hook. That preserves the
dependency-only environment, but the hook will not update it. We can keep astral-sh/uv#21083 scoped
to adding project-install selection to `uv check`; astral-sh/uv#6578 is related but covers other
commands.

## Classification

Classify this as an `enhancement`. The requested flag is not present on `uv check`, and the command
currently passes default installation options to its synchronization path. The report asks to add a
dependency-only synchronization mode already available on `uv sync`; it does not establish that
`uv check` violates documented behavior. `uv check` is also explicitly experimental.

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
