# uv sync fails on missing dependency which cannot be removed

Issue: astral-sh/uv#21429

Classification: bug

## Summary

The reporter deleted a local package directory in a monorepo, then removed that package's
dependency declaration from `pyproject.toml`. With uv 0.11.28, they report that the stale local
package remains in `uv.lock`, `uv sync` or locking cannot recover because the source directory is
missing, and `uv remove` cannot remove it. They currently recover only by manually editing generated
files. They also propose doctor-style diagnostics and more forgiving recovery in `uv remove` and
`uv lock`.

The reported failure could not be produced from that sequence alone. On Linux x86_64 with CPython
3.12.3, both the reported uv 0.11.28 and current uv 0.12.9 refreshed `uv.lock` successfully during an
ordinary `uv sync` after the dependency declaration and local directory were removed. Locked and
frozen syncs did fail, but for their documented stale-lock behavior. Because the report omits its
workspace manifests, exact command and error, and locked/frozen configuration, there is not enough
information to determine which path the reporter encountered.

No existing issue or pull request was found that tracks this same sequence with an ordinary lock
refresh. The closest discussions retain a live workspace or path declaration, use frozen or locked
operation, or leave a matched directory without a `pyproject.toml`.

## Current status

A maintainer has indicated that the issue does not yet contain enough information to reproduce or
diagnose the failure. The issue is awaiting a minimal reproducible example containing the uv
version, operating system, exact command, and its complete `--verbose` output. The project pointed
the reporter to its reproducible-example guidance and the reporting requirements in
astral-sh/uv#9452. This confirms that further investigation is blocked on reporter-supplied details;
it does not establish a root cause or change the bug classification.

## Reproduction

Outcome: **needs more information**.

The minimal fixture began with a root project containing:

```toml
[project]
name = "root"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["leaf"]

[tool.uv]
package = false

[tool.uv.sources]
leaf = { workspace = true }

[tool.uv.workspace]
members = ["packages/*"]
```

`packages/leaf/pyproject.toml` declared a `leaf` project. After `uv lock` created a lock containing
`leaf` with `source = { editable = "packages/leaf" }`, the `leaf` dependency was changed to
`dependencies = []` and the entire `packages/leaf` directory was deleted. The following commands
were then run with temporary caches and with ambient `UV_LOCKED` and `UV_FROZEN` unset:

```console
uv remove leaf
uv sync --locked
uv sync --frozen
uv sync
```

The results were the same in uv 0.11.28, the version reported in astral-sh/uv#21429, and uv 0.12.9:

- `uv remove leaf` failed with `The dependency leaf could not be found in project.dependencies`.
  This confirms that `uv remove` does not delete a lock-only entry after the requirement has already
  been removed from project metadata.
- `uv sync --locked` resolved the remaining one-package project, then refused to change the stale
  lockfile because `--locked` was provided.
- `uv sync --frozen` reused the stale lockfile and failed because the local `leaf` distribution no
  longer existed.
- Ordinary `uv sync` resolved one package successfully and rewrote `uv.lock` without `leaf`. With uv
  0.12.9, where the initial fixture had also been synced, it additionally uninstalled the editable
  `leaf` package from `.venv`.

Two evidence-backed variants on uv 0.12.9 also recovered with ordinary `uv sync`: an explicit
`members = ["packages/leaf"]` workspace declaration and a non-workspace
`leaf = { path = "../leaf", editable = true }` source. Both locked and frozen modes failed as above.

Relevant existing integration coverage is in `crates/uv/tests/lock/lock.rs`:

- `lock_remove_member` verifies that `uv lock --locked` rejects stale workspace state and an
  ordinary `uv lock` removes a workspace member and its transitive packages after the live
  dependency and membership declarations are removed.
- `lock_remove_member_non_project` verifies the equivalent pruning for a virtual workspace root.
- `lock_unused_external_workspace_source` verifies that an unused external workspace source whose
  directory is missing does not prevent an ordinary lock.

The tests do not cover the reporter's exact sequence of deleting the directory before the ordinary
sync. The direct command-line reproduction above covers that sequence and succeeds. To reproduce
the claimed ordinary-sync failure, maintainers still need all root and member `pyproject.toml`
sections before and after deletion (including dependency groups, optional dependencies,
`tool.uv.sources`, and `tool.uv.workspace`), the directory layout, the exact command and complete
`--verbose` output, any `uv.toml`, and whether `UV_LOCKED`, `UV_FROZEN`, `--locked`, or `--frozen`
is in effect. The reported macOS 26 Intel platform may also matter if the same complete fixture
succeeds on Linux.

## Draft response

Thanks for the report. After a dependency has been removed from the project metadata—including any
applicable dependency-group, optional-dependency, `tool.uv.sources`, and
`tool.uv.workspace.members` entries—an unfrozen `uv lock` or `uv sync` should refresh `uv.lock` and
remove the stale local package. `uv remove` only removes dependencies that are still declared in
project metadata; it does not directly edit lock-only records.

I could not reproduce an ordinary-sync failure with either uv 0.11.28 or uv 0.12.9. In a minimal
workspace, after removing the dependency declaration and deleting its directory, ordinary `uv sync`
rewrote `uv.lock` without the local package. `uv remove` reported that the dependency was already
absent from `project.dependencies`; `uv sync --locked` rejected the stale lockfile, and
`uv sync --frozen` failed because it intentionally reused the stale local source.

Could you provide the directory layout and all relevant root/member `pyproject.toml` sections before
and after deletion, the exact command and complete error output, any `uv.toml`, and whether
`UV_FROZEN`, `UV_LOCKED`, `--frozen`, or `--locked` is set? Please also say whether an ordinary
`uv sync` with those modes disabled still fails. That will distinguish a live metadata reference or
lock-preserving mode from a lock regeneration failure.

## Classification

This remains classified as a bug report because the central claim is a correctness failure: uv
allegedly cannot regenerate its derived lockfile after the dependency was removed from source
project metadata, leaving no supported recovery path. That failure is not confirmed: ordinary sync
recovered in the tested fixtures, while locked and frozen failures were expected. The request for a
doctor command and broader advice is an enhancement component, but it is secondary to the reported
recovery failure. Missing configuration prevents determining whether the report involves a live
metadata reference, lock-preserving mode, or a distinct bug. No existing issue or pull request is
close enough to centralize this report, so it is not a duplicate.

## Related

- astral-sh/uv#13670 — “Feature Request: Optional Workspaces” (open issue). This is the closest
  missing-local-workspace request: it asks uv to tolerate an absent optional workspace member and
  fall back to another source. It differs because that issue intentionally keeps the dependency and
  workspace declaration, whereas astral-sh/uv#21429 says the dependency declaration was removed and
  asks the lockfile to discard stale derived state.
- astral-sh/uv#7055 — “Ignore path dependency's workspace with `uv lock`” (open issue). It also
  reports `uv lock` failing because a local directory dependency refers to a missing workspace
  package. Its missing `codegen` member remains declared inside the vendored dependency, so it does
  not cover failure after all references to the deleted package have been removed.
- astral-sh/uv#6685 — “`--no-install-workspace` needs the pyproject.toml of each workspace member to
  be present” (closed issue). This established that missing member metadata used to block a
  workspace sync when only the root manifest and lockfile were present. Its Docker-layering case
  deliberately retained all workspace members and used lock-preserving modes, unlike the requested
  stale-entry removal here.
- astral-sh/uv#6737 — “Do not require workspace members to sync with `--frozen`” (merged pull
  request). This fixed astral-sh/uv#6685 specifically for frozen sync, where the existing lockfile is
  authoritative. It does not address a normal relock that should remove a deleted dependency.

## Search evidence

Authenticated GitHub searches covered open and closed issues and open, closed, and merged pull
requests. Literal queries included “missing workspace member,” “missing local dependency,” “stale
lockfile dependency,” “deleted workspace package,” and “remove broken dependency,” with separate
queries for `uv sync`, `uv lock`, and `uv remove`. Conceptual queries covered absent workspace
members, path dependencies whose source directories no longer exist, optional workspaces, stale
derived lock state, and recovery from broken environments. Fix-oriented review covered merged
changes for missing-member frozen sync and workspace-member discovery, with attention to changes
after uv 0.11.28.

astral-sh/uv#17196 and its merged changes astral-sh/uv#17901 and astral-sh/uv#18051 were inspected but
ruled out as close matches: they handle directories selected by workspace globs that still exist but
contain no `pyproject.toml` (often only gitignored files), while this report says the package
directory itself was deleted. astral-sh/uv#12661 and astral-sh/uv#16407 were also ruled out because
they concern stale workspace identity or error reporting under `--frozen`, rather than pruning a
removed local dependency during an ordinary lock refresh. Environment-repair issues such as
astral-sh/uv#16468 and astral-sh/uv#19412 involve damaged installed package files in `.venv`, not a
missing local source recorded in `uv.lock`.
