# LenientRequirement missing-dot fixup rewrites requests==2.32.3* to requests==2.3.*

Issue: astral-sh/uv#23406

Classification: bug

## Summary

On uv 0.13.10 and the same revision on `main`, a malformed wildcard in `[project].dependencies` is repaired to the wrong release prefix. The report shows `uv pip compile pyproject.toml` resolving `requests==2.3.0` from `requests==2.32.3*` and reports the same selection in `uv lock` and `uv sync`. The diagnostic is visible with `-v`, not at default verbosity. The closest history is the issue that motivated accepting missing-dot wildcards, the PR that introduced that repair, and the later move of its warning to tracing. None addressed the digit loss.

## Draft response

You've identified a bug in uv 0.13.10: the missing-dot repair drops digits from `requests==2.32.3*` and passes `requests==2.3.*` to the resolver. The warning is emitted through tracing, so it isn't shown by default. For now, use `requests==2.32.3.*` for a prefix match or `requests==2.32.3` for an exact pin. The repair should retain the full prefix, with a regression test for multi-digit version segments.

## Classification

This is a bug: although the input is malformed, the accepted repair changes the release prefix instead of preserving it or rejecting the input. The reported truncation follows from the same capture and replacement introduced in astral-sh/uv#416. No earlier fix or existing tracker for this truncation was found, so this is neither a known regression of the version rewrite nor a duplicate. The warning's visibility has separate history and does not explain or justify the wrong result.

## Related

- astral-sh/uv#408 (closed) — “Accept more invalid version specifiers” requested recovery for PyPI metadata such as `!=3.0*`, `!=3.1*`, and `!=3.2*`, as well as trailing commas. It concerned parse failures on short inputs, not a successful rewrite that drops digits.
- astral-sh/uv#416 (merged) — “Add missing-dot fix to lenient requirements” introduced the same repeated capture and replacement used by the parser at the reported revision. Its test checks the short exclusions from astral-sh/uv#408 and cannot expose truncation in a multi-digit release prefix.
- astral-sh/uv#775 (merged) — “Filter out files with invalid requires python specifiers” switched shared fixup diagnostics from a user-facing warning to `tracing::warn!`. That explains the separate visibility symptom; the PR did not change the missing-dot regex or address the wrong release prefix.

## Evidence

- The reported `main` commit, `e3e7c1eacb980339352a5ad69affcd3b6c206c92`, is also the uv 0.13.10 version bump. The release was published November 24, 2026.
- Static project metadata passes `project.dependencies` through `LenientRequirement` in `crates/uv-pypi-types/src/metadata/metadata_resolver.rs:229`. In `crates/uv-pypi-types/src/lenient_requirement.rs:32`, `(\d\.\d)+\*` matches the whole `2.32.3*` as two repetitions of `2.3`, and `${1}.*` keeps the last capture. The fallback accepts the result once it parses (`crates/uv-pypi-types/src/lenient_requirement.rs:70`). For `2.2.0*`, the match covers only `2.0*`, leaving the leading `2.` intact. Both substitutions were checked in isolation; the CLI command outcomes are from the reporter.
- `LenientVersionSpecifiers` shares the fixup table. Both existing missing-dot tests use the shorter `pyzmq`-style `!=3.0*` inputs (`crates/uv-pypi-types/src/lenient_requirement.rs:214` and `crates/uv-pypi-types/src/lenient_requirement.rs:279`).
- The repair emits `tracing::warn!`; the CLI defaults to tracing off unless configured otherwise and enables uv tracing with `-v` (`crates/uv/src/lib.rs:194`, `crates/uv/src/logging.rs:39`).

## Search scope

Searched open and closed issues and open, closed, and merged PRs with authenticated `gh`. Literal queries used the input and output versions, `LenientRequirement`, `LenientVersionSpecifiers`, and “Fixing invalid requirement” / “inserting missing dot.” Conceptual queries covered wrong-version selection, wildcard prefix matching, malformed specifiers and metadata, the project commands, and verbose-only warnings. Fix-oriented checks included PR listings, linked discussions, merged diffs, and current and historical parser paths.

Ruled out astral-sh/uv#21011 and astral-sh/uv#21012: that issue and fix allow whitespace before a version with an existing `.*` on an ordered comparator such as `>=`, using a different rule. Astral-sh/uv#1477, astral-sh/uv#1529, astral-sh/uv#2546, and astral-sh/uv#2550 concern ordered-comparator metadata and interactions with quote stripping, not losing digits while adding a dot. Astral-sh/uv#8326 concerns rejection of bare `==*` with no release prefix.
