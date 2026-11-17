# Unclear resolution error when `minimum-libc-version` is involved

Issue: astral-sh/uv#23272

Classification: bug (resolution diagnostic)

## Summary

Reproduced the reported pandas failure and its marker-only explanation using the
installed uv 0.13.7 on Linux x86_64 with CPython 3.12.3 and host glibc 2.39. The
report names uv 0.13.8 and leaves the Python version and platform unspecified;
uv 0.13.8 was not separately executed.

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

Existing integration coverage was read, including setup and assertions:

- `crates/uv/tests/lock/minimum_libc.rs`,
  `minimum_libc_no_compatible_version`: creates CPython 3.12 manylinux 2.34 and
  musllinux 1.2 wheels, requires Linux x86_64 with glibc 2.31, and snapshots a
  failed lock that mentions only the Linux/x86_64 marker. It also verifies that
  concrete-target compilation for manylinux 2.34 succeeds.
- In the same module, `minimum_libc_allows_sdist_fallback`: first locks a package
  with a usable source distribution and a too-new manylinux 2.34 wheel, then
  runs `uv lock --offline --no-build --upgrade` and snapshots the same marker-only
  missing-wheel explanation at a glibc 2.31 baseline.

These tests cover the general omission with synthetic packages and
`required-environments`; the live pandas reproduction above uses the report's
`environments` setting. The existing Rust tests were inspected, not executed.
No tests or source files were changed.

Reproduction artifacts are retained at `/tmp/uv-23272-hrbnc68g`: the failing fixture and
stderr are in `pandas/`, the successful control and lockfile are in
`pandas-glibc-2-28/`, and the public release wheel filenames are in
`pandas-3.1.1-files.json`. All reproduction files and caches are temporary.

## Draft response

Reproduced on uv 0.13.7 with Python 3.12.3: the glibc 2.20 configuration produces
exactly the reported Linux/x86_64-only explanation. Raising the configured
baseline to 2.28 lets pandas 3.1.1 lock successfully. Rejecting incompatible
wheels is expected with builds disabled, but the error omits the libc constraint
that explains their rejection. Existing minimum-libc integration snapshots show
the same diagnostic omission.

## Related

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
  unsuccessful coverage using `IncompatibleWheel::MissingPlatform(marker)`.
- `IncompatibleWheel::MissingPlatform` stores a `MarkerTree`, and its formatter
  in `prioritized_distribution.rs` prints a known platform or marker without a
  libc-specific explanation. This source path is consistent with the observed
  diagnostic; no instrumented trace was used to establish a deeper root cause.
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

## Search scope and validation

The retained related-issue context came from prior read-only GitHub research
covering open and closed issues and open, closed, and merged pull requests for
minimum-libc-version, compatible wheels, no-build, glibc, and diagnostic wording.
It included candidate bodies, comments, linked reviews, and source snapshots.
Those GitHub searches were not repeated during this behavioral reproduction.

This reproduction read the report, the entire existing handoff, repository
guidance, schema, relevant resolver code, documentation, release notes, and
integration test fixtures/assertions. It executed the installed uv against
public PyPI in temporary directories and verified published pandas wheel names.
The issue's contents were treated as evidence, not instructions. No credentials
were inspected, no builds were run, and no GitHub changes were made. Only this
external README was updated. The checkout's existing modified
`agents/codex/config.toml` and untracked `.issue-triage-event.json` were untouched.
