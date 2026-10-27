# 0.13.0: sdists in pre-POSIX (v7) tar format fail with "invalid tar identity"

Issue: astral-sh/uv#22955

Classification: bug

## Summary

Reproduced the reported installation failure on Linux x86_64 with Python 3.11.17 using both the installed uv 0.13.5 and uv 0.13.6. Installing `pypdf2==1.26.0` with `--no-cache` fails at byte 0 with `invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]`. The same installation succeeds with uv 0.12.0 and with `UV_LEGACY_TAR_BACKEND=1` on both tested 0.13 versions.

This is a v7 archive compatibility regression associated with the default-backend change in astral-sh/uv#22094. The hard-link and full-length user/group-name fixes in astral-sh/uv#22516 and astral-sh/uv#22522 address different inputs and do not resolve this failure.

## Classification

Bug: a demonstrated compatibility regression relative to uv 0.12.0 and the legacy tar backend. The published PyPDF2 1.26.0 sdist has an all-zero first-header identity, which the pinned tar-framing 0.0.15 rejects before checksum validation and extraction policy. GNU tar and Python's `tarfile` both successfully list the archive. The runtime failure, version comparison, archive inspection, and decoder implementation agree on the rejected input.

The 0.13.0 release notes document stricter archive validation; reproduction alone does not decide which old archive formats uv should support. The evidence establishes the loss of compatibility and a working fallback. No existing v7 fix or duplicate was found in the recorded issue search. Additional reporter information is not needed to reproduce the behavior.

## Reproduction

**Outcome: reproducible.**

### Environment and isolation

- Linux x86_64, kernel `6.17.0-1022-azure`.
- CPython 3.11.17 at `/opt/hostedtoolcache/Python/3.11.17/x64/bin/python3.11`, matching the reported Python 3.11 minor version.
- Installed executable on `PATH`: `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`; version output `uv 0.13.5 (x86_64-unknown-linux-gnu)`.
- Additional uv 0.12.0 and 0.13.6 Linux x86_64 executables came from public PyPI wheels, with each wheel's SHA-256 verified against PyPI metadata. They were downloaded into the reproduction directory; no installed executable was replaced or checkout binary built.
- All reproduction files, virtual environments, downloads, and caches are under `/tmp/uv-22955-wdy7hk75`. Runs used a clean environment, `UV_NO_CONFIG=1`, `UV_PYTHON_DOWNLOADS=never`, the public PyPI index, disabled keyring access, and temporary cache/config/data directories. No workspace or project configuration is required.
- Each version used a fresh virtual environment. Every installation used `--no-cache`; legacy-backend retries followed failed installations, so no previously installed package or cached built wheel masked extraction.

### Minimal commands

In a temporary directory, with Python 3.11 available and no inherited tar-backend override:

```sh
repro_dir=$(mktemp -d /tmp/uv-22955.XXXXXX)
cd "$repro_dir"
export TMPDIR="$repro_dir" UV_CACHE_DIR="$repro_dir/cache"
export UV_NO_CONFIG=1 UV_PYTHON_DOWNLOADS=never
export UV_DEFAULT_INDEX=https://pypi.org/simple
unset UV_LEGACY_TAR_BACKEND
uv venv --python /opt/hostedtoolcache/Python/3.11.17/x64/bin/python3.11
uv pip install --no-cache pypdf2==1.26.0
```

On uv 0.13.5 and 0.13.6, the install exits 1 with:

```text
error: Failed to download and build `pypdf2==1.26.0`
  cause: Failed to extract archive: pypdf2==1.26.0
  cause: Invalid tar file
  cause: at byte 0: invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]
  cause: invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]
```

The expected behavior reported by the user is successful extraction, build, and installation, as observed with uv 0.12.0. The fallback command succeeds on both tested 0.13 versions:

```sh
UV_LEGACY_TAR_BACKEND=1 uv pip install --no-cache pypdf2==1.26.0
```

| uv version | Tar backend | Observed result |
| --- | --- | --- |
| 0.12.0 | Default | Exit 0; builds and installs `pypdf2==1.26.0` |
| 0.13.5, installed on PATH | Default | Exit 1; exact invalid-identity error above |
| 0.13.5 | Legacy override | Exit 0; builds and installs `pypdf2==1.26.0` |
| 0.13.6 | Default | Exit 1; exact invalid-identity error above |
| 0.13.6 | Legacy override | Exit 0; builds and installs `pypdf2==1.26.0` |

Version comparisons used the same `venv --python ...` and `pip install --no-cache pypdf2==1.26.0` commands, with explicit `--python .venv-0.12.0/bin/python` or `--python .venv-0.13.6/bin/python` for the separate environments. `importlib.metadata.version("PyPDF2")` returned `1.26.0` after every successful installation. Versions 0.13.0 through 0.13.4 were not individually executed; their failure remains a reporter claim, consistent with the documented backend transition.

The temporary directory contains `reproduce.py`, per-command JSON records including stdout/stderr and exit codes, the inspected sdist, and metadata for the downloaded binaries.

### Archive validation

The public `PyPDF2-1.26.0.tar.gz` artifact has SHA-256 `e28f902f2f0a1603ea95ebe21dff311ef09be3d0f0ef29a3e44a932729564385`, matching PyPI metadata. Its first header names `PyPDF2-1.26.0/` and bytes 257–264 are all zero. Python's `tarfile` parsed all 32 members; GNU tar 1.35 listed all 32 members with exit 0 and no stderr. The package's `setup.py` was inspected before the successful installation comparisons.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for PyPDF2, v7/pre-POSIX headers, invalid tar identity, backend selection, and related archive regressions. No direct v7/all-zero-identity regression test was found.

Read the setup and assertions of these nearby tests in `crates/uv/tests/build/build_backend.rs`:

- `build_sdist_with_hard_link`: creates a Python tarfile archive with an explicit hard-link member, installs the sdist offline, and imports the installed module to verify the linked content.
- `build_sdist_with_full_group_name`: creates a PAX archive with a 32-byte group name, explicitly selects the default backend, and snapshots a successful offline installation.
- `build_sdist_with_long_group_name`: creates a PAX archive with a 40-byte group name and overriding PAX metadata, explicitly selects the default backend, and snapshots a successful offline installation.

These cover the two previous fixes, not v7 identities. The module is gated by `test-python` in `crates/uv/tests/build/main.rs`. Tests were inspected, not run or modified; the targeted reproductions used published uv executables.

## Related

- astral-sh/uv#22094 — Pull request, merged: Use tar-codec by default. Switched archive extraction to tar-codec in uv 0.13.0 and introduced UV_LEGACY_TAR_BACKEND. Establishes the backend transition matching this regression's timing; it does not track a v7 compatibility fix.
- astral-sh/uv#22510 — Issue, closed: 0.13.0: `uv build` rejects sdists whose group name is ≥32 characters ("invalid ustar gname field: missing NUL terminator"). Another 0.13.0 tar-decoder compatibility regression with the same legacy-backend workaround. Its confirmed trigger is a full-length ustar uname/gname without a NUL terminator, rather than a v7 header lacking ustar/GNU identity. Maintainer comments point to astral-sh/uv#22522.
- astral-sh/uv#22522 — Pull request, merged: Bump tar-codec to 0.0.15. Fixed astral-sh/uv#22510 by allowing unterminated uname/gname fields; released in uv 0.13.1. The pinned tar-framing 0.0.15 still rejects all-zero identities, so this fix does not cover v7 archives.
- astral-sh/uv#22492 — Issue, closed: v0.13.0 fails to install pybabel-json==0.2.0. A contemporaneous sdist extraction regression resolved by astral-sh/uv#22516. Its error explicitly rejects hard-link members under extraction policy; the new report fails earlier during header identification.
- astral-sh/uv#22516 — Pull request, merged: Allow hard-links when extracting sdists. The reporter's suggested lead relaxes hard-link extraction policy and shipped in uv 0.13.1. It does not change archive identity parsing, so it is related background rather than a duplicate or fix for this failure.

## Supporting evidence

- `CHANGELOG.md` documents the default switch to tar-codec in uv 0.13.0, released October 9, 2026, and the legacy-backend opt-out. The merged diff of astral-sh/uv#22094 confirms the change. `crates/uv-static/src/lib.rs` selects TokioTar when `UV_LEGACY_TAR_BACKEND` is true.
- `Cargo.toml` and `Cargo.lock` pin tar-codec and tar-framing to 0.0.15. The published tar-framing crate was downloaded into the reproduction directory and its SHA-256 matched the lockfile.
- In tar-framing 0.0.15, `src/header.rs` defines identity bytes 257–264 and only ustar/GNU identities. `src/stream.rs`, in `ParsedHeader::try_from_framed`, returns `InvalidIdentity` for other bytes before checksum validation. This matches the observed byte-0 rejection.
- `crates/uv-extract/src/stream.rs` permits unterminated uname/gname fields and configures hard-link extraction. The merged diffs of astral-sh/uv#22522 and astral-sh/uv#22516 confirm those separate fixes. Neither changes framing identity parsing.
- `CHANGELOG.md` lists both fixes under uv 0.13.1, released October 14, 2026; the confirmed 0.13.5 and 0.13.6 failures occur after those fixes.
- Recorded maintainer discussions on astral-sh/uv#22510 identify the full-width string-field problem and its fixing PR. Discussions on astral-sh/uv#22492 identify the hard-link restriction, recommend the legacy backend, and confirm its separate fix.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered invalid tar identity, Invalid tar file, pre-POSIX, v7, V7_FORMAT, pypdf2, and UV_LEGACY_TAR_BACKEND. Conceptual searches covered tar headers, magic, identity, old-style archives, compatibility, and sdist extraction; cause/fix searches separately covered tar-codec, tar-framing, ustar, and the 0.13 transition. Supplemented empty PR search results by filtering titles/bodies of the latest 2,000 PRs across all states and all 626 open PRs. Inspected candidate discussions, reviews, linked fixes, and release notes. Ruled out astral-sh/uv#21641 (network reset), astral-sh/uv#20394 (compressed-stream finalization), astral-sh/uv#19883 (future PEP 625 enforcement), and astral-sh/uv#22425 (closed documentation proposal) as duplicates. No existing v7 identity failure or historical v7 fix was found.

The PEP 625 roadmap's non-pax restriction remains an unchecked future task; it does not establish that this specific regression was fixed or is already tracked. The documentation proposal describes stricter behavior without implementing v7 support. The open compressed-stream PR addresses end-of-stream integrity, whereas this report fails on the first member header.

## Recommended follow-up

Evaluate v7 header decoding in tar-framing/tar-codec, integrate the resulting dependency update into uv, and add a focused regression test for a minimal v7 sdist. The confirmed workaround is `UV_LEGACY_TAR_BACKEND=1`. The existing hard-link and full-length group-name tests provide nearby integration-test patterns but do not exercise this failure.
