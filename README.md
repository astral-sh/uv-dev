# v0.13.0 fails to install pybabel-json==0.2.0

Issue: astral-sh/uv#22492

Classification: bug — installation compatibility regression under a documented breaking change

## Summary

Reproduced the reported `uv sync` failure with uv 0.13.0 and CPython 3.14.8 on
Linux x86_64. A minimal project with `pybabel-json==0.2.0` in its default `dev`
dependency group fails with the exact reported diagnostic, including byte offset
25088. The same project and Python interpreter install successfully with uv
0.12.13 and with uv 0.13.0 when `UV_LEGACY_TAR_BACKEND=1` is set. Each case used
its own initially empty cache and virtual environment.

The published source archive contains a hard link from
`PyBabel-json-0.2.0/README.rst` to `PyBabel-json-0.2.0/README` at offset 25088.
The default-backend change in astral-sh/uv#22094 documents rejection of hard-link
archives and the legacy-backend opt-out. The observed failure is therefore
consistent with the documented extraction policy.

The checkout now contains a focused fix enabling tar-codec hard-link extraction
and an updated parent regression requiring successful installation. The updated
regression passes with both backends. This is a local, unreleased change.

The updated report also states that uv 0.12.24 succeeds; the independently tested
baseline here was uv 0.12.13.

The reporter uses Linux 7.0.0-1017-azure x86_64, uv 0.13.0, and Python 3.14.8;
their project includes the dependency through `fm-data-api:dev==4.7.75`. The full
200-package project was not supplied or recreated. Its other dependencies are
not needed to reproduce this extraction failure.

## Classification

Installation compatibility regression: a published
package that installs with uv 0.12.13 fails with uv 0.13.0's default backend.
This is an observed compatibility loss, while hard-link rejection itself is an
explicitly documented breaking change. The focused fix restores installation
compatibility using the backend's supported hard-link opt-in while retaining
its target validation; it does not change parsers or remove other policies.

The historical same-package failure in astral-sh/uv#11223 was fixed by
astral-sh/uv#11221 in uv 0.5.28. That failure involved incorrect resolution of a
hard-link target; this failure rejects hard-link members outright. The merged
backend change explains the current behavior and is not a compatibility fix.
The parent regression is in astral-sh/uv-dev#2601. No released compatibility
fix was identified.

## Reproduction

**Outcome: reproducible.**

### Environment and isolation

- Host: Linux 6.17.0-1022-azure x86_64 GNU/Linux.
- Interpreter used for all three comparisons: CPython 3.14.8, installed under
  the reproduction directory with uv 0.13.0 and `uv python install --no-bin 3.14.8`.
- Baseline: installed executable on `PATH`,
  `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`, reporting
  `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Affected version: `uv 0.13.0 (x86_64-unknown-linux-gnu)`, installed from its
  published PyPI wheel into a temporary target using the installed uv:
  `uv pip install --python /usr/bin/python3 --target /tmp/uv-22492-umrppzp1/uv-0.13.0 --no-deps --only-binary :all: --default-index https://pypi.org/simple uv==0.13.0`.
- Fixtures, downloaded executables, Python, virtual environments, caches, and
  logs are under `/tmp/uv-22492-umrppzp1`. Subprocesses received a clean
  environment with temporary XDG and Python directories, `UV_NO_CONFIG=1`, and
  no inherited index or backend configuration. Sync used
  `UV_PYTHON_DOWNLOADS=never` and an explicit interpreter path.

### Fixture and commands

Each of `baseline`, `default`, and `legacy` contains this `pyproject.toml`:

```toml
[project]
name = "pybabel-json-repro"
version = "0.1.0"
requires-python = ">=3.14"

[dependency-groups]
dev = ["pybabel-json==0.2.0"]
```

This is a single virtual project with the implicit default `dev` group, no
workspace, no source overrides, and no build-system table. All commands use
ordinary, non-frozen `uv sync`. First, uv 0.12.13 generated a lockfile and
installed the dependencies. That lockfile was copied to the two fresh 0.13.0
projects, matching the report's failure after an already-resolved lockfile.
The three explicit sync invocations were equivalent to the following, with
the isolated environment described above:

```sh
reproduction_dir=/tmp/uv-22492-umrppzp1
python_314="$reproduction_dir/python/cpython-3.14.8-linux-x86_64-gnu/bin/python3.14"
uv_013="$reproduction_dir/uv-0.13.0/bin/uv"

UV_CACHE_DIR="$reproduction_dir/cache-baseline" uv sync \
  --directory "$reproduction_dir/baseline" \
  --python "$python_314" --default-index https://pypi.org/simple

cp "$reproduction_dir/baseline/uv.lock" "$reproduction_dir/default/uv.lock"
cp "$reproduction_dir/baseline/uv.lock" "$reproduction_dir/legacy/uv.lock"

UV_CACHE_DIR="$reproduction_dir/cache-default" "$uv_013" sync \
  --directory "$reproduction_dir/default" \
  --python "$python_314" --default-index https://pypi.org/simple

UV_LEGACY_TAR_BACKEND=1 UV_CACHE_DIR="$reproduction_dir/cache-legacy" "$uv_013" sync \
  --directory "$reproduction_dir/legacy" \
  --python "$python_314" --default-index https://pypi.org/simple
```

### Observed results

| uv version | Tar backend selection | Exit | Result |
| --- | --- | --- | --- |
| 0.12.13 | Default | 0 | Built and installed `pybabel-json==0.2.0` and `babel==2.18.0` |
| 0.13.0 | Default | 1 | Rejected a hard-link member at byte 25088 before building |
| 0.13.0 | `UV_LEGACY_TAR_BACKEND=1` | 0 | Built and installed the same package versions |

The failing run produced:

```text
Resolved 3 packages in 0.53ms
Downloading babel (9.7MiB)
error: Failed to download and build `pybabel-json==0.2.0`
  cause: Failed to extract archive: pybabel-json==0.2.0
  cause: Invalid tar file
  cause: at byte 25088: extraction policy rejected input: hard-link members are not allowed

hint: `pybabel-json` (v0.2.0) was included because `pybabel-json-repro:dev` (v0.1.0) depends on `pybabel-json`
```

`importlib.metadata.version` independently confirmed `pybabel-json=0.2.0` and
`babel=2.18.0` in both successful virtual environments. Logs are in
`baseline.log`, `default.log`, `legacy.log`, `baseline-installed.log`, and
`legacy-installed.log` under the reproduction directory.

The PyPI release offers `PyBabel-json-0.2.0.tar.gz` and no wheel. Its SHA-256 is
`204e7fdab0e39946b6793d5584e6927035d12f5813dc6efea24d580e2aa64ea9`, matching
both PyPI metadata and the generated lockfile. Reading the tar headers without
extracting them confirmed the README hard-link member at offset 25088.
Together with the fresh-cache backend comparison, this confirms that the
archive's hard link triggers the policy rejection.

### Existing test coverage and limits

Searched `crates/uv/tests/`, `crates/uv-client/tests/it/`, and `crates/uv-extract/`
for the package, hard-link archive handling, and backend selection. The parent
regression in `crates/uv/tests/sync/sync.rs::sync_sdist_hardlink` now directly
covers this package. It originally asserted default-backend rejection and
legacy-backend success; the updated test requires both to succeed, with a fresh
virtual environment between installs. Its module requires `test-python` and
`test-pypi`; no additional feature gate is needed.
The following related tests were read, including their setup and assertions:

- `crates/uv/tests/build/build.rs::build_with_symlink` creates a relative
  `pyproject.toml` symlink and asserts successful sdist and wheel builds. It
  covers symbolic links, not tar hard-link members.
- `crates/uv/tests/build/build.rs::venv_included_in_sdist` includes a virtual
  environment in a Hatchling sdist and checks unsafe external symbolic-link
  diagnostics and the venv hint for both backends. It does not cover this
  archive-internal hard link. Both build tests require `test-python` and
  `test-pypi`, and are Unix-only.
- `crates/uv/tests/python/python_install.rs::python_install_armv7` checks that
  both backends can install an archive containing duplicate terminfo symlinks;
  its successful assertions do not cover hard links.

The historical astral-sh/uv#11221 patch added `build_with_hardlink` under the
then-current `crates/uv/tests/it/build.rs`; that named test is absent from this
checkout. Current cache and installation hard-link tests concern filesystem
linking, not hard-link entries in source archives.

The release comparison above comes from executing published uv binaries against
the published package. Focused checkout validation is described under Fix; no
new test was added. The reporter's exact kernel and full project were not tested.
The documented legacy-backend workaround succeeded for the minimal fixture and
remains unverified for the full project.

## Fix

**Outcome: fixed in the checkout; not committed or published.**

### Maintainer assessment

After investigating, maintainer woodruffw said in astral-sh/uv#22492 on
October 13, 2026 (comment 6147828972) that the hard-link check could probably
be relaxed, and expressed an intent to keep this older package working.
This tentative direction is consistent with the hard-link opt-in described
below. The comment does not approve the specific checkout implementation or
confirm a merged fix or release.

### Root cause and implementation

The shared streaming tar extractor retained `LinkPolicy::default()`, which
rejects every hard-link member before attempting extraction. This rejects the
published archive's valid README link even though its target was already
extracted. The dependency already implements hard-link extraction, including
archive-relative target resolution and a requirement that the target be a
previously extracted file. No dependency change or alternate parser is needed.

`crates/uv-extract/src/stream.rs::tar_extract_policy` now explicitly enables
hard links. Name validation, target confinement, rejection of pre-existing
symbolic-link targets, and Windows symbolic-link skipping remain in place.
The legacy backend is unchanged.

The parent `sync_sdist_hardlink` test in `crates/uv/tests/sync/sync.rs` now
requires the default backend to build and install `pybabel-json==0.2.0`.
It retains the lockfile setup and the legacy-backend comparison, uses
`--no-cache`, and recreates the virtual environment before the legacy install
so neither successful assertion can be satisfied by an already-installed
package. Only this test file and the extraction policy file are modified.

### Related-path review

Reviewed the shared extractor's source-distribution, build, managed-Python,
and binary-install consumers; their tar paths reuse the same implementation.
Additional command variants would duplicate the parent coverage. Reviewed
`uv_build` source-distribution writers: both backends write regular file members,
so their default-policy unpacking helpers do not encode the hard-link rejection
bug. Publishing reads member payloads without applying this extraction policy,
and no distinct manifestation of this cause was demonstrated there.

The existing `build_with_symlink`, `venv_included_in_sdist`, and
`python_install_armv7` fixtures exercise symbolic links and do not require
changes. No additional test was added for an unproven variant or an unrelated
configuration format.

### Validation

- Verified the unchanged parent test passes while asserting the exact
  hard-link rejection at byte 25088.
- Changed its expected result to a successful install before changing production
  code; the test then failed on that same rejection, confirming the regression.
- With the production fix, `cargo test --locked -p uv --test sync
  sync::sync_sdist_hardlink -- --exact` passes (one test), exercising fresh
  source builds and installs through both backends.
- `cargo test --locked -p uv --test build -- build::build_basic
  build::venv_included_in_sdist --exact` passes (two tests). This verifies the
  source-tree → sdist → wheel round trip and retains unsafe external-symlink
  rejection and its virtual-environment diagnostic under both tar backends.
- Windows cross-compilation passes with warnings denied:
  `cargo xwin clippy --locked -p uv-extract --target x86_64-pc-windows-msvc -- -D warnings`.
  This used cargo-xwin 0.23.1 via `uv tool run`, a temporary Rust 1.99.0
  toolchain, LLVM 18, and SDK/build caches under `/tmp`. It is a compilation
  check, not a Windows runtime test.
- Ran `cargo fmt --all` using a temporary Rust 1.99.0 toolchain because the
  runner's system toolchain lacks rustfmt and is read-only. `git diff --check`
  passes.

No full suite or release-profile build was run. The published-version
reproduction and the focused debug tests establish the package installation
behavior; the complete reporter project remains outside the supplied fixture.

## Related

- astral-sh/uv-dev#2601 — Parent regression pull request. Adds `sync_sdist_hardlink`, which records the default-backend failure and successful legacy fallback. This fix retains that test and changes its default-backend assertion to successful installation.
- astral-sh/uv#22094 — Use tar-codec by default (pull request, merged). Changed the default tar backend for uv 0.13.0. Its release notes explicitly describe rejection of previously accepted hard-link archives and document UV_LEGACY_TAR_BACKEND=1. The released extraction policy rejects hard links, matching the reported error.
- astral-sh/uv#11223 — v0.5.27 fails to install pybabel-json==0.2.0 (issue, closed). The same package version previously failed during hard-link extraction. A maintainer confirmed that a fix had merged; it shipped in uv 0.5.28. That failure involved incorrect link-target resolution, whereas this report shows explicit rejection by the new backend.
- astral-sh/uv#11221 — Fix hardlinks in tar unpacking (pull request, merged). Historical fix released in uv 0.5.28 for hard-link targets incorrectly resolved relative to the member's parent directory. Establishes that hard-link extraction support had been restored before the uv 0.13.0 backend change.

## Supporting evidence

- `CHANGELOG.md`, uv 0.13.0, “Use tar-codec for tar archives by default,”
  attributes the change to astral-sh/uv#22094 and explicitly documents
  hard-link rejection and `UV_LEGACY_TAR_BACKEND=1`.
- astral-sh/uv#22094 merged on October 1, 2026. Its body describes the default
  change for extraction, source-distribution creation, and publishing metadata,
  with the legacy-backend opt-out.
- The released `crates/uv-extract/src/stream.rs::tar_extract_policy` kept
  tar-codec's default hard-link rejection. The checkout fix explicitly enables
  hard links through `LinkPolicy::allow_hard_links(true)`.
  `crates/uv-static/src/lib.rs::TarBackend::from_env`
  selects `TokioTar` when the legacy environment variable is true, and
  `TarCodec` otherwise. This matches the observed comparison.
- astral-sh/uv#22143, merged October 8, 2026, changes archive-reader
  specialization; it does not describe a hard-link policy fix.
- astral-sh/uv#11221 merged February 4, 2025, fixing incorrect hard-link target
  resolution through the tar dependency and adding regression tests.
  `changelogs/0.5.x.md` places the fix in uv 0.5.28.
- astral-sh/uv#11223 describes the earlier same-package failure and was closed
  after the historical fix. Its relation is the source archive's hard link,
  while the immediate rejection mechanism differs.

## Related-issue search context

The related-issue search covered open and closed issues and pull requests for
the package, exact hard-link policy error, invalid tar diagnostics, uv 0.13.0,
hard-link archive handling, tar-codec, and the legacy-backend variable. Recent
PR listings and changelog references supplemented keyword searches. That search
identified no separate open tracker; the supplied parent regression pull request
is recorded above.

The following inspected candidates do not centralize this behavior:

- astral-sh/uv#21641 concerns download connection resets and changing failure
  locations; changing networks resolved the reporter's problem.
- astral-sh/uv#12163 concerns links outside the extraction directory.
- astral-sh/uv#1858 concerns symbolic-link target lookup and Windows privileges.
- astral-sh/uv#21128 concerns the diagnostic hint for an unsafe absolute
  symbolic link in a bundled virtual environment.
- astral-sh/uv#22425 proposed documentation of the same compatibility change
  and fallback; it closed without merging on October 9, 2026.
- astral-sh/uv#22143 changes reader specialization without a hard-link fix.
- astral-sh/uv#11213 is historical context for astral-sh/uv#11221: a cached
  hard-linked file in an sdist exposed the older link-target resolution bug.

## Suggested maintainer response

Confirmed with uv 0.13.0 and Python 3.14.8: the published archive contains a
hard link, which the new default tar backend rejects as documented in the
0.13.0 release notes. `UV_LEGACY_TAR_BACKEND=1 uv sync` succeeds for a minimal
project containing this dependency, as does uv 0.12.13. The earlier failure in
astral-sh/uv#11223 had a different mechanism and was fixed by
astral-sh/uv#11221. The checkout fix restores installation with the default
backend, while the legacy environment variable remains a workaround for the
released version.
The reporter's full project has not been tested.

Pull request: https://github.com/astral-sh/uv-dev/pull/2602
