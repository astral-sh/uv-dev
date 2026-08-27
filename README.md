# uv-pep508 evaluates python_version in a,b,c as always true

Issue: astral-sh/uv#21310

Classification: bug

## Summary

The reported behavior was reproduced with the installed uv 0.12.6 on Linux, using CPython 3.12.3.
A project restricted to Python 3.12 incorrectly attempted to resolve a dependency guarded by
`python_version in "3.8,3.9"`. Under the historical PEP 508 substring semantics, that dependency
should be inactive because `3.12` is not a substring of `3.8,3.9`. The same result occurred with
`"3.8, 3.9"`, while the whitespace-separated control `"3.8 3.9"` was correctly inactive. This demonstrated downstream uv
resolver impact. The reporter subsequently identified the construct in a corpus of PyPI wheels,
including example package versions and wheel filenames.

The implementation cause is `parse_version_in_expr` splitting the right-hand string only on
whitespace and parsing every token as a PEP 440 version. A token such as `3.8,3.9,3.13` failed that
parse, so the expression was ignored and the resulting marker tree was `true`. A proposed fix
extends that existing version-aware parser to recognize commas as separators and updates the
marker-simplification expectations accordingly, but it has not been accepted upstream.

The limitation originated in astral-sh/uv#6172. Its discussion explicitly considered
`python_full_version in "3.11,3.12,3.13"`, then chose strict whitespace-only support until more
edge cases arose. This report supplies such a case. No newer issue or pull request was found at
initial triage; astral-sh/uv#21311 and astral-sh/uv-dev#889 have since been opened with
implementations for comma-delimited version membership.

## Reported ecosystem impact

The reporter searched their PyPI corpus and reported at least 164 wheels across 14 projects using
comma-delimited version membership markers. This corpus result has not been independently verified,
and it may not include affected source distributions, but it establishes that the syntax is not
limited to a hypothetical or single-package case.

Reported wheel counts by project:

- `flight-profiler`: 98
- `fortls`: 16
- `responses`: 14
- `vcrpy`: 9
- `awslogs`: 5
- `colander`: 4
- `django-fluent-comments`: 4
- `analytics-python`: 3
- `paste_it`: 3
- `aws2fa`: 2
- `awslogs-oguzzi`: 2
- `honcho`: 2
- `sked`: 1
- `ssawslogs`: 1

The reporter later supplied example wheel filenames, including `analytics_python` 1.2.2–1.2.4,
`aws2fa` 0.0.2–0.0.3, `awslogs` 0.6.0–0.11.0, `awslogs_oguzzi` 0.12.1–0.12.2, and `colander`
1.5–1.6.0. The maintainer observed that all provided markers target EOL Python versions that uv no
longer supports and cited that as a reason not to broaden compatibility. The local reproduction
nevertheless shows that discarding such a marker as true can activate its guarded dependency while
resolving for a currently supported Python version.

## Reproduction

Outcome: **reproducible**.

Environment:

- uv 0.12.6 (`x86_64-unknown-linux-gnu`), installed at `/opt/hostedtoolcache/uv/0.12.6/x86_64/uv`
- CPython 3.12.3 at `/usr/bin/python3.12`
- All fixture files and the uv cache were placed under `$RUNNER_TEMP`; the repository checkout and
  existing user state were not modified.

Minimal `pyproject.toml`:

```toml
[project]
name = "marker-repro"
version = "0.1.0"
requires-python = ">=3.12,<3.13"
dependencies = [
  "definitely-not-a-real-package-uv-21310; python_version in '3.8,3.9'",
]
```

Command:

```console
$ UV_CACHE_DIR="$RUNNER_TEMP/uv-21310-cache" uv --no-config lock --offline --python 3.12
Using CPython 3.12.3 interpreter at: /usr/bin/python3.12
  × No solution found when resolving dependencies:
  ╰─▶ Because definitely-not-a-real-package-uv-21310 was not found in the cache and your project
      depends on definitely-not-a-real-package-uv-21310, we can conclude that your project's
      requirements are unsatisfiable.
```

The failure shows that uv treated the marked dependency as active even though the project's entire
supported Python range is 3.12. Changing the marker to the comma-plus-space variant
`python_version in '3.8, 3.9'` produced the same failure. Changing only the marker value to the
whitespace-separated control `python_version in '3.8 3.9'` succeeded with `Resolved 1 package`,
confirming that the fixture distinguishes the reported separator behavior.

Before the fix, unit coverage in `crates/uv-pep508/src/marker/tree.rs` encoded the limitation:
`test_marker_simplification` asserted that both `python_version in '3.9, 3.10'` and
`python_version in '3.9,3.10'` simplified to true, while `test_version_in_evaluation` verified the
supported whitespace-delimited form. The parent regression added the missing resolver coverage in
`crates/uv/tests/lock/lock.rs`.

## Proposed fix and validation

Outcome: **implemented and validated, but not accepted upstream**.

The parent regression `lock::lock_python_version_in_comma` in `crates/uv/tests/lock/lock.rs` was
first confirmed to pass while snapshotting the undesirable offline resolution failure. Its snapshot
was then changed to require a successful one-package lock; before the production change, that
desired assertion failed because uv still tried to resolve `iniconfig`.

The proposed change to `crates/uv-pep508/src/marker/parse.rs` treats commas, as well as whitespace,
as separators in the existing version-aware `in` and `not in` parser. This preserves the
established marker algebra while allowing comma-delimited PEP 440 versions to produce a real `VersionIn` expression rather
than being discarded. The directly related assertions in
`crates/uv-pep508/src/marker/tree.rs::test_marker_simplification` now verify that comma-separated
forms with and without following spaces simplify to the expected Python full-version range. Other
unsupported arbitrary separators, such as the word `or`, retain their prior behavior. Inspection of
the inverted-operand parser, list-marker parser, simplifier, and lock tests found no separate
producer/consumer implementation affected by this whitespace-tokenization cause.

Successful focused validation:

- `cargo test --package uv-pep508` — 116 unit tests and one doc test passed.
- `cargo test --package uv --test lock --features test-universal lock::lock_python_version_in_comma -- --exact`
  — the updated parent end-to-end lock regression passed.
- `cargo +stable clippy --package uv-pep508 --all-targets -- -D warnings` — passed. The pinned 1.98.0
  toolchain did not have the clippy component installed, so the available stable toolchain was used.
- `cargo +stable fmt --all -- --check` and `git diff --check` — passed. The available stable
  `rustfmt` was used because the pinned toolchain's component directory is read-only and lacks
  `rustfmt`.

## Maintainer decision

In astral-sh/uv#21311, a maintainer declined to extend comma-separated compatibility. The current
dependency-specifier guidance says `in` and `not in` are not valid for version fields; publishing
tools should reject them, while locking and installation tools may reject them or treat them as
false. The maintainer also noted that the concrete wheels supplied in astral-sh/uv#21310 use the
syntax only for EOL Python versions outside uv's supported range. Their stated preference is to
leave uv's existing compatibility behavior unchanged rather than support additional forms of the
deprecated syntax.

This decision supersedes the handoff's earlier expectation that comma support should be merged.
Both astral-sh/uv#21311 and astral-sh/uv-dev#889 remain open at the time of this update, but their
comma-separator approach conflicts with the recorded maintainer direction.

## Classification

Classify astral-sh/uv#21310 as a bug based on the reproduced unconditional-true evaluation. The
historical PEP 508 interpretation used substring matching, while current dependency-specifier
guidance disallows `in` and `not in` for version fields and permits installers to reject the marker
or treat it as false. uv-pep508's specialized whitespace-only parser instead discards the
comma-delimited expression as true, which can activate the guarded dependency on unrelated Python
versions. The reproduction and prior test expectations establish that behavior. A fix was
implemented and validated, but the maintainer decision is not to extend support for the deprecated
syntax because the concrete marker values concern unsupported EOL Python versions.

This is not a regression: astral-sh/uv#6172 never supported comma-separated strings. It is also not
a duplicate of astral-sh/uv#21309. That open issue has the same unconditional-true result and is
part of the broader limitation around valid PEP 508 version membership, but its trigger is a quoted
value on the left (`"3.11" in python_version`) and it follows the inverted-version parser path.
Supporting comma-delimited right-hand strings is a distinct case with separate parsing choices.

## Related

- astral-sh/uv#6172 — **Add support for `python_version in ...` markers** (merged pull request).
  This is the historical implementation of whitespace-delimited, version-aware membership. Its
  maintainer discussion explicitly raised comma-delimited version strings and deferred them while
  asking to see whether additional edge cases arose, making it the strongest design evidence for
  this report.
- astral-sh/uv#3683 — **Support `in` operators with `python_version` marker** (closed issue). This
  original bug tracked version membership being treated as arbitrary/true and was closed by
  astral-sh/uv#6172. Its example uses whitespace-delimited versions, so the implemented fix does
  not cover the present comma-delimited trigger.
- astral-sh/uv#21309 — **uv-pep508 parses value in python_version as universally true** (open
  issue). This adjacent report also shows a valid version-membership expression becoming
  unconditionally true, but it uses reversed operands and exercises a different parser path.
- astral-sh/uv#21311 — **fix: Allow commas as python_version string separators** (open pull
  request). This directly implements the requested comma parsing. A maintainer stated that modern
  guidance disallows containment operators on version fields and declined to extend support,
  noting that the supplied wheel examples target unsupported EOL Python versions.
- astral-sh/uv-dev#889 — **Support comma-delimited Python version membership markers** (open pull
  request). This independently contains the validated parser and lock-test changes described above,
  but its approach is now contrary to the maintainer decision recorded on astral-sh/uv#21311.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `python_version in`, `python_full_version in`, comma-delimited version examples,
`always true`, uv-pep508, and relevant parser identifiers. Conceptual queries covered version
membership and list markers, historical PEP 508 substring behavior, invalid markers becoming true,
version-aware normalization, and marker algebra. Fix-oriented inspection followed the historical
chain from astral-sh/uv#3675 through astral-sh/uv#3683 to astral-sh/uv#6172 and checked for newer
work mentioning astral-sh/uv#21310. The follow-up review also inspected astral-sh/uv#21311 and
astral-sh/uv-dev#889 and incorporated the maintainer's modern-spec compatibility decision.

astral-sh/uv#3675 was inspected but omitted from the related list because it is a downstream
resolver symptom involving the whitespace-delimited `pathlib2` marker already fixed by
astral-sh/uv#6172. astral-sh/uv#20816 was also inspected after a literal comma-version search, but
it concerns conjunction semantics for comma-separated `project.requires-python` specifiers, not
PEP 508 marker membership.
