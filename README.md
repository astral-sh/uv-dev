# Varying order of resolution-markers from the same `pyproject.toml`

Issue: astral-sh/uv#22040

Classification: bug

## Summary

The reporter observed two Renovate runs against the same project configuration and the same
Renovate image. The lock-file-maintenance run produced one order for four top-level
`resolution-markers`; a package-specific `rioxarray` update created a follow-up pull request whose
only diff moved one of those markers. The marker set and resolved packages were unchanged.

The public pull requests support the observable symptom. DigitalEarthSweden/digital-earth-sweden-community#167
was created by Renovate 44.104.2 and refreshed the lockfile. Five minutes after it merged,
DigitalEarthSweden/digital-earth-sweden-community#168 was created by the same Renovate version. Its
complete two-line diff moves the marker for Python below 3.14 on Windows ARM64 below the marker for
Python 3.14 and newer on other platforms; despite its package-update title, it changes no package
version because the project's relative `exclude-newer` setting excludes the new release.

Current source provides a strong explanation for why full and partial upgrades can differ:

- `--upgrade` marks the existing lock unusable, so resolution starts without its fork markers.
- `--upgrade-package` marks the existing lock preferable, allowing its package versions and fork
  markers to seed the next resolution.
- Merged astral-sh/uv#21000 added Python-lower-bound scheduling for those initial forks. It retains
  the input order only for forks whose Python bounds tie.
- Lock serialization simplifies and deduplicates the resulting markers but does not apply an
  independent canonical sort.

This matches the shape of the reported reorder and makes astral-sh/uv#21000 the likely regression
point. The exact uv version and command lines used inside Renovate remain unconfirmed, so a direct
reproduction is still needed before treating the mechanism as proven.

## Draft response

Thanks for the paired Renovate pull requests. The second diff confirms an ordering-only change: one
`resolution-markers` entry moves, with no resolved package change.

uv currently handles these paths differently: a full upgrade resolves without the existing
lockfile, while a package-only upgrade can reuse its fork markers; astral-sh/uv#21000 added
Python-bound scheduling for those reused initial forks. That makes astral-sh/uv#21000 a likely
regression point, though we still need the exact invocation to confirm it.

Could you provide the `uv --version` output and the uv command lines from Renovate's debug log?
Alternatively, please confirm whether running `uv lock --upgrade` followed by
`uv lock --upgrade-package rioxarray` from a clean checkout reproduces the one-line reorder. That
would give us a focused regression case.

## Classification

This is a bug. The package-specific operation creates lockfile churn while preserving the exact
same set of resolution markers and package versions. A lockfile should have stable serialization
for semantically identical resolution output, even when full and package-specific upgrades reach
that output through different resolver paths.

It is not a duplicate. No existing issue found in the literal, conceptual, and fix-oriented
searches tracks this exact ordering-only behavior. The closest open issue, astral-sh/uv#17747,
also involves package-specific upgrades and unexpected `resolution-markers` changes, but its
observable failure is that a freshly written lockfile is immediately considered stale. Its linked
candidate fix, astral-sh/uv#17752, changes marker-tree canonicalization rather than initial-fork
scheduling.

## Related

- astral-sh/uv#21000 (merged pull request, "Respect `fork-strategy` for `environments`") is the
  strongest historical link and likely regression point. It began sorting initial forks from an
  existing lockfile by their lower Python bounds. Its documentation explicitly says tied forks keep
  their supplied order, and its tests changed serialized marker order along the Python axis.
- astral-sh/uv#20999 (closed issue, "`fork-strategy` has no effect on the forks that come from
  `[tool.uv] environments`") prompted astral-sh/uv#21000. It establishes the intended reason for
  sorting initial forks: selected versions needed to respect `fork-strategy`. It did not discuss
  stable marker ordering across full and partial upgrade modes.
- astral-sh/uv#17747 (open issue, "`uv lock` not working as expected in some cases when
  `upgrade_package` is set") is the closest open adjacent report. A package-specific upgrade also
  changes resolution markers unexpectedly, but the lock remains perpetually stale and the linked
  work points to a different marker-canonicalization problem. Discussion should not be centralized
  there without evidence that the mechanisms are the same.

## Search coverage and ruled-out candidates

Literal searches covered `resolution-markers`, marker order/sorting/reordering,
`--upgrade-package`, and lockfile changes without version or package changes. Conceptual searches
covered deterministic and stable lockfiles, initial-fork ordering, `fork-strategy`, marker
canonicalization, fork preferences, and full-versus-partial resolution. Fix-oriented review covered
merged pull requests and closed issues around initial-fork scheduling and marker serialization.

The following plausible results were inspected but ruled out as the same issue:

- astral-sh/uv#10988 reported inconsistent lockfiles, but the reporter confirmed that different uv
  versions produced materially different marker expressions.
- astral-sh/uv#9296 reported duplicate entries in `resolution-markers`, not different ordering of
  the same entries.
- astral-sh/uv#16839 reported a false "Lockfile changes detected" message under `--dry-run`, with no
  written lockfile diff.
- astral-sh/uv#17752 is an open candidate fix for astral-sh/uv#17747's compound-disjunction
  canonicalization failure, not for initial-fork ordering.
