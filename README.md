# Panic (`OperatorLocalCombo`) when `python_full_version not in '<version>+local'` is combined with an upper bound

Issue: astral-sh/uv#22804

Classification: bug

## Summary

No duplicate found. astral-sh/uv#6172 provides the containment-marker implementation history; astral-sh/uv#21310 and astral-sh/uv#21311 discuss adjacent parsing limitations, not this panic.

The reporter describes an offline process abort when a dependency marker combines
`python_full_version not in '3.20+x'` with `python_full_version < '4'`.
The panic is `VersionSpecifierBuildError { kind: OperatorLocalCombo { operator: LessThan, version: "3.20+x" } }`
from an unwrapped result in `crates/uv-pep440/src/version_specifier.rs:629`.
Reported exit status is 134 (SIGABRT).

The report names uv 0.13.4 and 0.12.24, Linux x86_64, and Python 3.14. This is a synthetic
marker-handling example; the reporter does not claim an affected real project. The issue has
no comments at inspection time. Static inspection supports the failure mechanism; the CLI
examples were not executed during this investigation.

## Report and triggering conditions

The minimal project supplied by the reporter is:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = ["a ; python_full_version not in '3.20+x' and python_full_version < '4'"]
```

The observable claims were separated before searching:

- `uv lock --offline` aborts for this project, without needing the dependency to exist.
- `uv pip compile --universal --no-index` reportedly aborts with the same requirement in
  `requirements.in`.
- `uv export --frozen` reportedly aborts when a lockfile dependency edge has
  `python_full_version not in '3.20+x'` and the lockfile's `resolution-markers` contains
  `python_full_version < '4'`. The report describes this lockfile shape but does not attach
  a complete export fixture.
- The reporter says removing the upper bound, using `!=` instead of `not in`, or removing
  the local suffix avoids the panic.
- Expected behavior is a controlled error or non-panicking marker handling. There is no request
  to expand local-version or containment syntax support.

Search dimensions included the panic/abort symptom; the three command paths; PEP 508 marker
parsing, range intersection and rendering; the conjunction of a local-version exclusion and
upper bound; and the exact error and function identifiers. The reported explanation was
checked separately from searches for observable failures.

## Draft response

Your example exposes a bug in marker handling. The current source retains the local suffix for `not in` ranges, then unwraps an error when rendering an ordered comparison against that local version. Version-field containment has deliberately limited support, as discussed in astral-sh/uv#21311, but it should not abort the process. The next step is regression coverage for your bounded exclusion and a change that handles or rejects the local-version bound without panicking, including the reported compile and export paths.

## Classification

The reported process abort is incorrect behavior, and the source confirms an unchecked conversion from a local-version range bound to a forbidden ordered PEP 440 specifier. Limited or deprecated containment syntax does not justify a panic. No inspected issue or PR tracks this same failure, and no prior fix or regression was established.

The related containment discussions establish a compatibility boundary, not permission to
abort on input. No inspected historical fix addresses this combination, and the report also
names an older affected release. Therefore this is neither a duplicate of the containment
support request nor an established recurrence of a previously fixed bug.

## Related

- astral-sh/uv#6172 — Add support for `python_version in ...` markers (pull request, merged). Introduced version-field containment support with deliberately limited, whitespace-separated exact-version semantics. Its discussion explicitly covers python_full_version; it establishes the relevant implementation history but does not address local-version exclusions combined with bounds.
- astral-sh/uv#21310 — uv-pep508 evaluates python_version in a,b,c as always true (issue, closed). Nearby containment-marker correctness report. Comma-separated versions were ignored and evaluated as true; the discussion declined broader support because version-field containment is deprecated. It reports neither local-version bounds nor a panic.
- astral-sh/uv#21311 — fix: Allow commas as python_version string separators (pull request, open). Proposes a parser change for astral-sh/uv#21310. Maintainer comments explain why containment support should not be expanded. It remains unmerged and concerns separators, not the local-version range-rendering panic, so it is not a duplicate or a fix for this report.

## Supporting source evidence

Inspected checkout: `1765e51d67f43299d8fbd37b302b773a80c8499d`.

- `crates/uv-pep508/src/marker/parse.rs:393`: `parse_version_in_expr` parses whitespace-separated
  values using `Version::from_str` and retains the parsed versions, including local segments.
- `crates/uv-pep508/src/marker/algebra.rs:249`: `VersionIn` expressions for
  `PythonFullVersion` use `Edges::from_versions`.
- `crates/uv-pep508/src/marker/algebra.rs:1692`: `from_versions` constructs singleton
  ranges without stripping suffixes and complements the range for `NotIn`.
  By contrast, ordinary comparison specifiers go through `only_release()` at line 1659.
- `crates/uv-pep508/src/marker/simplify.rs:264`: `range_inequality` only renders an
  exclusion as `!=` when the overall bounding range is unbounded on both sides.
  Adding `< '4'` prevents that shortcut.
- `crates/uv-pep508/src/marker/simplify.rs:94`: remaining version ranges are rendered
  through `VersionSpecifier::from_release_only_bounds`. The local-version exclusion leaves
  an excluded bound at `3.20+x`.
- `crates/uv-pep440/src/version_specifier.rs:448`: `from_version` rejects a local version
  combined with an incompatible operator using `OperatorLocalCombo`.
  `from_upper_bound` at line 623 constructs `LessThan` for an excluded bound and unwraps
  that rejected result at line 629.

These source facts explain the reported difference between `not in` and `!=`, and why the
upper bound matters. They establish the unsafe error handling without independently confirming
the reported exit status or every CLI path.

Existing tests in `crates/uv-pep508/src/marker/tree.rs` cover version containment evaluation
and simplification, including unbounded exclusions. A targeted search in the marker crates
and CLI integration tests found no local-suffix containment case. No tests or code were changed.

## Search scope and exclusions

Used authenticated gh to search open/closed issues and open/closed/merged PRs for OperatorLocalCombo, VersionSpecifierBuildError, from_release_only_bounds, version_specifier.rs, python_full_version/not in, marker panic/crash/simplification, local versions/segments, and version bounds. Searched lock --offline, universal pip compile, and frozen export panic paths separately, without package/version/platform restrictions. PR keyword searches omitted known historical PRs; supplemented them with the 1,500 most recent PRs across all states and direct inspection of linked issues, PRs, and comments. Ruled out astral-sh/uv#1855 and astral-sh/uv#2430 (package-version equality), astral-sh/uv#2478 (local package range support), astral-sh/uv#16406 (missing export root), astral-sh/uv#22034 (trace logging of false constraints), and astral-sh/uv#22727 (invalid --python-version request). Also inspected astral-sh/uv#22651's recent marker-rendering rewrite; no evidence establishes this panic as a regression of a previously fixed bug.

Additional inspected candidates were excluded from the related list:

- astral-sh/uv#3683 is the completed request implemented by astral-sh/uv#6172; it did not report
  a panic.
- astral-sh/uv#21309 concerns reversed containment evaluating as true and was closed as
  unsupported syntax; it has no local bound or abort.
- astral-sh/uv#3917 discusses `platform_release` numeric versus lexical evaluation.
- astral-sh/uv#8381 panics with a missing resolution-graph entry when sources vary by platform.
- astral-sh/uv#22259 concerns opaque Darwin release values and logical negation, not local
  Python versions or specifier-construction errors.
- astral-sh/uv#20818 concerns prerelease package selection from constraints.
- astral-sh/uv#22049 fixes post-release matching for package version bounds.
- astral-sh/uv#22215 concerns hashing internal bounds that compare equal.

PR keyword search was incomplete in practice: even a search for known astral-sh/uv#6172
returned no result. Recent PR listing and reference-chain inspection supplemented that gap.
A small set of search calls also hit a transient API rate limit; subsequent searches succeeded.
The absence of a duplicate is a finding within this search scope, not a claim of exhaustive
coverage of every historical PR.

## Suggested next steps

Reproduce the offline lock example and add focused regression coverage in the relevant existing
integration modules. Exercise universal compilation and frozen export where they reach distinct
rendering paths; construct a complete lockfile fixture for the export case. Check the existing
marker tests before adding overlapping coverage and apply any required test feature gates.

Choose a controlled rejection or normalization strategy for local versions in containment
ranges, consistent with the deliberately limited support discussed in astral-sh/uv#21311.
Avoid creating an ordered local-version specifier and unwrapping its error. The report does not
establish that replacing `not in` with `!=` preserves all intended semantics.

This handoff records source inspection and GitHub research only. No runtime reproduction was
attempted, no checkout files were edited, and no GitHub changes were made.
