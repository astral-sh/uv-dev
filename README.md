# Add `--active` flag for `uv lock`

Issue: astral-sh/uv#21009

Classification: duplicate

## Summary

The report asks for an `--active` option on `uv lock` so that locking can use an activated
environment instead of failing when the project path `.venv` exists but is not itself a virtual
environment. The example uses `.venv` as a container for separate `3.10`, `3.12`, and `3.14`
environments and currently requires temporarily renaming that directory.

astral-sh/uv#19832 is the canonical report. It was opened by the same reporter, describes the same
`uv lock` command, directory layout, exact invalid-environment error, and rename workaround, and
already proposes `--active` as one possible remedy. The new issue explicitly presents the flag as a
temporary solution for astral-sh/uv#19832 and adds no distinct trigger or capability that needs a
separate discussion.

Two implementation attempts are linked from the canonical issue. The open astral-sh/uv#19833 takes
the more direct approach of letting `uv lock` ignore an invalid project environment and fall back to
global or managed interpreter discovery. The closed astral-sh/uv#20333 proposed the same fallback;
it was closed by its author without review comments, so its closure does not establish a maintainer
decision on the design.

## Draft response

Thanks. This request is already covered by astral-sh/uv#19832: it reports the same `uv lock`
failure for a non-virtual-environment `.venv`, including the version-subdirectory layout, and
already identifies `--active` as one possible remedy. There is also an open proposed fix in
astral-sh/uv#19833 to have `uv lock` fall back to normal interpreter discovery instead of requiring
a CLI bypass. Let's keep the behavior and solution discussion on astral-sh/uv#19832, so I'll close
this as a duplicate.

## Classification

This is a duplicate because astral-sh/uv#19832 already tracks the same underlying failure and the
same proposed `--active` capability. Although astral-sh/uv#21009 is framed as an enhancement and is
currently labeled `enhancement`, the duplicate classification takes precedence because the open
canonical issue already contains both the bug report and this alternative remedy.

This is not a regression of a previously fixed bug. The canonical issue remains open, its direct
fix astral-sh/uv#19833 remains open, and no merged fix for this behavior was found.

## Related issues and pull requests

- astral-sh/uv#19832 (open issue), **`uv lock` errors if `.venv` is not valid**: Direct canonical
  match. It has the same reporter, command, `.venv/<version>` layout, exact error, expected fallback,
  workaround, and proposed `--active` alternative.
- astral-sh/uv#19833 (open pull request), **Added flag for whether to fail on invalid environments
  during project interpreter discovery**: Direct implementation for astral-sh/uv#19832. It changes
  only `uv lock` to fall back to global or managed interpreter discovery when the project
  environment is invalid.
- astral-sh/uv#20333 (closed pull request), **Allow fallback to interpreter discovery for non-venv
  .venv directory**: Earlier alternative implementation for astral-sh/uv#19832 with the same
  intended fallback. The author closed it without review comments.
- astral-sh/uv#11189 (merged pull request), **Add support for respecting `VIRTUAL_ENV` in project
  commands via `--active`**: Relevant history for the requested capability. It introduced
  `--active` for environment-mutating project commands but explicitly passed `active = false` in
  the `uv lock` path, confirming that lock currently does not expose active-environment selection.

## Search and supporting evidence

Literal searches covered `uv lock` with `.venv`, `--active`, and the exact error fragment
`Project virtual environment directory ... is not a valid Python environment`. Conceptual searches
covered invalid or missing project environments, interpreter discovery and fallback, active virtual
environments, `VIRTUAL_ENV`, and `UV_PROJECT_ENVIRONMENT`. Fix-oriented searches covered open,
closed, and merged pull requests and followed the cross-reference chain from astral-sh/uv#19832 to
astral-sh/uv#19833 and astral-sh/uv#20333, plus the history of `--active` in astral-sh/uv#11189.

Several plausible neighbors were inspected but are not close enough for the related list:

- astral-sh/uv#19885 has the same invalid-environment message, but concerns `uv sync` after a Docker
  build accidentally copied part of a local `.venv`; the reporter resolved it as a build-context
  problem.
- astral-sh/uv#11219 has the same message during concurrent `uv run` processes, but its trigger is a
  race while creating the project environment rather than an intentionally non-venv `.venv`.
- astral-sh/uv#17780 discusses `--active`, but the maintainer explanation concerns changing the sync
  target without changing project-root discovery, not `uv lock` failing on an invalid environment.
- astral-sh/uv#13235 concerns `uv run --no-sync` recreating an incompatible environment, not locking
  with a non-venv `.venv` directory.
