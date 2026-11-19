# Misleading version constraint output from `uv pip install`

Issue: astral-sh/uv#23293

Classification: bug

## Summary

On CPython 3.13.16, `uv pip install 'aprict<=0.1.0'` correctly fails to resolve. The diagnostic separates two yanked development releases from the versions that require Python 3.14 or later, even though both yanked releases declare the same Python requirement. The reporter wants the diagnostic to mention yanking only when the release would otherwise be usable, without changing yank eligibility.

## Reproduction

**Outcome: reproducible.** The report used uv 0.12.22 and CPython 3.13.16 on Arch Linux x86_64. On Linux x86_64 (kernel 7.0.0-1017-azure), the installed uv 0.13.7 and PyPI-distributed uv 0.12.22 and 0.13.9 produced identical diagnostics with CPython 3.13.16. The runs used public PyPI, no project or uv configuration, and a temporary environment and cache. `--dry-run` avoids installing `aprict`.

From a fresh temporary directory:

```sh
export UV_CACHE_DIR="$PWD/cache" UV_PYTHON_INSTALL_DIR="$PWD/python"
uv --no-config venv --python 3.13.16 .venv
uv --no-config pip install --dry-run --python .venv/bin/python \
  --default-index https://pypi.org/simple 'aprict<=0.1.0'
```

All three uv versions exited 1. The diagnostic said `0.0.24.dev1` and `0.0.34.dev8` were unusable because they were yanked. It cited `Python>=3.14` only for the remaining versions, splitting them into three ranges. PyPI metadata listed `Requires-Python: >=3.14` on all 78 published files at or below `0.1.0`, including the wheel and source archive of each yanked release. As controls, replacing the range with `aprict==0.0.24.dev1` and, separately, `aprict==0.0.34.dev8` on uv 0.13.7 also exited 1; each exact pin reported the Python incompatibility directly.

In `crates/uv/tests/pip/pip_install_scenarios.rs`, `package_only_yanked_in_range` snapshots rejection when only a yanked version is in range, and `python_greater_than_current` snapshots rejection when an exact pin requires a newer Python. Neither fixture combines both reasons on the same release. No test covering that interaction was found in the inspected integration tests.

In the checkout, `crates/uv-resolver/src/version_map.rs` marks disallowed yanked distributions as incompatible, and `crates/uv-resolver/src/resolver/mod.rs` returns from the incompatible-candidate branch before `check_requires_python`. This is consistent with the difference between the range and exact-pin diagnostics; it does not establish that candidate selection needs to change.

## Classification

This is a diagnostic bug, not an incorrect resolution result. The range message omits a Python requirement that independently rules out the yanked releases, which can give the impression that obtaining either file would work around the failure. The related diagnostic simplifications address repeated reasons or redundant version lists, not this interaction. No duplicate or regression was established.

## Related

- astral-sh/uv#3877 (closed) — The closest yank-specific report: consecutive `sentry` releases repeated the same yank reason. A maintainer closed it as a special case of astral-sh/uv#1901. It did not report a Python requirement hidden by yanking.
- astral-sh/uv#1901 (closed) — The canonical thread for failure to combine repeated incompatibilities for unusable distributions; its original example repeated the same missing Python ABI wheel reason. It did not track how to choose between two applicable reasons for the same files.
- astral-sh/uv#6154 (merged) — Adds error-tree reduction for versions unavailable for the same reason. It can combine reasons already recorded by the resolver, but does not surface a `Requires-Python` check skipped for a yanked distribution.
- astral-sh/uv#4075 (closed) — A nearby verbosity report: the diagnostic already said that all `rooster-blue` versions required newer Python, then listed those versions redundantly. Here, the all-versions conclusion is absent because the yanked releases are reported separately.
- astral-sh/uv#9885 (merged) — Fixes astral-sh/uv#4075 by dropping redundant availability enumeration when the version ranges match. It does not add a Python incompatibility to the diagnostic for a candidate that was rejected as yanked.

## Search scope

Searched open and closed issues using `aprict`, the two development versions, and the fragments `was yanked`, `does not satisfy`, and `cannot be used`. Searched separately for the underlying Python-compatibility omission and the verbose version enumeration using `Requires-Python`, unavailable distributions, incompatible versions, redundant clauses, and the `area:error-messages` label. Authenticated PR search did not surface known PRs, so the check also screened the titles and relevant descriptions of all open PRs and the 1,000 most recent PRs across open, closed, and merged states, back through the reported release. Older fixes were checked through issue comments and cross-references.

The output in astral-sh/uv#622 mentions both yanking and Python ABI, but for different version ranges, and the report is about releases appearing unavailable when no wheel matches. The fix in astral-sh/uv#6160 removes tautological dependency clauses, and the discussion in astral-sh/uv#9886 led to astral-sh/uv#9957 simplifying Python clauses already present in the trace. Neither recovers a missing incompatibility reason. The report in astral-sh/uv#8280 concerns verbose listing in a longer dependency trace, not missing Python metadata on yanked candidates. The work in astral-sh/uv#21430 and astral-sh/uv#21435 concerns which artifacts are retained in a lockfile, not the reason shown for a failed `pip install`.
