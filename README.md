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

The reporter uses Linux 7.0.0-1017-azure x86_64, uv 0.13.0, and Python 3.14.8;
their project includes the dependency through `fm-data-api:dev==4.7.75`. The full
200-package project was not supplied or recreated. Its other dependencies are
not needed to reproduce this extraction failure.

## Classification

Retain the installation compatibility regression classification: a published
package that installs with uv 0.12.13 fails with uv 0.13.0's default backend.
This is an observed compatibility loss, while hard-link rejection itself is an
explicitly documented breaking change. Reproduction does not establish that the
policy is unintended or that accepting these archives is the desired fix.

The historical same-package failure in astral-sh/uv#11223 was fixed by
astral-sh/uv#11221 in uv 0.5.28. That failure involved incorrect resolution of a
hard-link target; this failure rejects hard-link members outright. The merged
backend change explains the current behavior and is not a compatibility fix.
No separate open tracker or fix release was identified in the related-issue
search.

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
for the package, hard-link archive handling, and backend selection. No current
test directly covering this package or its hard-link rejection was found.
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

No repository test suite was run and no tests were added. The result comes
from executing the published uv binaries against the published package. The
reporter's exact kernel and full project were not tested; the documented
legacy-backend workaround succeeded for the minimal fixture, and remains
unverified for the full project.

## Related

- astral-sh/uv#22094 — Use tar-codec by default (pull request, merged). Changed the default tar backend for uv 0.13.0. Its release notes explicitly describe rejection of previously accepted hard-link archives and document UV_LEGACY_TAR_BACKEND=1. The current extraction policy confirms hard-link rejection, matching the reported error.
- astral-sh/uv#11223 — v0.5.27 fails to install pybabel-json==0.2.0 (issue, closed). The same package version previously failed during hard-link extraction. A maintainer confirmed that a fix had merged; it shipped in uv 0.5.28. That failure involved incorrect link-target resolution, whereas this report shows explicit rejection by the new backend.
- astral-sh/uv#11221 — Fix hardlinks in tar unpacking (pull request, merged). Historical fix released in uv 0.5.28 for hard-link targets incorrectly resolved relative to the member's parent directory. Establishes that hard-link extraction support had been restored before the uv 0.13.0 backend change.

## Supporting evidence

- `CHANGELOG.md`, uv 0.13.0, “Use tar-codec for tar archives by default,”
  attributes the change to astral-sh/uv#22094 and explicitly documents
  hard-link rejection and `UV_LEGACY_TAR_BACKEND=1`.
- astral-sh/uv#22094 merged on October 1, 2026. Its body describes the default
  change for extraction, source-distribution creation, and publishing metadata,
  with the legacy-backend opt-out.
- `crates/uv-extract/src/stream.rs::tar_extract_policy` keeps tar-codec's
  default hard-link rejection. `crates/uv-static/src/lib.rs::TarBackend::from_env`
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
PR listings and changelog references supplemented keyword searches. No separate
open tracker for this compatibility regression was identified.

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
astral-sh/uv#11221. The workaround can be recommended with the qualification
that the reporter's full project has not been tested.
