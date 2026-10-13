# pip compile selects backports-strenum with incompatible Requires-Python for Python 3.12

Issue: astral-sh/uv#22486

Classification: duplicate

## Summary

The reporter uses uv 0.12.18 (September 22, 2026) on macOS arm64 with CPython
3.12.6. Given a requirements file containing only `ai-edge-litert==2.3.0`,
`uv pip compile requirements.txt --python-version 3.12.6` selects
`backports-strenum==1.3.1`. Installation succeeds, but `uv pip check`
reports that the dependency requires Python `>=3.8.6,<3.11`.

The reporter expects a release whose declared range includes Python 3.12, or
resolution failure. They report that a `backports.strenum<1.3` constraint
selects 1.2.8 and produces a clean dependency check, including with a dedicated
cache.

Duplicate of the same-package report astral-sh/uv#15666, with ongoing discussion in astral-sh/uv#7869. astral-sh/uv#4022 and merged astral-sh/uv#4086 / astral-sh/uv#6882 document and implement the underlying upper-bound behavior.

## Draft response

This matches astral-sh/uv#15666, which covers the same ai-edge-litert/backports-strenum dependency chain. uv currently ignores dependency Requires-Python upper bounds during resolution, including with --python-version, while uv pip check validates the full declared range. That explains the discrepancy you observed. Please continue the discussion in astral-sh/uv#7869, which remains open for this behavior. You can keep the backports.strenum<1.3 constraint you verified as a workaround.

## Classification

astral-sh/uv#15666 already covers the same dependency chain and ignored Python upper bound; astral-sh/uv#7869 tracks the resulting resolve/install versus pip check discrepancy. Source and documentation confirm lower-bound-only resolution and full-specifier validation by pip check. The explicit Python 3.12 compile reproduction does not establish a separate mechanism or regression: astral-sh/uv#6882 intentionally introduced this compile behavior in 2024, and no intervening fix enforcing dependency upper bounds was identified.

Duplicate classification takes precedence because existing discussions cover
both the dependency-selection problem and the later checker diagnostic.
The documented policy explains the mechanism; it does not establish that the
selected release is compatible with Python 3.12. Closure of the historical
design issue is not evidence of a fix enforcing upper bounds.

## Related

- astral-sh/uv#15666 — `uv` installs package that is incompatible with environment python version (issue, closed). Same ai-edge-litert → backports-strenum dependency chain on Python >=3.11. Maintainers confirmed the unconditional dependency and ignored Requires-Python upper bound. The older report used uv sync on Python 3.11; this report adds pip compile on Python 3.12.
- astral-sh/uv#7869 — UV will add and install packages that are not compatible with current python version (issue, open). Tracks the same successful resolution/installation followed by a Requires-Python upper-bound error from uv pip check. Uses geojson on Python 3.12 and remains open with compatibility, needs-design, and needs-decision labels.
- astral-sh/uv#4022 — Discarding upper-bounds on `Requires-Python` (issue, closed). Canonical design discussion referenced by the matching reports. Its closure accompanied implementing upper-bound ignoring, not fixing selection of releases whose declared upper bounds exclude the target.
- astral-sh/uv#6882 — Use lower-bound semantics for all Python compatibility comparisons (pull request, merged). Merged September 2, 2024; explicitly made pip compile with an explicit Python target ignore upper bounds consistently with universal resolution. Explains why --python-version does not prevent this selection and establishes that the behavior predates uv 0.12.18.
- astral-sh/uv#4086 — Ignore upper-bounds on `Requires-Python` (pull request, merged). Merged June 6, 2024; introduced lower-bound-only dependency compatibility and closed astral-sh/uv#4022. Historical implementation of the reported mechanism, not a fix enforcing upper bounds.

## Supporting evidence

### Report and metadata

The event and supplied issue body agree. The supplied discussion includes a
maintainer comment addressing the resolution/checker distinction below.
The report separates into two observable claims:

- Compilation selects a dependency outside its declared Python range despite
  an explicit target version.
- Installation accepts the result, while `uv pip check` reports the Python
  incompatibility.

Relevant conditions are a dependency without an environment marker, an upper
bound below the selected interpreter, and an older release with a wider
declared range. The normalized names `backports-strenum` and
`backports.strenum` identify the same distribution. No cache-specific or
macOS-specific cause is established.

Read-only requests to public PyPI JSON independently confirmed:

- `https://pypi.org/pypi/backports.strenum/1.3.1/json`: both release information
  and distribution-file entries declare `>=3.8.6,<3.11`.
- `https://pypi.org/pypi/backports.strenum/1.2.8/json`: both declare
  `>=3.8.6,<4.0.0`.
- `https://pypi.org/pypi/ai-edge-litert/2.3.0/json`: `requires_dist` includes
  `backports.strenum` without an environment marker.

The installed-wheel metadata and successful workaround are reporter evidence;
the full installation was not reproduced during this triage.

### Source and documentation

- `docs/pip/compatibility.md:442` explicitly documents ignoring dependency
  `Requires-Python` upper bounds. `docs/concepts/resolution.md:136` documents
  the same policy.
- `crates/uv-resolver/src/resolver/mod.rs:1735` checks distribution Python
  compatibility through `RequiresPython::is_contained_by`.
- `crates/uv-distribution-types/src/requires_python.rs:265` implements that
  check by comparing lower bounds only.
- `crates/uv-installer/src/site_packages.rs:257` checks the installed
  interpreter against the full metadata specifier set. The diagnostic at
  `crates/uv-installer/src/site_packages.rs:763` matches the report's error
  wording.

These independent paths explain why selecting and installing the dependency
can succeed before the environment check fails.

### Maintainer discussion and timing

On October 13, 2026, charliermarsh (MEMBER) confirmed in astral-sh/uv#22486
that ignoring `Requires-Python` upper bounds is generally intended, citing
the compatibility documentation. The maintainer separately described the
subsequent `uv pip check` error as something that "feels like an inconsistency"
(comment ID 6136056755). This focuses further investigation on the checker
discrepancy; it is not a decision to enforce upper bounds during resolution
or a commitment to change the checker. The comment supplies no new
reproduction or fix and does not change the duplicate relationship to
astral-sh/uv#7869.

In astral-sh/uv#15666, a maintainer confirmed that LiteRT declared the backport
unconditionally and that adding a marker to a separate first-party dependency
does not change LiteRT's declaration. The suggested upstream correction was
to add a marker if the backport is unnecessary on newer Python versions.
That recommendation depends on LiteRT's actual runtime requirements.

The comments in astral-sh/uv#15666 point to astral-sh/uv#14711, which in turn
points to astral-sh/uv#4022. The linked merged astral-sh/uv#14752 only splits
the compatibility documentation into upper-bound and version-specifier
sections; its diff contains no resolver fix.

astral-sh/uv#5045 and astral-sh/uv#6859 led to astral-sh/uv#6882, whose
description explicitly covers ordinary `pip compile` with an explicit
Python version as well as universal compilation. The 2024 implementation,
the 2025 same-package report, and the current source all precede or support
the behavior reported for uv 0.12.18. No evidence establishes a returned,
previously fixed bug.

## Search coverage and exclusions

Searched astral-sh/uv through authenticated gh across open/closed issues and open/closed/merged PRs. Literal queries covered backports-strenum, backports.strenum, ai-edge-litert, the Requires-Python error, pip compile, --python-version, and pip check. Conceptual searches covered incompatible Python versions, ignored upper bounds, and lower-bound compatibility; fix-oriented searches followed closed reports and merged changes. Inspected bodies, comments, and timeline references, including astral-sh/uv#15666 → astral-sh/uv#14711 → astral-sh/uv#4022 and astral-sh/uv#5045 / astral-sh/uv#6859 → astral-sh/uv#6882. Inspected astral-sh/uv#14752 and confirmed it only clarified documentation. Ruled out astral-sh/uv#11624 as first-party interpreter enforcement, astral-sh/uv#17649 as build-failure/backtracking behavior, and astral-sh/uv#4959 as universal-resolution/prerelease handling.

The first-party enforcement proposal concerns the package directly being
installed and interpreter selection. It does not cover this transitive
dependency. The backtracking report involves choosing old source releases and
stopping after build failures, whereas this report resolves and installs
successfully. The universal-resolution report concerns marker forks and
prerelease dependencies; this reproduction does not use `--universal`.

PR searches returned no direct matches; relevant merged PRs were recovered
from issue comments and timelines and inspected directly. Broad search
results were used to discover candidates, not as proof that no other related
work exists.

## Maintainer next step

Investigate whether `uv pip check` should report an incompatibility for an
upper bound deliberately ignored during resolution, using the existing
reproduction and the maintainer's distinction above. Centralize that discussion
with astral-sh/uv#7869 and cross-reference the exact same-package history in
astral-sh/uv#15666. The reporter's explicit version
constraint is an available workaround. Do not describe a clean dependency
check as proof of runtime compatibility or claim that a resolver fix is
scheduled.

This handoff is based on read-only issue, PR, source, and public metadata
inspection. No builds, package installation, or test execution were needed.
Only this README was updated; no checkout files or GitHub objects were changed.
