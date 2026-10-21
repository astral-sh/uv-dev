# Panic (`OperatorLocalCombo`) when `python_full_version not in '<version>+local'` is combined with an upper bound

Issue: astral-sh/uv#22804

Classification: bug

## Summary

Reproduced on the installed uv 0.13.4 binary: `uv lock --offline`,
`uv pip compile --universal --no-index`, and `uv export --frozen` all abort with
`OperatorLocalCombo { operator: LessThan, version: "3.20+x" }` when a
`python_full_version not in '3.20+x'` marker is combined with an upper bound of `< '4'`.
Each process terminated with SIGABRT, matching shell status 134. No dependency download
or package build is needed.

The reproduction ran on Ubuntu 24.04.5 x86_64, rather than the reported Arch Linux host.
The exact lock fixture reproduced with CPython 3.14.8. Universal compilation also
reproduced before Python 3.14 was installed, with host Python 3.12.3 available.
The reported three non-panicking variants returned ordinary resolution errors in
universal compilation. Removing the export lockfile's upper-bound resolution marker
made frozen export succeed.

No duplicate was identified in the recorded related-issue investigation.
astral-sh/uv#6172 provides containment-marker implementation history;
astral-sh/uv#21310 and astral-sh/uv#21311 concern adjacent parsing limitations.
The report is synthetic, with no claimed affected real project. It also names uv 0.12.24;
that version was not tested, and no last known-good release or regression boundary was established.

## Classification

Bug: an input marker causes a process abort instead of a controlled diagnostic or
successful marker handling. This classification rests on observed CLI behavior.
The choice between rejecting this marker and supporting a normalized form remains a
maintainer decision; limited containment support does not require accepting this syntax.

No inspected issue or pull request was identified as the same failure. The adjacent
containment discussions establish compatibility constraints, and no inspected historical
fix establishes this as a recurrence of a previously fixed bug.

## Related

- astral-sh/uv#6172 — Add support for `python_version in ...` markers (pull request, merged). Introduced version-field containment support with deliberately limited, whitespace-separated exact-version semantics. Its discussion explicitly covers python_full_version; it establishes the relevant implementation history but does not address local-version exclusions combined with bounds.
- astral-sh/uv#21310 — uv-pep508 evaluates python_version in a,b,c as always true (issue, closed). Nearby containment-marker correctness report. Comma-separated versions were ignored and evaluated as true; the discussion declined broader support because version-field containment is deprecated. It reports neither local-version bounds nor a panic.
- astral-sh/uv#21311 — fix: Allow commas as python_version string separators (pull request, open). Proposes a parser change for astral-sh/uv#21310. Maintainer comments explain why containment support should not be expanded. It remains unmerged and concerns separators, not the local-version range-rendering panic, so it is not a duplicate or a fix for this report.

## Reproduction

Outcome: **reproducible** for all three reported command paths.

### Environment and isolation

- Executable on PATH: `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv`.
- Version: `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS, Linux `7.0.0-1017-azure`, x86_64.
- Initial host interpreter: Python 3.12.3. `uv python install 3.14` installed CPython
  3.14.8 into the temporary reproduction directory for the exact lock example.
- Reported environment: Arch Linux, Linux 7.2.8 x86_64, uv 0.13.4 and 0.12.24,
  Python 3.14 (patch version unspecified).
- Fixtures and captured stdout/stderr are under `/tmp/uv-22804-b0ezgnie/`.
  Subprocesses used an isolated environment with temporary HOME, TMPDIR, XDG directories,
  `UV_CACHE_DIR`, and `UV_PYTHON_INSTALL_DIR`; `UV_NO_CONFIG=1`, `UV_OFFLINE=1`, and
  `UV_PYTHON_DOWNLOADS=never` were set for reproduction commands. Only the explicit
  Python installation enabled network access. Core dumps were disabled.
- The installed uv executable was used throughout. No checkout build, source edit,
  test edit, or GitHub change was made.

### Offline lock

In a temporary directory, write `pyproject.toml`:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = ["a ; python_full_version not in '3.20+x' and python_full_version < '4'"]
```

With Python 3.14 available, run:

```sh
uv lock --offline
```

Observed stderr, omitting the variable thread identifier:

```text
Using CPython 3.14.8

thread 'main2' panicked at crates/uv-pep440/src/version_specifier.rs:629:78:
called `Result::unwrap()` on an `Err` value: VersionSpecifierBuildError { kind: OperatorLocalCombo { operator: LessThan, version: "3.20+x" } }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

Python's subprocess API returned `-6` (SIGABRT), equivalent to shell status 134.
Before installing Python 3.14, this exact fixture exited 2 because no interpreter
satisfying `>=3.14` was available. That interpreter prerequisite was resolved without
changing the reported fixture.

### Universal compilation

In another temporary directory, write `requirements.in`:

```text
a ; python_full_version not in '3.20+x' and python_full_version < '4'
```

Run:

```sh
uv pip compile --universal --no-index requirements.in
```

Observed the same source location and exact `OperatorLocalCombo` error in thread
`main`, followed by SIGABRT (`subprocess.returncode == -6`, shell equivalent 134).
This reproduced with only host Python 3.12.3 available. Package `a` does not need to exist.

Three separate control inputs were each passed to the same universal compile command:

| Requirement marker | Observed result |
| --- | --- |
| `python_full_version not in '3.20+x'` | Exit 1, ordinary unsatisfiable-requirements diagnostic; no panic. |
| `python_full_version != '3.20+x' and python_full_version < '4'` | Exit 1, ordinary unsatisfiable-requirements diagnostic; no panic. |
| `python_full_version not in '3.20' and python_full_version < '4'` | Exit 1, ordinary unsatisfiable-requirements diagnostic; no panic. |

The controls have no available package versions because index access is disabled.
They validate the reported panic distinction without establishing that these marker
spellings have equivalent semantics.

### Frozen export

The report describes a lockfile shape but does not supply a complete file. The following
minimal fixture was constructed and tested. Its package entries do not require downloads.

`pyproject.toml`:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = ["a ; python_full_version not in '3.20+x'"]
```

`uv.lock`:

```toml
version = 1
revision = 3
requires-python = ">=3.14"
resolution-markers = ["python_full_version < '4'"]

[[package]]
name = "repro"
version = "0.1.0"
source = { virtual = "." }
dependencies = [
    { name = "a", marker = "python_full_version not in '3.20+x'" },
]

[[package]]
name = "a"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
```

Run:

```sh
uv export --frozen
```

Observed the generated-file header, followed by the same `OperatorLocalCombo` panic
at `version_specifier.rs:629:78` in thread `main2`, and SIGABRT (`-6`, shell equivalent 134).
As a control, removing only the `resolution-markers` line made the command exit 0 and emit:

```text
a==1.0.0 ; python_full_version != '3.20+x'
    # via repro
```

### Existing test coverage

Searched `crates/uv/tests/`, including `crates/uv/tests/it/`, and
`crates/uv-client/tests/it/`, plus the marker and version-specifier crates.
No CLI regression test for this local-suffix containment plus upper-bound combination
was found. In this checkout the relevant integration modules are
`crates/uv/tests/lock/lock.rs`, `crates/uv/tests/pip_compile/pip_compile.rs`, and
`crates/uv/tests/project/export.rs`.

Read the following related test setups and assertions:

- `crates/uv-pep508/src/marker/tree.rs::test_version_in_evaluation` checks version-field
  containment against Python 2.7 and 3.7 marker environments, including `not in` for
  `python_version` and `implementation_version`. It does not exercise a local suffix
  combined with an upper bound or CLI rendering.
- `crates/uv-pep508/src/marker/tree.rs::test_marker_simplification` checks marker equality
  and rendered strings through `assert_simplifies`, including ordinary version containment
  and unbounded exclusions. It lacks the reported bounded local-version exclusion.
- `crates/uv/tests/project/export.rs::requirements_txt_simplifies_selected_root_extra_markers_from_lock`
  writes a minimal lockfile and snapshots successful frozen export after selecting extras.
  It establishes an existing fixture pattern but does not cover version containment.

No repository tests were run or added; the installed CLI was exercised directly.
Any follow-up regression tests should use the existing integration modules and applicable
feature gates without duplicating these distinct existing cases.

## Supporting source evidence

Inspected checkout: `1765e51d67f43299d8fbd37b302b773a80c8499d`.
The following source behavior is consistent with the observed panic and control cases:

- `crates/uv-pep508/src/marker/parse.rs:393`: `parse_version_in_expr` parses whitespace-separated
  values using `Version::from_str`, retaining local segments.
- `crates/uv-pep508/src/marker/algebra.rs:249`: `VersionIn` expressions for
  `PythonFullVersion` use `Edges::from_versions`.
- `crates/uv-pep508/src/marker/algebra.rs:1692`: `from_versions` constructs singleton
  ranges without stripping suffixes, then complements the range for `NotIn`.
  Ordinary comparison specifiers instead go through `only_release()` at line 1659.
- `crates/uv-pep508/src/marker/simplify.rs:264`: `range_inequality` renders exclusions
  as `!=` only when the overall bounding range is unbounded on both sides.
  The upper bound prevents that shortcut.
- `crates/uv-pep508/src/marker/simplify.rs:94`: other version ranges are rendered through
  `VersionSpecifier::from_release_only_bounds`.
- `crates/uv-pep440/src/version_specifier.rs:448`: `from_version` rejects an incompatible
  operator with a local version using `OperatorLocalCombo`. `from_upper_bound` at line 623
  constructs `LessThan` for an excluded bound and unwraps that result at line 629.
  The observed panic identifies precisely this rejected operator/version pair and location.

Runtime evidence confirms the abort and input distinctions. It does not establish a
release regression boundary or which rejection/normalization policy should replace the panic.

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

Add focused regression coverage for the confirmed offline lock, universal compile, and
frozen export paths where they exercise distinct rendering behavior. Check existing coverage
and applicable feature gates before adding tests.

Choose controlled rejection or normalization for local versions in containment ranges,
consistent with the limited support discussed in astral-sh/uv#21311. Avoid constructing
an ordered local-version specifier and unwrapping its error. The successful controls do not
establish that replacing `not in` with `!=` preserves every intended semantic distinction.
