# Unclear resolution error when `minimum-libc-version` is involved

Issue: astral-sh/uv#23272

Classification: bug (resolution diagnostic)

## Summary

Reproduced the reported pandas failure and its marker-only explanation using the
installed uv 0.13.7 on Linux x86_64 with CPython 3.12.3 and host glibc 2.39. The
report names uv 0.13.8 and leaves the Python version and platform unspecified;
the original reproduction did not separately execute uv 0.13.8. The fix
validation below builds the checkout, which reports uv 0.13.8.

With `pandas>=3.1.1`, disabled builds, a configured minimum glibc version of 2.20,
and resolution limited to Linux x86_64, `uv lock` exits 1 and says pandas 3.1.1
has no Linux/x86_64-compatible wheels. The explanation omits glibc and 2.20.
Changing only the configured glibc baseline to 2.28 makes the same project lock
successfully with pandas 3.1.1. Public PyPI metadata confirms that Linux x86_64
wheels exist and carry manylinux 2.24/2.28 tags.

Rejecting wheels that cannot satisfy the configured libc baseline is expected.
The reproduced issue is the incomplete explanation, rather than evidence of an
incorrect unsatisfiable result. The request to also show the available wheels'
minimum glibc requirement is an additional diagnostic improvement.

A focused fix is now present in the checkout: Linux wheel-coverage errors retain
and display the configured libc baseline. The parent regression and directly
related integration coverage pass; see **Fix** below.

## Classification

The observed error names only OS and architecture even though the libc baseline
changes whether the same pandas version is accepted. This supports classifying
astral-sh/uv#23272 as a resolution diagnostic bug. The report does not claim a
regression or provide a last known-good version. Reproduction on uv 0.13.7 and
existing integration snapshots show that the symptom predates the reported
uv 0.13.8 environment; no version boundary was established.

The affected subsystem is universal dependency resolution and artifact coverage
reporting. This evidence does not establish a host-platform detection problem,
an incorrect package selection, or a requirement to remove musl wheels from the
lockfile. No exact duplicate or prior fix was identified in the retained related
issue research below.

## Reproduction

Outcome: **reproducible**.

Environment observed on November 17, 2026:

- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.7/x86_64/uv`;
  `uv --version` reports `uv 0.13.7 (x86_64-unknown-linux-gnu)`.
- Linux x86_64, kernel `7.0.0-1017-azure`, host glibc 2.39.
- CPython 3.12.3, explicitly selected with `--python /usr/bin/python3`.
- Public index `https://pypi.org/simple`; clean subprocess environment, temporary
  HOME/config/data/cache directories, disabled keyring, and no Python downloads.

The minimal fixture directly writes the project state resulting from the
report's `uv init .`, `uv add pandas`, and configuration edit. No package
installation or initial lockfile is necessary to trigger the reported error.
There are no workspace members, dependency groups, or source overrides.

```sh
repro="$(mktemp -d /tmp/uv-23272-XXXXXX)"
cd "$repro"
mkdir home
cat > pyproject.toml <<'TOML'
[project]
name = "libc-diagnostic-repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["pandas>=3.1.1"]

[tool.uv]
no-build = true
preview-features = ["minimum-libc-version"]
minimum-libc-version = { glibc = "2.20" }
environments = ["sys_platform == 'linux' and platform_machine == 'x86_64'"]
TOML
env -i PATH="$PATH" HOME="$repro/home" TMPDIR="$repro" \
  XDG_CONFIG_HOME="$repro/config" XDG_CACHE_HOME="$repro/xdg-cache" \
  XDG_DATA_HOME="$repro/data" UV_CACHE_DIR="$repro/cache" \
  UV_PYTHON_INSTALL_DIR="$repro/python" UV_PYTHON_DOWNLOADS=never \
  UV_NO_MANAGED_PYTHON=1 UV_KEYRING_PROVIDER=disabled UV_NO_PROGRESS=1 \
  LC_ALL=C.UTF-8 \
  uv lock --python /usr/bin/python3 --default-index https://pypi.org/simple
```

Observed exit code: 1. After the interpreter-selection line, stderr is exactly:

```text
error: No solution found when resolving dependencies for split (markers: platform_machine == 'x86_64' and sys_platform == 'linux')
  cause: Because pandas==3.1.1 has no `platform_machine == 'x86_64' and sys_platform == 'linux'`-compatible wheels and only pandas<=3.1.1 is available, we can conclude that pandas>=3.1.1 cannot be used.
         And because your project depends on pandas>=3.1.1, we can conclude that your project's requirements are unsatisfiable.
```

No `uv.lock` is created. The diagnostic matches the report and contains neither
the configured glibc 2.20 constraint nor an explanation of the wheels' libc
requirements. Expected: identify the configured libc baseline in the failure;
the reporter additionally requests the minimum supported glibc version.

Control: in a separate temporary project, use the same fixture, interpreter,
index, and command with only `glibc = "2.20"` changed to `glibc = "2.28"`.
Observed exit code: 0, `Resolved 5 packages`, and a lockfile containing pandas
3.1.1. This confirms by execution that the libc baseline changes the outcome.
The public pandas 3.1.1 release metadata lists, among others:

```text
pandas-3.1.1-cp312-cp312-manylinux_2_24_x86_64.manylinux_2_28_x86_64.whl
pandas-3.1.1-cp312-cp312-musllinux_1_2_x86_64.whl
```

The manylinux wheel advertises a glibc baseline newer than 2.20; the musllinux
wheel does not provide glibc coverage. No attempt was made to find the lowest
baseline that resolves the entire dependency tree.

The parent regression from astral-sh/uv-dev#3130 updates
`minimum_libc_no_compatible_version` in
`crates/uv/tests/lock/minimum_libc.rs` to use a local
`py3-none-manylinux_2_28_x86_64` wheel, the report's `environments` setting,
disabled builds, and glibc 2.20. It originally snapshots the same incomplete
diagnostic. It also verifies that concrete-target compilation for manylinux 2.28
succeeds and that raising only the baseline to 2.28 permits locking.

Other directly relevant existing tests in that module include
`minimum_libc_allows_sdist_fallback` (usable sdist followed by disabled builds),
`minimum_libc_direct_url` (independent direct-wheel URL validation with glibc and
musl requirements), and `minimum_libc_switch_families` (both libc families).
Their original failure snapshots likewise omit the baseline. The follow-up fix
updates these snapshots; execution and round-trip validation are recorded below.
The tests use the module's existing `test-python` and `test-universal` gates.

The original live pandas reproduction stored its fixtures and public wheel
filenames under `/tmp/uv-23272-hrbnc68g`. The self-contained commands above do not
depend on those artifacts. Fix-validation logs are at `/tmp/uv-libc-baseline.log`,
`/tmp/uv-libc-red.log`, `/tmp/uv-libc-fixed.log`, and
`/tmp/uv-libc-platform-control.log`.

## Draft response

Reproduced the omitted libc constraint with the reported pandas configuration.
The fix includes the configured glibc and/or musl baseline in Linux wheel
compatibility errors, including failures for direct wheel URLs. The parent
regression now requires the glibc 2.20 explanation, and the related minimum-libc
integration tests pass without changing wheel selection or lockfile behavior.

## Related

- astral-sh/uv-dev#3130 — Parent regression pull request. Supplies the retained local-wheel regression in `crates/uv/tests/lock/minimum_libc.rs`; this fix updates its expected error while retaining both successful controls.
- astral-sh/uv#21651 — Add minimum libc versions for universal resolution (pull request, merged). Introduced minimum-libc-version for universal resolution on September 18, 2026. Its tests already emit the reported marker-only missing-wheel explanation when the configured glibc baseline is unsatisfied. Establishes the feature's origin and intended compatibility rules; it did not fix the diagnostic.
- astral-sh/uv#10581 — Hint user towards `--python-platform linux` targeting glibc 2.17 (issue, open). The linked maintainer discussion explicitly proposes explaining a glibc version mismatch. It concerns the implicit glibc 2.17 baseline of a concrete --python-platform target; this report concerns an explicit minimum-libc-version constraint omitted from a universal-resolution marker diagnostic. Related messaging work, but not the same failure path.
- astral-sh/uv#18169 — Wheel availability hint does not include musllinux tag in alpine image (issue, open). Another confirmed gap in communicating libc compatibility: a maintainer explains that Alpine requires musllinux and questions the missing hint. It concerns install-time musl-versus-glibc platform hints, whereas this report concerns a configured glibc version during uv lock. Its linked proposed fix, astral-sh/uv#18341, was closed without merging.
- astral-sh/uv#12597 — Markers for Glibc/manylinux version (issue, open). Tracks glibc-aware backtracking in project resolution and led to astral-sh/uv#21651. The original request was to select compatible package versions on older glibc systems; this report already uses that capability and identifies an incomplete explanation when no version satisfies it.

## Supporting evidence

- `docs/concepts/resolution.md`, “Minimum libc version”, explains that markers
  cannot express libc implementation/version. A manylinux 2.17 wheel can cover
  glibc 2.31, but a manylinux 2.34 wheel cannot. Musl wheels are retained without
  satisfying required glibc coverage; a usable source distribution can satisfy
  coverage when builds are enabled.
- `crates/uv-resolver/src/prioritized_distribution.rs`, `implied_libc_markers`,
  computes artifact coverage with `minimum_libc_version.platform_coverage(tag)`.
  `crates/uv-resolver/src/resolver/mod.rs`, `fork_version_registry`, reports
  unsuccessful coverage using `IncompatibleWheel::MissingPlatform`. The direct
  wheel URL path in the same module constructs that diagnostic independently.
- Before the fix, `MissingPlatform` stored only a `MarkerTree` and its formatter
  printed only the known platform or marker, although artifact coverage had
  already applied `minimum-libc-version`. Both producers now retain the optional
  baseline, which the formatter includes for Linux coverage failures. The
  pre-fix output, focused failing snapshot, and passing corrected snapshots
  confirm this diagnostic-context omission.
- Prior related-issue research records that merged astral-sh/uv#21651 introduced
  the feature and the same marker-only snapshots on September 18, 2026. A
  review reply confirms that older manylinux baselines satisfy newer configured
  glibc floors and that retaining additional wheels is intentional.

## Candidates ruled out as duplicates

astral-sh/uv#22913 and its open implementation astral-sh/uv#22942 concern choosing
a Linux libc family and excluding musl artifacts. Maintainer comments distinguish
requiring glibc coverage from pruning other wheels; neither tracks the omitted
glibc version in this failure.

astral-sh/uv#23037 and astral-sh/uv#23038 use similar missing-wheel wording but ask
for resolution to tolerate unsupported environments or limit individual
dependencies. This report accepts an unsatisfiable resolution and challenges its
explanation.

Closed astral-sh/uv#19566 concerns universal locking being independent of the
current host; its maintainer response explains that distinction. It does not
establish a previous fix for libc-aware error text. Closed astral-sh/uv#22716
concerns a dependency genuinely lacking Windows wheels, rather than an omitted
libc condition.

Historical merged astral-sh/uv#10739 adds install-time tag hints, and merged
astral-sh/uv#15484 validates already-installed wheels after a platform change.
Neither fixes this universal-resolution diagnostic. Closed, unmerged
astral-sh/uv#18341 proposed a musllinux platform hint for astral-sh/uv#18169.
Closed, unmerged astral-sh/uv#21671 proposed a different libc configuration
design. None establishes a regression of a previously corrected message.

## Fix

Outcome: **fixed** in the checkout; no commit or GitHub changes were made.

The confirmed cause is the loss of diagnostic context between libc-aware
artifact coverage and `IncompatibleWheel::MissingPlatform`: both registry
resolution and direct wheel URL resolution constructed the incompatibility with
only its PEP 508 marker. Markers cannot represent libc versions, so the rendered
error could not explain the configured baseline.

The fix passes `Option<MinimumLibcVersion>` through both producers and appends,
for example, `at the configured libc baseline (glibc 2.20)` to Linux coverage
errors. `MinimumLibcVersion` implements display for glibc, musl, or both families.
Unset or empty libc requirements and markers disjoint from Linux retain their
existing diagnostic wording. Artifact eligibility, resolver choices, serialized
configuration, and lockfile formats are unchanged. Reporting the minimum libc
version advertised by every available wheel remains outside this focused fix.

The parent regression remains in `crates/uv/tests/lock/minimum_libc.rs` and now
requires the baseline in its failure snapshot. Its concrete-target compile and
raised-baseline success checks remain intact. Existing direct-URL, dual-libc,
and disabled-sdist snapshots were corrected for demonstrated instances of the
same omission. No new tests or unrelated fixtures were added. Neighboring
`crates/uv/tests/pip_compile/pip_compile.rs` required-environment fixtures were
also inspected; they configure no libc baseline and their marker-only errors do
not contradict this change. Lockfile write/validation paths already persist and
compare libc configuration, so they need no production changes.

Validation used debug-profile end-to-end tests:

- Before production changes, `cargo test -p uv --test lock minimum_libc::`
  passed all 9 tests, including the parent snapshot of the undesirable output.
- After changing only the parent expectation,
  `cargo test -p uv --test lock minimum_libc::minimum_libc_no_compatible_version -- --exact`
  failed precisely because stderr lacked the configured glibc 2.20 baseline.
- After the fix, `cargo test -p uv --test lock minimum_libc::` passed all 9
  tests. This includes registry and direct-URL failures, glibc/musl requirements,
  source-distribution fallback, lockfile invalidation and re-read checks, frozen
  export/sync round trips, retained wheel hashes, and concrete-target compile.
- `cargo test -p uv --test lock lock::lock_required_environment_macos_release -- --exact`
  passed as a non-libc platform-diagnostic control.
- `RUSTUP_TOOLCHAIN=stable cargo fmt --all` and its `-- --check` invocation
  succeeded, as did `git diff --check`. The pinned Rust toolchain lacks rustfmt,
  so formatting used the already-installed stable formatter.

Only three affected production files and the original parent integration-test
file are modified. No dependency, manifest, lockfile, workflow, or unrelated
repository file was changed.

## Search scope and validation

The retained related-issue context came from prior read-only GitHub research
covering open and closed issues and open, closed, and merged pull requests for
minimum-libc-version, compatible wheels, no-build, glibc, and diagnostic wording.
It included candidate bodies, comments, linked reviews, and source snapshots.
Those GitHub searches were not repeated during reproduction or the fix.

The report, entire persisted handoff, related JSON context, parent regression,
repository guidance, relevant resolver code, documentation, release notes, and
neighboring integration tests were read. The initial live reproduction used the
installed uv against public PyPI in temporary directories. The fix was validated
with the checkout's debug build and focused tests as detailed above. Untrusted
issue and fixture contents were treated as evidence, not instructions. No
credentials were inspected, and no GitHub state or Git configuration was changed.
Only this README was modified within the issue-context directory.

Pull request: https://github.com/astral-sh/uv-dev/pull/3131
