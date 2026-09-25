# no-build and dynamic version does not build first-party package

Issue: astral-sh/uv#21982

Classification: duplicate

## Summary

astral-sh/uv#21982 reports that `uv cache clean && uv lock -P test` fails for a first-party
setuptools project when `[tool.uv] no-build = true` and the project version is dynamic. Because uv
tries to prepare backend metadata for the local project, the global build policy stops the operation
with `Building source distributions for test is disabled`.

The same underlying capability is already tracked by open issue astral-sh/uv#17416: apply
`no-build` to external dependencies without preventing the first-party project from using its build
backend. That issue's discussion explicitly includes projects whose versions are derived from VCS
metadata. The new report is a concise lock-specific reproduction of that existing limitation.

## Draft response

Thanks for the minimal reproduction. This is the same underlying limitation tracked in
astral-sh/uv#17416: `no-build` currently also blocks build-backend metadata preparation for the
first-party project, while this dynamic version requires the backend during locking.
astral-sh/uv#10622 added support for omitting dynamic project versions from the lockfile, but its
discussion notes that locking still invokes the backend. We'll use astral-sh/uv#17416 to track
separating first-party project builds from dependency builds; your setuptools-scm example is a
useful lock-specific reproduction.

## Classification

Duplicate of astral-sh/uv#17416. Both reports require the same policy distinction: keep builds of
third-party dependencies disabled while allowing the first-party project to run its backend. The
dynamic setuptools-scm version is the condition that makes the missing first-party exception visible
during `uv lock`, rather than a separate requested capability.

This is not a regression of the dynamic-lockfile work in astral-sh/uv#10622. That merged pull
request deliberately made dynamic project versions omissible from `uv.lock`, and its discussion
explicitly recorded that `uv lock` could still invoke the build backend. The open canonical issue
therefore takes precedence over a new bug or enhancement classification.

## Supporting evidence

- The reproduction combines a global build prohibition, a first-party source tree, lock-time
  metadata preparation, and a dynamic version supplied by setuptools-scm. Removing incidental
  details such as the Windows path and package name leads directly to the local-project exception
  requested by astral-sh/uv#17416.
- The current source distinguishes first-party/editable sources when building a distribution, but
  the metadata-preparation path still checks the `no-build` requirement before invoking the backend.
  This is consistent with the reported error and with the existing design discussion; it does not
  establish a Windows-specific failure.
- astral-sh/uv#12607 contains the earlier maintainer explanation that `no-build` currently covers
  local source trees too and that supporting the desired behavior requires distinguishing local
  project builds from dependency builds.
- astral-sh/uv#10622 confirms that omitting a dynamic version from the lockfile and avoiding backend
  execution are separate concerns. Its discussion calls the remaining backend invocation known
  work, so the behavior was not previously fixed and then reintroduced in uv 0.12.19.

## Related

- astral-sh/uv#17416 — Canonical open match. It requests that the project be excluded from
  `no-build` by default while external source builds remain prohibited, and its discussion includes
  the same dynamic VCS-version use case.
- astral-sh/uv#12607 — Earlier open design discussion about differentiating local source-tree builds
  from dependency builds. Its direct examples use `uv build` and `uv sync`, whereas
  astral-sh/uv#21982 demonstrates the same policy during lock-time metadata preparation.
- astral-sh/uv#10622 — Merged historical partial support for dynamic project versions. It omits
  dynamic versions from lockfiles, but the PR discussion explicitly says the backend is still
  invoked.

## Search notes

Literal searches covered `no-build`, `dynamic version`, `setuptools_scm`, the exact
`Building source distributions ... is disabled` error family, and `uv lock`. Conceptual searches
covered first-party and local source trees, static versus dynamic metadata, backend metadata
preparation, and project exclusions from build policy. Fix-oriented searches included closed issues
and merged pull requests related to static metadata and dynamic lockfiles.

astral-sh/uv#9776 and its fix astral-sh/uv#9785 were inspected but are not the canonical match: they
handle a project whose metadata is fully static, so uv can avoid backend execution entirely.
astral-sh/uv#17487 was also inspected but concerns an external dependency whose dependency metadata
requires backend execution, rather than whether the first-party project should be exempt from a
global build prohibition. astral-sh/uv#7533 and astral-sh/uv#11046 concern representation and caching
of dynamic versions in the lockfile, not the first-party `no-build` policy.
