# uv sync accesses unavailable files even when told not to

Issue: astral-sh/uv#21972

Classification: duplicate

## Summary

The report shows `uv sync --locked --no-group lab` failing because a path archive used only by the
excluded `lab` dependency group is unavailable on the syncing machine. Replacing that package's
source with `--no-sources-package` instead makes resolution fail against the registry, and supplying
dependency metadata leads to a missing-hash error.

This matches the open discussion in astral-sh/uv#11675: dependency-group and extra selectors choose
what is installed during sync, but lockfile resolution still considers all declared groups and their
sources. The use of `--locked` is significant. Current documentation defines it as validating that
the lockfile would remain unchanged after resolution, while `--frozen` uses the existing lockfile
without checking whether it is up to date. astral-sh/uv#11573 records the same locked-versus-frozen
distinction for an unavailable platform-specific path wheel.

A maintainer has now asked the reporter to try `uv sync --frozen --no-group lab`, explicitly
confirming that `--locked` must access the files to validate the lockfile while `--frozen` trusts
the lockfile without verification. The reporter has not yet confirmed whether the suggested command
succeeds in this reproduction.

## Draft response

Thanks for the report. This matches astral-sh/uv#11675. `--no-group lab` excludes that group from
the environment sync, but it does not exclude the group from the resolution used to validate the
lockfile. Since `--locked` asserts that the lockfile would remain unchanged after a resolution, uv
still needs to inspect the declared path source. `--no-sources-package smaract-ctl` changes the
source used for that resolution; it does not remove the `lab` group, which is why uv then looks for
the package in the registry.

When the lockfile was generated on a machine where the archive is available, use
`uv sync --frozen --no-group lab --no-progress` on a machine where it is unavailable. `--frozen`
uses the existing lockfile without validating it through a new resolution, so a
`tool.uv.dependency-metadata` entry should not be needed for this workflow. Let's keep the broader
discussion in astral-sh/uv#11675.

## Classification

Duplicate of astral-sh/uv#11675. Both reports concern `uv sync` evaluating an unavailable path
source attached only to a group or extra that was not selected for installation. The newer report
adds the important `--locked` condition and a path archive on Windows, but it does not establish a
separate regression: current command documentation and the maintainer explanation in
astral-sh/uv#11573 confirm that locked mode performs resolution to validate the lockfile. The
appropriate existing-lockfile mode for unavailable sources is `--frozen`.

## Related

- astral-sh/uv#11675 (open issue) — Closest canonical match. It reports `uv sync --only-group` and
  `--no-extra` evaluating a path source belonging to a non-selected dependency group or optional
  dependency. Maintainer comments explain that all packages and groups are resolved even when they
  are excluded from the sync operation, and the thread identifies `--frozen` as the way to skip
  that resolution when an existing lockfile can be trusted.
- astral-sh/uv#11573 (closed issue) — Exact precedent for the `--locked` triggering condition. A
  platform-specific local wheel absent on the current platform was still inspected under locked
  sync. Maintainers confirmed that locked mode must resolve to validate the lockfile and recommended
  frozen mode instead. It differs only in using an optional platform wheel rather than an excluded
  dependency-group archive.

## Supporting evidence

The current locking documentation says that locking resolves the project's dependencies, whereas
syncing installs a subset from the lockfile. It defines `--locked` as checking whether the lockfile
is up to date and `--frozen` as using it without that check. The current CLI description is more
explicit: locked mode asserts that `uv.lock` would remain unchanged after a resolution. The group
documentation describes `--no-group` in terms of packages not being installed, consistent with the
maintainer explanations in the related issues.

The maintainer comment on astral-sh/uv#21972 directly confirms this interpretation and proposes
`uv sync --frozen --no-group lab` as the next diagnostic step. This is a source-backed workaround,
but its success remains unverified until the reporter responds.

Literal searches covered the exact `Distribution not found at`, `Failed to generate package
metadata`, `--no-group`, `--no-sources-package`, dependency-metadata/hash, path-source, and locked
errors. Conceptual searches covered excluded or optional groups, unavailable private/path sources,
cross-platform local distributions, lockfile validation, frozen mode, and resolving all groups.
Open and closed issues and open, closed, and merged pull requests were searched; no pull request
represented a closer fix or active implementation.

astral-sh/uv#9804 was also inspected and reproduces a missing local path dependency excluded with
`--no-dev`, with the same frozen-mode answer, but it is redundant with the more exact items above.
astral-sh/uv#11104 concerns excluding an inaccessible private-index group from resolution and is
conceptually related, but its source type and requested capability are broader. astral-sh/uv#13487
concerns cross-platform path-source inspection while updating a lockfile, rather than group
selection. astral-sh/uv#21429 was ruled out because its missing path remains a selected direct
dependency.
