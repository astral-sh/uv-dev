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

No existing issue or pull request was found that tracks this same sequence: the local dependency is
removed from project metadata, its directory is gone, and a normal lock refresh still cannot prune
the stale lock entry. The closest discussions all retain a workspace or path declaration, use frozen
or locked operation, or leave a matched directory without a `pyproject.toml`.

Repository evidence makes the missing details important. Current integration coverage shows that an
unused external workspace source whose directory does not exist does not prevent `uv lock` from
succeeding. The lockfile is derived state, so after every live dependency reference has been removed
from the project metadata, an unfrozen `uv lock` or `uv sync` should regenerate it without the stale
package. Conversely, `uv remove` removes requirements declared in project metadata; it is not a
command for directly deleting a lock-only record. The report does not include the relevant
`pyproject.toml` sections, the exact commands or errors, or whether locked/frozen mode is active, so
the precise failing path and root cause are not established.

## Draft response

Thanks for the report. After a dependency has been removed from the project metadata—including any
applicable dependency-group, optional-dependency, `tool.uv.sources`, and
`tool.uv.workspace.members` entries—an unfrozen `uv lock` or `uv sync` should refresh `uv.lock` and
remove the stale local package. `uv remove` only removes dependencies that are still declared in
project metadata; it does not directly edit lock-only records.

Could you retry with the current release, uv 0.12.9, and provide a minimal reproduction if it still
fails? Please include the directory layout and relevant `pyproject.toml` files before and after the
deletion, the exact commands and complete error output, and whether `UV_FROZEN`, `UV_LOCKED`, or
equivalent command-line flags are set. That will show whether a live workspace/source reference
remains or whether lock regeneration itself is failing.

## Classification

This is a bug report because the central claim is a correctness failure: uv allegedly cannot
regenerate its derived lockfile after the dependency was removed from the source project metadata,
leaving no supported recovery path. If reproduced with all live references removed and without
frozen or locked mode, that is contrary to the current locking model and existing coverage for
unused missing workspace sources. The request for a doctor command and broader advice is an
enhancement component, but it is secondary to the reported recovery failure. The missing
reproduction prevents confirmation of the mechanism, not classification of the reported behavior.
No existing issue or pull request is close enough to centralize this report, so it is not a
duplicate.

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
