# Misleading version constraint output from `uv pip install`

Issue: astral-sh/uv#23293

Classification: bug

## Summary

On Python 3.13.16 with uv 0.12.22, `uv pip install 'aprict<=0.1.0'` correctly fails to resolve, but explains the exclusions in two groups: two development releases were yanked, and the rest of the range requires Python 3.14 or later. Both yanked releases also declare `Requires-Python: >=3.14`. The separate yank clauses split the ranges, make the explanation longer, and suggest that obtaining the yanked files could get around the Python requirement. The reporter wants the diagnostic to mention yanking only when the release would otherwise be usable; they are not asking to change yank eligibility.

The closest historical work addressed repeated yank clauses or redundant version lists. It does not cover a Python requirement that is missing from the trace for yanked releases.

## Draft response

The resolver is right to reject `aprict<=0.1.0` on Python 3.13, but the error gives the wrong impression. Both yanked releases also declare `Requires-Python: >=3.14`, so downloading either one won’t make it compatible with Python 3.13. The same message reproduces on uv 0.13.9. If you can change interpreters, use Python 3.14 or newer to satisfy the package’s Python requirement. The diagnostic should make that shared requirement clear without changing how uv handles yanked releases.

## Classification

This is a diagnostic correctness bug, not a request for new install behavior. The resolution fails correctly; the message omits an independently sufficient reason for excluding the yanked releases and implies a workaround that does not satisfy their metadata. No existing item found tracks that same interaction. The historical simplifications were merged in 2024, before both affected uv releases, and did not address this interaction; there is no evidence that a previously fixed bug returned.

## Related

- astral-sh/uv#3877 (closed) — The closest yank-specific report: consecutive `sentry` releases repeated the same yank reason. A maintainer closed it as a special case of astral-sh/uv#1901. It did not report a Python requirement hidden by yanking.
- astral-sh/uv#1901 (closed) — The canonical thread for failure to combine repeated incompatibilities for unusable distributions; its original example repeated the same missing Python ABI wheel reason. It did not track how to choose between two applicable reasons for the same files.
- astral-sh/uv#6154 (merged) — Adds error-tree reduction for versions unavailable for the same reason. It can combine reasons already recorded by the resolver, but does not surface a `Requires-Python` check skipped for a yanked distribution.
- astral-sh/uv#4075 (closed) — A nearby verbosity report: the diagnostic already said that all `rooster-blue` versions required newer Python, then listed those versions redundantly. Here, the all-versions conclusion is absent because the yanked releases are reported separately.
- astral-sh/uv#9885 (merged) — Fixes astral-sh/uv#4075 by dropping redundant availability enumeration when the version ranges match. It does not add a Python incompatibility to the diagnostic for a candidate that was rejected as yanked.

## Supporting evidence

- A dry-run `uv pip install 'aprict<=0.1.0'` against PyPI in a CPython 3.13.18 virtual environment reproduced the separate yank and Python clauses with uv 0.12.22 and uv 0.13.9. Both exited with status 1. GitHub records those releases on October 2, 2026, and November 17, 2026, respectively.
- PyPI release metadata shows `Requires-Python: >=3.14` on every published file at or below `0.1.0`, including the wheel and source archive for each yanked release (`0.0.24.dev1` and `0.0.34.dev8`).
- In the reported release and this checkout, `crates/uv-resolver/src/version_map.rs` marks registry wheels and source distributions as incompatible when the yank is not allowed. In `crates/uv-resolver/src/resolver/mod.rs`, an incompatible candidate returns before `check_requires_python` is called. That ordering explains why the Python requirement is absent from the yank clauses. The reproduction establishes the diagnostic problem; it does not establish that changing candidate selection is the right fix.

## Search scope

Searched open and closed issues using `aprict`, the two development versions, and the fragments `was yanked`, `does not satisfy`, and `cannot be used`. Searched separately for the underlying Python-compatibility omission and the verbose version enumeration using `Requires-Python`, unavailable distributions, incompatible versions, redundant clauses, and the `area:error-messages` label. Authenticated PR search did not surface known PRs, so the check also screened the titles and relevant descriptions of all open PRs and the 1,000 most recent PRs across open, closed, and merged states, back through the reported release. Older fixes were checked through issue comments and cross-references.

The output in astral-sh/uv#622 mentions both yanking and Python ABI, but for different version ranges, and the report is about releases appearing unavailable when no wheel matches. The fix in astral-sh/uv#6160 removes tautological dependency clauses, and the discussion in astral-sh/uv#9886 led to astral-sh/uv#9957 simplifying Python clauses already present in the trace. Neither recovers a missing incompatibility reason. The report in astral-sh/uv#8280 concerns verbose listing in a longer dependency trace, not missing Python metadata on yanked candidates. The work in astral-sh/uv#21430 and astral-sh/uv#21435 concerns which artifacts are retained in a lockfile, not the reason shown for a failed `pip install`.
