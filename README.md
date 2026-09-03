# `exclude-newer` behavior with pull-through PyPI caches missing upload timestamps & locking build backends

Issue: astral-sh/uv#21449

Classification: duplicate

## Summary

The report combines two concerns from a frozen CI workflow:

- `UV_FROZEN=1` keeps application resolution on the checked-in `uv.lock`, but installing or building
  the local project still creates an isolated PEP 517 environment and resolves
  `build-system.requires` such as `hatchling`.
- The configured proxpi pull-through index does not expose PEP 700 `upload-time` metadata. With
  `UV_EXCLUDE_NEWER=P2D`, uv therefore treats every candidate lacking an upload time as unavailable
  and cannot resolve the isolated build environment.

The request to lock build requirements and their transitive closure in `uv.lock` is already tracked
by astral-sh/uv#5190. For `uv build` specifically, astral-sh/uv#18894 tracks generating a complete,
hashed build-constraints input. Missing upload times are an established index-metadata limitation,
not evidence of a new resolver defect: uv cannot evaluate an upload-time cutoff without the
timestamp. Package-scoped and index-scoped opt-outs were added by astral-sh/uv#16854 and
astral-sh/uv#18839.

## Draft response

There are two separate pieces here. `--frozen` tells project commands to use `uv.lock` without
checking whether it is current; it does not make PEP 517 isolated build environments part of that
lock. Fully locking build requirements and their transitive dependencies is tracked in
astral-sh/uv#5190. If this is specifically `uv build`, automatic generation of the complete hashed
constraints input is tracked in astral-sh/uv#18894; today the reproducible path is a fully pinned,
hashed constraints file with `uv build --build-constraint constraints.txt --require-hashes`. For project
syncs, `build-constraint-dependencies` can constrain backend versions, but it still requires
resolution and is not a complete build-dependency lock.

`exclude-newer` cannot determine whether a file predates the cutoff when the index omits the PEP 700
`upload-time`, so treating that file as unavailable is intentional. With uv 0.12.9, you can set
`exclude-newer = false` on the configured proxpi `[[tool.uv.index]]`, as implemented by
astral-sh/uv#18839, or use
`[tool.uv] exclude-newer-package = { hatchling = false }`, as implemented by astral-sh/uv#16854. A
package override must also cover any transitive build requirements whose timestamps are missing.
There is no mode that can both enforce a timestamp cutoff and accept artifacts whose timestamps are
unknown.

Since the remaining full-locking capability is already tracked in astral-sh/uv#5190, we can continue
that discussion there.

## Classification

This is a duplicate because the requested zero-resolution CI capability depends on putting isolated
build requirements, including transitives and hashes, into `uv.lock`. That is the same capability
tracked by the open canonical enhancement astral-sh/uv#5190; maintainers also redirected
astral-sh/uv#12446 and astral-sh/uv#13416 there.

The reported `exclude-newer` result does not establish a bug. Repository documentation explicitly
requires an index to provide PEP 700 `upload-time`; otherwise the artifact is unavailable unless its
package or index is opted out. The warning reports the user-provided cutoff, rather than establishing
that uv assigned that cutoff as a synthetic upload time. `--frozen` skips lockfile freshness checks,
but it does not promise that isolated build requirements are already locked.

Duplicate takes precedence over `question` because a central open issue already tracks the principal
missing capability. The cache portion has a direct supported answer in current uv.

## Related

- astral-sh/uv#5190 — **Locking of build dependencies** (open issue). This is the canonical request
  for `uv.lock` to include isolated build requirements and prevent unlocked artifact downloads during
  installation.
- astral-sh/uv#18894 — **Generate requirements file for `uv build`'s build constraints argument**
  (open issue). This is the closest first-party `uv build` variant and requests automatic generation
  of the complete hashed build-requirements input now maintained manually.
- astral-sh/uv#12449 — **exclude-newer should have an extra flag to allow it to skip over packages
  that don't have publish date information.** (open issue). This tracks fallback behavior for absent
  PEP 700 metadata. Maintainer comments explain the reproducibility tradeoff and point to scoped
  opt-outs.
- astral-sh/uv#16854 — **Allow disabling `exclude-newer` per package** (merged pull request). This
  implemented `exclude-newer-package = { hatchling = false }` for timestamp-less package sources.
- astral-sh/uv#18839 — **Add `exclude-newer` to `[[tool.uv.index]]`** (merged pull request). This
  implemented `exclude-newer = false` for a configured index, directly covering a pull-through proxy
  that cannot supply upload timestamps.

## Supporting evidence

- astral-sh/uv#10394 established that uv needs the Simple API's PEP 700 `upload-time` field to decide
  which files fall before a cutoff; a timestamp visible only in an index's web UI is insufficient.
- astral-sh/uv#16813 and astral-sh/uv#16846 requested scoped handling for registries or packages that
  omit upload times. They were addressed by astral-sh/uv#16854 and astral-sh/uv#18839.
- Current project documentation describes `--frozen` as using the lockfile without checking whether
  it is up to date, not as disabling all build-environment resolution.
- Current build documentation recommends `uv build --build-constraint constraints.txt --require-hashes` to
  constrain build requirements to pinned versions and known hashes.
- `build-constraint-dependencies` is recorded in `uv.lock` and reused by project commands, but it is
  a set of constraints, not a locked build dependency graph. It does not eliminate isolated
  resolution or automatically enumerate and hash the backend's transitive dependencies.

## Search coverage

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `missing an upload date`, `filtered by exclude-newer`, `build-system.requires`,
`UV_FROZEN`, and `uv.lock`. Conceptual searches separately covered PEP 700 upload metadata, private
mirrors and pull-through caches, package- and index-specific cooldown bypasses, build isolation,
build constraints, deterministic builds, and locked build backends. Comments and reference chains
from astral-sh/uv#10394, astral-sh/uv#16813, astral-sh/uv#16846, astral-sh/uv#12446, and
astral-sh/uv#13416 were inspected to locate the canonical discussions and implementations.

astral-sh/uv#12476 was inspected but is only adjacent: it proposes deriving an sdist build cutoff
from release dates, rather than locking the current project's build backend or handling a proxy that
omits timestamps.
