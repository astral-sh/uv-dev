# 0.13.0: sdists in pre-POSIX (v7) tar format fail with "invalid tar identity"

Issue: astral-sh/uv#22955

Classification: bug

## Summary

Reproduced the reported installation failure on Linux x86_64 with Python 3.11.17 using both the installed uv 0.13.5 and uv 0.13.6. Installing `pypdf2==1.26.0` with `--no-cache` fails at byte 0 with `invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]`. The same installation succeeds with uv 0.12.0 and with `UV_LEGACY_TAR_BACKEND=1` on both tested 0.13 versions.

This is a v7 archive compatibility regression associated with the default-backend change in astral-sh/uv#22094. The hard-link and full-length user/group-name fixes in astral-sh/uv#22516 and astral-sh/uv#22522 address different inputs and do not resolve this failure. A maintainer identifies the archive as nonconforming to the current sdist specification and raises security concerns about adding v7 support; restoring compatibility is not an agreed implementation plan.

## Classification

Bug: a demonstrated compatibility regression relative to uv 0.12.0 and the legacy tar backend. The published PyPDF2 1.26.0 sdist has an all-zero first-header identity, which the pinned tar-framing 0.0.15 rejects before checksum validation and extraction policy. GNU tar and Python's `tarfile` both successfully list the archive. The runtime failure, version comparison, archive inspection, and decoder implementation agree on the rejected input.

The 0.13.0 release notes document stricter archive validation; reproduction alone does not decide which old archive formats uv should support. The evidence establishes the loss of compatibility and a working fallback. No existing v7 fix or duplicate was found in the recorded issue search. Additional reporter information is not needed to reproduce the behavior.

## Maintainer assessment and support policy

In the October 27, 2026 discussion on astral-sh/uv#22955 (comment ID 6447485795), maintainer woodruffw identifies the sdist as invalid under the [current source distribution specification](https://packaging.python.org/en/latest/specifications/source-distribution-format/), citing its requirement for POSIX.1-2001 pax tar archives. Successful reads by GNU tar and Python's `tarfile` establish interoperability with those readers, not compliance with that sdist requirement.

The maintainer explains that tar-codec also accepts GNU-format archives because Python's `tarfile` historically defaulted to that format. They qualify their understanding that v7 was never Python's default. They describe v7 as defined by the original UNIX implementation rather than a specification and raise the difficulty of implementing it securely while avoiding differing interpretations across archive readers. These are the maintainer's compatibility and security assessments; the reproduction does not independently establish the historical claim or demonstrate an exploit.

The existing bug classification records the demonstrated loss of compatibility. The comment does not announce a final support decision, a fix, or a commitment to implement v7. The next decision is whether uv should support this legacy input despite its nonconformance and, if so, what precisely bounded decoding behavior is acceptable.

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

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for PyPDF2, v7/pre-POSIX headers, invalid tar identity, backend selection, and related archive regressions. The initial search found no direct v7/all-zero-identity regression test. The checkout now includes `build_sdist_with_v7_tar` in `crates/uv/tests/build/build_backend.rs`, added by astral-sh/uv-dev#3000. It constructs a minimal v7 sdist, snapshots the default backend rejection, and verifies that the legacy backend installs it successfully.

Read the setup and assertions of these nearby tests in `crates/uv/tests/build/build_backend.rs`:

- `build_sdist_with_hard_link`: creates a Python tarfile archive with an explicit hard-link member, installs the sdist offline, and imports the installed module to verify the linked content.
- `build_sdist_with_full_group_name`: creates a PAX archive with a 32-byte group name, explicitly selects the default backend, and snapshots a successful offline installation.
- `build_sdist_with_long_group_name`: creates a PAX archive with a 40-byte group name and overriding PAX metadata, explicitly selects the default backend, and snapshots a successful offline installation.

These cover the two previous fixes, not v7 identities. The module is gated by `test-python` in `crates/uv/tests/build/main.rs`. The initial reproduction used published uv executables. The subsequent fix investigation and parent-test validation are recorded below.

## Related

- astral-sh/uv-dev#3000 — Parent regression pull request: adds `build_sdist_with_v7_tar`, asserting the default-backend failure and legacy-backend success.

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

Clarify the intended support policy for legacy v7 sdists in light of the maintainer's specification and security concerns before implementing decoder changes or changing the parent regression to expect success. If maintainers choose to support v7, define the accepted header semantics and validation requirements in tar-framing/tar-codec, integrate the resulting dependency update into uv, and cover both installation and the independently demonstrated publishing metadata reader. If rejection remains intentional, document the compatibility boundary and assess whether the error needs clearer format-specific guidance. The confirmed workaround is `UV_LEGACY_TAR_BACKEND=1`; the comment does not establish any guarantee about its long-term availability. The existing hard-link and full-length group-name tests provide nearby integration-test patterns but do not exercise this failure.

## Fix

**Outcome: not fixed within the permitted change scope.**

The confirmed rejection is in the external `tar-framing` 0.0.15 dependency: `ParsedHeader::try_from_framed` accepts only ustar and GNU identities. `tar-codec` 0.0.15 wraps that reader and exposes no v7 compatibility setting in `DecodePolicy` or its framing policy. uv's extraction policy is reached only after this identity check. Supporting v7 in the decoder requires changes in astral-codec and a dependency update, outside the permitted source/test paths and the prohibition on manifest/lockfile changes.

Automatically selecting the legacy backend based on archive headers would introduce a new backend-selection and validation policy. Rewriting archive headers would duplicate framing logic inside uv. Neither is a small correction to the existing decoder, so no such workaround was introduced.

The review also covered `uv-publish`'s independent `source_dist_pkg_info_tar_codec` reader, uv-build-backend's GNU/PAX writers, and neighboring hard-link, full/long group-name, long-path, and publishing dry-run tests. The GNU/PAX fixtures do not encode an assumption that contradicts v7 support. No producer failure was demonstrated.

A separate metadata-consumer failure was demonstrated with the installed uv 0.13.5: a temporary `foo-1.0.0.tar.gz` containing a valid v7 `foo-1.0.0/PKG-INFO` member fails `uv publish --dry-run --publish-url http://127.0.0.1:9/upload /tmp/uv-v7-fix-96i20yhr/foo-1.0.0.tar.gz` with the same byte-0 all-zero-identity error (exit 1). Setting `UV_LEGACY_TAR_BACKEND=1` makes that command succeed (exit 0). No upload was performed. The initial attempt with `--offline` was rejected because publishing does not support offline mode; it did not exercise archive decoding. The fixture and output records remain in that temporary directory.

### Focused validation and final state

- `cargo test -p uv --test build build_backend::build_sdist_with_v7_tar -- --exact` passed in the debug/test profile: one test passed, 230 filtered out. This verifies that the original parent test asserts the default-backend failure and the successful legacy fallback.
- Temporarily changing only the default-backend snapshot to expect successful installation, then running `INSTA_UPDATE=no cargo test -p uv --test build build_backend::build_sdist_with_v7_tar -- --exact`, failed as expected: actual exit 1 with `at byte 0: invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]`, rather than expected exit 0. No production changes preceded this check.
- The temporary test edit was restored byte-for-byte. No production fix or test changes are retained, and the checkout is unchanged. No broader suite or release build was run.
- `cargo fmt --all` was attempted but could not run because rustfmt is absent from the Rust 1.99.0 toolchain. There are no retained Rust changes requiring formatting.

The successful baseline and expected failing success assertion confirm the compatibility regression, not a fix or a support-policy decision. If v7 support is approved, native decoder support and its dependency integration remain necessary before the parent test can permanently assert successful installation.
