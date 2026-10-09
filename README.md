# Test regression on Gentoo: `venv::create_venv_caches_interpreter`

Issue: astral-sh/uv#22383

Classification: bug

## Summary

Independently reproduced the reported cached-versus-queried `sys_base_executable`
mismatch with uv 0.12.24 and CPython 3.12.15 on Ubuntu 24.04.5 x86_64. Selecting
Python through a test-harness-style executable symlink is sufficient; Gentoo is
not required. The same fixture produces consistent metadata with uv 0.12.23 and
the installed uv 0.12.13. Cache warming was introduced by astral-sh/uv#21304 in
uv 0.12.24. The direct fix is open in astral-sh/uv#22394, following proposals in
astral-sh/uv-dev#2548 and astral-sh/uv-dev#2549.

The reporter runs uv 0.12.24's tests on Gentoo Linux amd64 with CPython 3.12.15.
The assertion at `crates/uv/tests/python/venv.rs:118` compares the environment loaded
from the warmed interpreter cache with one queried using a fresh cache. The reported
difference is `sys_base_executable`: the cached environment contains the test
harness path ending in `python/3.12/python3`, while the freshly queried environment
contains `/usr/bin/python3.12`. The expected result is identical interpreter
metadata regardless of whether the cache is used.

## Draft response

Reproduced with uv 0.12.24 and CPython 3.12.15 on Ubuntu using a `python3`
symlink, matching the test harness setup. The warmed cache records that symlink
as the base executable; querying the venv's Python reports the real `python3.12`
path. Child venvs consequently select different executable targets on cache hits
and misses. The same setup produces matching metadata and child-venv targets
with uv 0.12.23. Cache warming and this test were introduced in
astral-sh/uv#21304, released in 0.12.24.

A fix is open in astral-sh/uv#22394. It skips cache warming for symlinked base executables and invalidates affected cached metadata. Please rerun the failing test with that patch on Gentoo and let us know whether it passes.

## Classification

Bug: independent runtime reproduction confirms incorrect cached interpreter
metadata. The warmed `sys_base_executable` retains the selected `python3`
symlink, while a fresh venv query reports the real `python3.12` executable.
Among the cached fields consumed by `Interpreter`, this is the only difference
in the reproduction. Cache warming and this test were introduced by
astral-sh/uv#21304 in uv 0.12.24. No earlier canonical report or previously fixed
instance of this same problem was found. The directly related fixes were created
in response to this issue, so their existence does not justify duplicate
classification.

The metadata discrepancy affects child-venv executable selection, so it extends
beyond the test assertion. Runtime observations, repository source, and maintainer
review comments agree on the executable-symlink mechanism. The original Gentoo
Rust test and the proposed fix were not executed during this reproduction.

## Reproduction

**Outcome: reproducible.** The reported field mismatch was observed directly in
uv's generated interpreter-cache records and in child-venv executable targets.
All reproduction files, downloaded tools, and caches are under
`/tmp/uv-22383-y6JmCn`; no checkout files or GitHub objects were changed.

### Environment and preparation

- Report: Gentoo Linux amd64, uv 0.12.24, CPython 3.12.15; the reported kernel is
  `7.2.9-gentoo-dist-bin`, and interpreter metadata identifies glibc 2.44.
- Observed locally: Ubuntu 24.04.5 x86_64, CPython 3.12.15 from
  `/opt/hostedtoolcache/Python/3.12.15/x64/bin/python3.12`, a non-standalone
  interpreter. The interpreter's version matches the report, while its prefix
  and operating-system distribution differ.
- The installed `uv` on `PATH` is 0.12.13. It was used both as a baseline and to
  install the published binary wheels for uv 0.12.23 and 0.12.24 into separate
  temporary `--target` directories. `msgpack==1.1.2` was installed into another
  temporary directory to decode only the newly generated interpreter records.
  No Rust builds were needed.
- Reproduction subprocesses use an explicit minimal environment, a temporary
  `TMPDIR` and `UV_PYTHON_INSTALL_DIR`, and `UV_PYTHON_DOWNLOADS=never`. Every uv
  scenario uses `--no-config --offline --cache-dir <temporary-directory>`.
  There is no project, workspace, dependency group, or package dependency.

### Minimal command sequence

The following distills the observed child-venv behavior. `UV_BIN` selects the
temporarily installed affected binary; it can be replaced with a uv 0.12.24
executable on another machine. The full measurement script also checks the
startup marker and decodes the cache records.

```sh
repro=$(mktemp -d /tmp/uv-22383-minimal-XXXXXX)
UV_BIN=/tmp/uv-22383-y6JmCn/uv-0.12.24/bin/uv
PYTHON_BIN=/opt/hostedtoolcache/Python/3.12.15/x64/bin/python3.12
mkdir -p "$repro/python/3.12"
ln -s "$PYTHON_BIN" "$repro/python/3.12/python3"
"$UV_BIN" --no-config --offline --cache-dir "$repro/warm" \
  venv --python "$repro/python/3.12/python3" "$repro/parent"
"$UV_BIN" --no-config --offline --cache-dir "$repro/warm" -v \
  venv --python "$repro/parent/bin/python" "$repro/child-warm"
"$UV_BIN" --no-config --offline --cache-dir "$repro/fresh" -v \
  venv --python "$repro/parent/bin/python" "$repro/child-fresh"
readlink "$repro/child-warm/bin/python"
readlink "$repro/child-fresh/bin/python"
"$repro/parent/bin/python" -I -B -c 'import sys; print(sys._base_executable)'
```

For uv 0.12.24, the first child target is
`$repro/python/3.12/python3`; the second child target and Python's
`sys._base_executable` are the real `$PYTHON_BIN` path. All uv commands succeed;
the failure is inconsistent metadata and executable selection.

### Direct metadata comparison and version controls

The full reproduction creates the parent venv, then installs this
`lib/python3.12/site-packages/sitecustomize.py`, following the existing test:

```python
from pathlib import Path
Path(__file__).with_name("interpreter-started").touch()
```

It runs `uv pip list --python <parent>/bin/python` once with the creation cache
and once with a fresh cache, then reads their `interpreter-v4/**/*.msgpack`
records for that exact executable. This establishes whether Python actually
started and compares uv's stored `sys_base_executable`, rather than inferring it
from source alone.

| uv version | Parent metadata warmed at creation | Python starts on first lookup with original cache | Cached versus fresh base executable | Child targets |
| --- | --- | --- | --- | --- |
| 0.12.24 | Yes | No | Symlink versus real executable | Different |
| 0.12.23 | No | Yes | Both real executable | Identical |
| 0.12.13, installed on PATH | No | Yes | Both real executable | Identical |

The fresh-cache lookup starts Python in all three cases. For uv 0.12.24, the
observed cached base is
`/tmp/uv-22383-y6JmCn/affected-0.12.24/python/3.12/python3`; the queried base is
`/opt/hostedtoolcache/Python/3.12.15/x64/bin/python3.12`. The records also differ
in `sys_base_exec_prefix` and `sys_path`, which `from_virtualenv` intentionally
leaves empty and `Interpreter` does not consume. All fields consumed by
`Interpreter` match except `sys_base_executable`, precisely the field in the
reported assertion. The two older versions produce identical complete records
for the parent venv after lookup.

Evidence files:

- `/tmp/uv-22383-y6JmCn/reproduce.py` — the executed measurement script.
- `/tmp/uv-22383-y6JmCn/affected-0.12.24.log` — commands, marker observations,
  decoded differences, and child-venv targets for the affected version.
- `/tmp/uv-22383-y6JmCn/baseline-0.12.23.log` and
  `/tmp/uv-22383-y6JmCn/installed-0.12.13.log` — explicit control runs.
- Each scenario directory contains `cached.json` and `queried.json` with the
  decoded parent-interpreter records.

### Existing test coverage and limits

`crates/uv/tests/python/venv.rs::create_venv_caches_interpreter` is the exact
reported integration test. Its setup creates environments from both a selected
Python 3.12 and a parent venv. Its `sitecustomize.py` marker asserts that a warmed
lookup skips Python, a fresh-cache lookup starts Python, and the resulting
`PythonEnvironment` values are equal. The module is gated by `test-python` in
`crates/uv/tests/python/main.rs`; the test has no additional gate.
`crates/uv-test/src/lib.rs` supplies the temporary `python3` symlink on Unix.
The nearby `create_venv_caches_upgradeable_interpreter` test additionally requires
`test-python-managed` and covers recreation of an upgradeable managed venv,
which is a different configuration.

The original Rust assertion was not run, and no Gentoo installation was used.
Instead, the matching field discrepancy was measured with the published affected
binary, the reported Python version, and the harness's symlink arrangement.
The reproduction establishes the behavior on Ubuntu as well as the release
boundary; it does not validate the proposed fix.

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
Review and validate that patch against the reported Gentoo test invocation and
the reproduced executable-symlink case. The uv 0.12.24 behavior is independently
confirmed, with uv 0.12.23 providing a passing behavior control. Testing the
proposed fix downstream remains the useful next step.

This temporary handoff was updated and isolated CLI reproductions were run. No
checkout files, existing user state, or GitHub objects were modified. No Rust
builds or repository test binaries were run.
