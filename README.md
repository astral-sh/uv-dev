# `uv python upgrade` doesn't upgrade free-threaded versions

Issue: astral-sh/uv#22831

Classification: bug

## Summary

The reported behavior is reproducible and a focused fix is now present in the
checkout. Download requests retain the installed Python variant, correcting
both bare upgrades and free-threaded reinstalls. Both parent regression tests
assert the corrected behavior and pass, along with five related integration
tests.

Before the fix, with both standard and free-threaded
CPython 3.14.7 installed, bare `uv python upgrade` installs only the newer
standard build and leaves the free-threaded installation at 3.14.7. A second
bare upgrade reports that all versions are current. Explicitly requesting
`uv python upgrade 3.14t` successfully upgrades the free-threaded installation.

The reporter uses uv 0.13.5 on Ubuntu 24.04 under WSL, with Linux x86_64 GNU
managed CPython installations, and expects both variants to upgrade to 3.14.9.
The runner provides uv 0.13.4. Its bundled catalog reproduced the behavior with
3.14.8, and a separate fresh installation using the checkout's download catalog
reproduced the reported 3.14.7-to-3.14.9 transition. The released uv 0.13.5 executable was
not run in that reproduction; the catalog override does not change the uv
executable's version. The subsequent fix validation built and tested the
checkout's uv 0.13.5 in the debug profile.

Removing old interpreters is a separate request tracked by astral-sh/uv#15805.
Retaining the old installations after upgrading is documented behavior because
virtual environments may still reference them.

## Classification

This is a bug in automatic upgrade selection: the documentation promises that
bare `uv python upgrade` upgrades all installed managed Python versions, and the
skipped free-threaded CPython build is available and installs successfully when
explicitly selected. The limitations for upgrading PyPy, GraalPy, and Pyodide do
not explain this CPython behavior.

The pre-fix integration failures confirm the loss of the variant: conversion
from a managed installation constructed a request using
`PythonVariant::Default`, and the automatic upgrade path drops the patch
component and deduplicates requests. The fix uses the installation key's variant
instead. The same conversion also constructs reinstall requests.
The related implementation-preservation fix in astral-sh/uv#16420 addresses
PyPy versus CPython, not free-threaded versus standard CPython. No prior fix or
canonical duplicate for this exact variant issue was identified in the earlier
issue-context review. This reproduction does not establish when the behavior
first appeared; it is already present in the runner's uv 0.13.4.

## Reproduction

**Outcome: reproducible before the fix.** The following records the initial
installed-executable reproduction. All installation, upgrade, and listing
commands completed with exit status 0. Checkout regression tests and fix
validation are recorded in the Fix section.

### Environment and isolation

- Executable on PATH: `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv`;
  `uv --version` reports `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- Runner: Ubuntu 24.04.5 LTS, Linux 7.0.0-1017-azure, x86_64 GNU;
  system Python 3.12.3. The runner is not WSL. The report specifies Ubuntu 24.04,
  WSL kernel 6.6.87.2-microsoft-standard-WSL2, and system Python 3.12.3.
- No project, workspace, virtual environment, dependency groups, or Python
  version file is required. Each fixture starts with only standard and
  free-threaded managed CPython 3.14.7.
- Actual commands ran under `env -i` with isolated managed installations,
  executable links, uv cache, XDG configuration/data/cache directories, and
  temporary files under `/tmp/uv-22831-VcXdH1`. Configuration discovery was
  disabled with `UV_NO_CONFIG=1`; the installed uv on PATH was used throughout.

### Bundled-catalog reproduction

The essential command sequence, with fresh temporary directories, is:

```sh
repro_dir="$(mktemp -d)"
mkdir -p "$repro_dir/tmp" "$repro_dir/bin"
cd "$repro_dir"
env -i PATH="$repro_dir/bin:$PATH" \
  TMPDIR="$repro_dir/tmp" \
  XDG_CONFIG_HOME="$repro_dir/config" \
  XDG_DATA_HOME="$repro_dir/data" \
  XDG_CACHE_HOME="$repro_dir/cache" \
  UV_CACHE_DIR="$repro_dir/cache" \
  UV_PYTHON_INSTALL_DIR="$repro_dir/python" \
  UV_PYTHON_BIN_DIR="$repro_dir/bin" \
  UV_NO_CONFIG=1 UV_NO_PROGRESS=1 \
  bash --noprofile --norc <<'SH'
uv python install 3.14.7 3.14.7t
uv python list --only-downloads 3.14t
uv python upgrade
uv python list --only-installed --managed-python
SH
```

The available-download listing showed
`cpython-3.14.8+freethreaded-linux-x86_64-gnu`. Bare upgrade installed only
`cpython-3.14.8-linux-x86_64-gnu`. The subsequent installed listing showed the
standard executable pointing to 3.14.8 and the free-threaded executable still
pointing to 3.14.7; no 3.14.8 free-threaded installation was created.

### Reported 3.14.9 target and workaround

For a separate fresh fixture, the checkout's
`crates/uv-python-managed/download-metadata.json` was copied to
`/tmp/uv-22831-VcXdH1/download-metadata.json`. The same installed uv 0.13.4
executable was run with `UV_PYTHON_DOWNLOADS_JSON_URL` set to that copied file,
`UV_PYTHON_INSTALL_DIR=/tmp/uv-22831-VcXdH1/reported/python`, and
`UV_PYTHON_BIN_DIR=/tmp/uv-22831-VcXdH1/reported/bin`. The catalog entries were
not edited. This is a metadata override, not execution of uv 0.13.5.

```sh
uv python list --only-downloads 3.14t
uv python install 3.14.7 3.14.7t
uv python upgrade
uv python list --only-installed --managed-python
uv python upgrade
uv python upgrade 3.14t
uv python list --only-installed --managed-python
```

Observed results:

1. The download listing included
   `cpython-3.14.9+freethreaded-linux-x86_64-gnu`.
2. Installation created both 3.14.7 variants.
3. Bare upgrade downloaded and installed only
   `cpython-3.14.9-linux-x86_64-gnu`. Direct execution of the temporary
   `python3.14 --version` and `python3.14t --version` links reported
   `Python 3.14.9` and `Python 3.14.7`, respectively.
4. Repeating bare upgrade printed
   `All versions already on latest supported patch release`, despite the
   available free-threaded patch upgrade.
5. Explicit `uv python upgrade 3.14t` downloaded and installed
   `cpython-3.14.9+freethreaded-linux-x86_64-gnu`. The free-threaded executable
   then reported `Python 3.14.9`. Both old 3.14.7 installations remained.

The explicit selector is a verified workaround in this environment. Neither
WSL nor the released uv 0.13.5 executable was tested directly in that initial
reproduction. These limitations do not
prevent reproducing the skipped-variant behavior with the installed uv.

### Existing test coverage

The initial reproduction reviewed the existing tests without running them.
The parent regression PR subsequently added two tests, now updated and executed
as described in the Fix section.

- `crates/uv/tests/python/python_upgrade.rs`,
  `python_upgrade_without_version`: installs standard CPython 3.11.8, 3.12.8,
  and 3.13.1; explicitly upgrades 3.13; then verifies that bare upgrade installs
  the remaining newer 3.11 and 3.12 patches and subsequently reports a no-op.
  It does not install a free-threaded variant.
- In the same module, `python_upgrade_implementation` installs only PyPy 3.11
  and verifies that bare upgrade does not install CPython. It does not test
  CPython variant preservation. The parent PR added
  `python_upgrade_freethreaded` to cover both variants; it now expects both
  latest patches and executes the upgraded free-threaded interpreter.
- `crates/uv/tests/python/python_install.rs`, `python_install_freethreaded`,
  covers explicit free-threaded installation and selection, executable links,
  and virtual-environment use, not upgrading both installed variants.
- `crates/uv/tests/python/python_install.rs`, `python_reinstall_freethreaded`,
  was also added by the parent PR. It now expects replacement of the
  free-threaded installation and executes its Python binary to verify
  `Py_GIL_DISABLED` is 1.
- `crates/uv-python-types/src/download_request.rs`,
  `upgrade_request_preserves_variant`, starts with an already free-threaded
  request and verifies that removing defaults and the patch yields
  `3.13+freethreaded`. It does not convert a managed installation into a request.
- The Python install and upgrade integration modules are gated by
  `test-python-managed` in `crates/uv/tests/python/main.rs`.

Logs and executable wrappers are retained under `/tmp/uv-22831-VcXdH1`:
`install.log`, `before.log`, `upgrade.log`, and `after.log` record the bundled
catalog run; `reported/available.log`, `reported/install.log`,
`reported/before.log`, `reported/upgrade.log`, `reported/after.log`,
`reported/repeated-upgrade.log`, `reported/explicit-upgrade.log`, and
`reported/after-explicit.log` record the 3.14.9 run. The wrappers are
`uv-isolated` and `uv-reported-catalog`.

## Fix

**Outcome: fixed.** The checkout contains a one-line production fix and updates
to both regression tests from astral-sh/uv-dev#2938. No new test function was
needed.

### Root cause and implementation

`From<&ManagedPythonInstallation> for PythonDownloadRequest` in
`crates/uv-python-managed/src/managed.rs` replaced the installed variant with
`PythonVariant::Default`. Bare upgrade uses this conversion before dropping the
patch and deduplicating requests, so standard and free-threaded installations
collapsed into a standard-only request. The separate reinstall branch in
`crates/uv-python-commands/src/install.rs` also uses this conversion through
`PythonRequest::Key(installation.into())`, causing explicit free-threaded
reinstalls to download a standard build instead.

The conversion now passes `key.variant` to `VersionRequest::from_components`.
Version, implementation, architecture, OS, libc, and prerelease handling remain
unchanged. Inspection confirmed that `only_minor()`, request matching, minor
installation keys, and upgrade selection already retain or compare variants.
Neighboring install, debug/free-threaded selection, list, uninstall, standard
reinstall, and implementation-preservation fixtures were inspected. No other
contradictory expectation requiring correction was found. No additional command
aliases or speculative variant matrix were added.

### Regression evidence and validation

With the production code unchanged, both original parent tests passed while
asserting the undesirable behavior. Their snapshots were then changed to expect
both upgraded variants and a free-threaded replacement on reinstall. Both tests
failed at those snapshots before production code was changed: upgrade installed
only standard 3.14.9, and reinstall added standard 3.14.7 instead of replacing
the free-threaded installation. The failure transcript is
`/tmp/uv-22831-red.log`.

The updated `python_upgrade_freethreaded` also verifies the upgraded executable
reports the latest patch and both subsequent bare and explicit free-threaded
upgrades are no-ops. The updated `python_reinstall_freethreaded` executes the
reinstalled binary and verifies its free-threaded build configuration. Both
remain under the existing `test-python-managed` module gate.

After the production fix, the following debug-profile command passed all seven
selected integration tests (no ignored tests):

```sh
UV_PYTHON_CACHE_DIR=/tmp/uv-22831-test-python-cache INSTA_UPDATE=no \
  cargo test -p uv --test python --features test-python-managed -- \
  python_upgrade::python_upgrade_freethreaded \
  python_upgrade::python_upgrade_without_version \
  python_upgrade::python_upgrade_implementation \
  python_install::python_reinstall \
  python_install::python_install_freethreaded
```

The reinstall filter selects `python_reinstall`, `python_reinstall_patch`, and
`python_reinstall_freethreaded`; the other filters each select one test. This
covers the two corrected parent regressions, interpreter execution, standard
single/multiple/patch reinstalls, bare upgrades across minor versions, explicit
free-threaded install/find/virtual-environment use, and PyPy implementation
preservation. The successful transcript is `/tmp/uv-22831-green.log`.

The initial baseline command encountered the runner's read-only default archive
cache before reaching the regression. Setting `UV_PYTHON_CACHE_DIR` to `/tmp`
resolved that environment limitation for baseline, failing-expectation, and
fixed runs. The pinned Rust 1.99.0 toolchain lacks rustfmt and its installation
is read-only, so formatting used the installed stable toolchain:
`cargo +stable fmt --all` and
`RUSTUP_TOOLCHAIN=stable cargo fmt --all -- --check` both succeeded.
`git diff --check` also passed.

Only `crates/uv-python-managed/src/managed.rs`,
`crates/uv/tests/python/python_upgrade.rs`, and
`crates/uv/tests/python/python_install.rs` were changed in the checkout. No full
suite, release build, Windows/WSL execution, commits, or GitHub changes were
performed.

## Related

- astral-sh/uv-dev#2938 — Parent regression PR. Added the upgrade and reinstall
  regressions; both are retained and updated to require correct variant preservation.

- astral-sh/uv#16420 — Check for matching Python implementation during `uv python upgrade` (pull request, merged). Changed bare upgrades to construct requests from managed installations, the same conversion that replaced the installed variant with PythonVariant::Default before this fix. Merged on 2025-10-23 to fix implementation preservation; its regression test covers PyPy, not free-threaded CPython.
- astral-sh/uv#16416 — `uv python upgrade` installs cpython-3.11 if pypy-3.11 exists (issue, closed). Historical bug in the same automatic upgrade-request construction. Maintainer diagnosis identified lost implementation information; astral-sh/uv#16420 fixed that. This report concerns lost CPython variant information, so it is not a duplicate of the resolved PyPy problem.
- astral-sh/uv#15805 — A way to remove old Python versions (issue, open). Tracks the reporter's secondary cleanup request, including retaining the latest free-threaded versions. Maintainers discuss pruning and existing virtual-environment references. It does not track skipped free-threaded upgrades.
- astral-sh/uv#18770 — Add `--outdated` to `uv python uninstall` and `--uninstall` to `uv python upgrade` (pull request, open). Proposes the cleanup requested in astral-sh/uv#15805, rebasing astral-sh/uv#16018 and adding cleanup after upgrades. It remains unmerged and does not fix the variant loss in automatic upgrade requests.

## Supporting source and release context

- `docs/concepts/python-versions.md`, “Upgrading Python versions,” documents
  bare upgrade as upgrading all installed versions and explains retaining old
  interpreters and respecting explicit patch pins.
- `crates/uv-python-commands/src/install.rs` derives automatic upgrade requests
  through `PythonDownloadRequest::from(installation)`, applies `only_minor()`,
  and deduplicates them in an `IndexSet`. Explicit targets use a separate
  parsing branch.
- `crates/uv-python-managed/src/managed.rs`,
  `From<&ManagedPythonInstallation> for PythonDownloadRequest`, supplied
  `PythonVariant::Default` before the fix and now supplies `key.variant`.
  `crates/uv-python-types/src/request.rs` retains the supplied variant in
  `only_minor()`. The defective conversion made the two installed variants
  become the same standard-CPython upgrade request.
- The earlier issue-context review inspected the same upgrade loop and
  hardcoded variant at tag `0.13.5` through the GitHub contents API. That is
  source evidence for the reported release, separate from the executable
  reproduction on uv 0.13.4.
- `CHANGELOG.md` dates uv 0.13.5 to 2026-10-21 and describes CPython 3.14.9 and
  the updated security builds in astral-sh/uv#22802. Both 3.14.9 variants are
  present in the checkout catalog with build revision `20261021`; the
  free-threaded build was actually downloaded in the explicit-upgrade check.
- Earlier history review found that astral-sh/uv#13954 introduced patch upgrades
  with standard and free-threaded Python support, closing astral-sh/uv#12977.
  The stabilization discussion in astral-sh/uv#16637 was later closed as stable.

The updated parent regressions now cover the managed-installation conversion
through both automatic upgrade and explicit reinstall, including execution of
the resulting free-threaded interpreter.

## Search scope and exclusions

The following records the earlier issue-context search, retained for continuity;
this reproduction did not repeat the GitHub-wide search or change any tracker.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using literal python upgrade/free-threaded/freethreaded, 3.14, 3.14t, and 3.14.9+freethreaded terms; conceptual update, nogil/no-GIL, variant, skip, and managed-interpreter terms; and separate old-version removal, outdated, and prune searches. Searched PythonDownloadRequest and variant-preservation identifiers separately from symptoms. PR search returned no matches, so additionally enumerated all 13,224 PR titles and inspected relevant bodies, comments, reviews, and diffs. Followed historical fix and cleanup references. Ruled out astral-sh/uv#20434 (explicit equality parsing), astral-sh/uv#18795 and astral-sh/uv#19012 (Windows registry listing), astral-sh/uv#21323 (replacement of same-version build revisions), and astral-sh/uv#16675 (non-CPython transparent upgrades). No existing tracker for this exact defect or prior fix of it was found.

Additional inspected candidates were excluded for these reasons:

- astral-sh/uv#18862 concerns reusing a free-threaded environment for an
  unconstrained `3.14` interpreter request. A maintainer explains that
  `3.14+gil` requests the GIL-enabled interpreter. This does not concern
  downloading patch upgrades.
- astral-sh/uv#8189 concerns whether an explicit uninstall request for
  `3.13` should also remove `3.13t`; it is distinct from upgrading every
  installed variant without targets.
- astral-sh/uv#21790 and astral-sh/uv#21794 concern provider-defined and
  optimization build identities. Both were closed without merging in favor of
  another approach; they do not establish a fix for this free-threaded upgrade
  defect.
- astral-sh/uv#15756 discusses documentation and implicit free-threaded
  interpreter selection, not automatic patch upgrade requests.
- astral-sh/uv#16783 concerns an ambiguous message when no managed installations
  are available, not skipping one of two installed variants.
- astral-sh/uv#16018 is the earlier, still-open cleanup proposal. The inspected
  successor astral-sh/uv#18770 rebases it and adds cleanup after upgrades; neither
  resolves variant loss during automatic request construction.

The initial reproduction left the checkout unchanged. The subsequent fix
changes only the three production/integration-test files listed above and this
handoff. Existing user Python installations and GitHub state were not modified.

Pull request: https://github.com/astral-sh/uv-dev/pull/2939
