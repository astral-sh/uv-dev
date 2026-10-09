# Test regression on Gentoo: `venv::create_venv_caches_interpreter`

Issue: astral-sh/uv#22383

Classification: bug

## Summary

Cache-warming regression introduced by astral-sh/uv#21304 in uv 0.12.24. The direct fix is open in astral-sh/uv#22394, following proposals in astral-sh/uv-dev#2548 and astral-sh/uv-dev#2549.

The reporter runs uv 0.12.24's tests on Gentoo Linux amd64 with CPython 3.12.15.
The assertion at `crates/uv/tests/python/venv.rs:118` compares the environment loaded
from the warmed interpreter cache with one queried using a fresh cache. The reported
difference is `sys_base_executable`: the cached environment contains the test
harness path ending in `python/3.12/python3`, while the freshly queried environment
contains `/usr/bin/python3.12`. The expected result is identical interpreter
metadata regardless of whether the cache is used.

## Draft response

The mismatch comes from venv cache warming: uv records the test harness's `python3` symlink as the base executable, while CPython queried inside the venv reports `/usr/bin/python3.12`. Cache warming and this test were introduced in astral-sh/uv#21304, released in 0.12.24.

A fix is open in astral-sh/uv#22394. It skips cache warming for symlinked base executables and invalidates affected cached metadata. Please rerun the failing test with that patch on Gentoo and let us know whether it passes.

## Classification

The reported assertion and source establish incorrect cached interpreter metadata: the warmed sys_base_executable retains the test harness's python3 symlink, while a fresh venv query reports /usr/bin/python3.12. Cache warming and this test were introduced by astral-sh/uv#21304 in the reported release, uv 0.12.24. No earlier canonical report or previously fixed instance of this same problem was found. The directly related fixes were created in response to this issue, so their existence does not justify duplicate classification.

The report establishes a correctness failure, not a request for new functionality
or merely a question about test configuration. Repository source and maintainer
review comments support the executable-symlink mechanism. This handoff does not
claim an independent Gentoo reproduction or a released fix.

## Related

- astral-sh/uv#22394 — Skip venv cache warming for symlinked base executables (open). Direct proposed fix for astral-sh/uv#22383. Skips cache warming for symlinked Unix base executables, clears stale destination entries, and advances the interpreter cache to v5. Adds coverage for matching cached and queried base paths and child-venv targets. Opened after this report, so it does not make the issue a duplicate.
- astral-sh/uv#21304 — Warm the interpreter cache when syncing workspace metadata (merged). Introduced venv interpreter-cache warming and the exact failing test. Merged on October 7, 2026, and included in uv 0.12.24 on October 8. Its inferred metadata retains the selected base-executable path, explaining the release timing and the discrepancy exposed by the test.
- astral-sh/uv-dev#2548 — Resolve executable symlinks when warming venv interpreter caches (open). Initial proposed fix explicitly targeting astral-sh/uv#22383, resolving executable symlinks while retaining parent-directory links. A maintainer points to astral-sh/uv-dev#2549 as a simpler alternative.
- astral-sh/uv-dev#2549 — Skip venv cache warming for symlinked base executables (closed). Alternative fix linked from astral-sh/uv-dev#2548. Closed without merging after promotion to astral-sh/uv#22394; documents the same symlink mismatch and reports targeted regression tests passing.

## Supporting evidence

- `crates/uv/tests/python/venv.rs:78` defines the failing test. It creates a venv,
  uses a `sitecustomize.py` marker to distinguish cached metadata from a Python
  subprocess query, and compares the resulting environments at line 118.
- `crates/uv-test/src/lib.rs:1018` normalizes Unix test interpreter names by
  creating a `python3` symlink for each selected version. This accounts for the
  temporary executable path in the report without requiring a Gentoo-specific
  symlink convention.
- `crates/uv-python-interpreter/src/interpreter.rs:151` assigns
  `virtualenv.base_executable` to `sys_base_executable`.
  `crates/uv/src/commands/venv.rs:281` then calls `cache_virtualenv`.
  `crates/uv-python-interpreter/src/environment.rs:369` derives and caches the
  new environment's metadata without running its Python. The inspected checkout
  has no general guard for a symlinked Unix base executable.
- The diff and test-inventory comment on astral-sh/uv#21304 confirm that it added
  both cache warming and `create_venv_caches_interpreter`. It merged on
  October 7, 2026. `changelogs/0.12.x.md:1390` dates uv 0.12.24 to October 8,
  and the performance entry at line 1425 attributes cache warming to that PR.
  The report was opened October 9.
- In review comments on astral-sh/uv#22394, a maintainer explains that the symlink
  comes from the test harness and that Python can report its unresolved path
  when called directly but its resolved target when called inside a venv.
  The proposed patch queries Python instead of inferring this case, clears the
  destination entry on recreation, and changes the cache bucket from
  `interpreter-v4` to `interpreter-v5`.
- The proposed regression test in astral-sh/uv#22394 checks cached versus queried
  base paths and the executable targets of child venvs. Its description reports
  three venv-cache tests and two interpreter-cache tests passing, plus targeted
  Clippy and formatting checks. These are PR-reported results, not tests run
  during this handoff.
- The only comment on astral-sh/uv#22383 suggests using a snapshot to improve the
  assertion's readability; it does not establish a separate root cause or fix.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered create_venv_caches_interpreter, caches_interpreter, sys_base_executable, and sys._base_executable. Conceptual searches covered Gentoo venv failures, interpreter-cache tests, base executables, symlinks, cache warming, and internal:testing, including searches without platform/version qualifiers. PR search returned no hits, so supplemented it with all-state PR listings, release notes, and direct PR/comment/diff inspection. Followed astral-sh/uv-dev#2548 through astral-sh/uv-dev#2549 to astral-sh/uv#22394. Ruled out astral-sh/uv#18510 (system-site-packages cache pollution), astral-sh/uv#15188 (snapshot filtering), astral-sh/uv#17509 (test environment isolation), astral-sh/uv#21073 (stale version metadata after libpython changes), and astral-sh/uv#22310 (timestamp-sensitive cache-invalidation test). None establishes a prior fix of this symlink-path regression.

The older Gentoo reports do not match the failed metadata equality assertion:
astral-sh/uv#15188 concerns a Python-path snapshot filter, while comments on
astral-sh/uv#17509 trace widespread failures to environment-variable isolation.
The latter discussion points to astral-sh/uv#17515 and astral-sh/uv#17659 for those
separate failures.

Although the cache-warming source mentions astral-sh/uv#18510, that issue requires
system-site-packages transitions and concerns inherited import paths; the
reported field here is the base executable. The historical fix
astral-sh/uv#21073 handles stale Python versions when a launcher loads a different
libpython, and astral-sh/uv#22310 fixes a mock-executable timestamp assumption.
Neither is evidence that this previously fixed bug returned.

## Status and next step

As inspected on October 9, 2026, astral-sh/uv#22394 remains open and unmerged.
Review and validate that patch against the Gentoo test invocation. The reporter's
environment is already sufficiently identified to classify the failure; testing
the proposed fix downstream is the useful next step.

Only this temporary handoff was updated. No checkout files or GitHub objects were
modified, and no builds or tests were run.
