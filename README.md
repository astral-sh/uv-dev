# 0.13.0: sdists in pre-POSIX (v7) tar format fail with "invalid tar identity"

Issue: astral-sh/uv#22955

Classification: bug

## Summary

A distinct v7 tar compatibility bug following astral-sh/uv#22094. The closest reports, astral-sh/uv#22510 and astral-sh/uv#22492, were fixed by astral-sh/uv#22522 and astral-sh/uv#22516 in uv 0.13.1, but neither fix covers all-zero v7 identities.

The reporter runs `uv venv` followed by `uv pip install --no-cache pypdf2==1.26.0` on Linux x86_64 with Python 3.11. Extraction fails at byte 0 with `invalid tar identity: found [0, 0, 0, 0, 0, 0, 0, 0]`. They report success with uv 0.12.0, failure with 0.13.0 through 0.13.6, and success when `UV_LEGACY_TAR_BACKEND=1` is set.

Independent source and archive inspection confirms the incompatible header and the decoder rejection path. The installation command and version comparison were not rerun.

## Draft response

The PyPDF2 archive's first header has an all-zero identity, which the pinned tar-framing 0.0.15 rejects before extraction policy is applied. This confirms a compatibility bug separate from the hard-link fix in astral-sh/uv#22516 and the user/group-name fix in astral-sh/uv#22522.

You can use `UV_LEGACY_TAR_BACKEND=1 uv pip install --no-cache pypdf2==1.26.0` as a workaround. The next step is to add v7 decoding support in tar-framing/tar-codec and a regression test for these sdists.

## Classification

Source and archive inspection establish a compatibility failure: the published PyPDF2 1.26.0 sdist has an all-zero header identity, and the pinned tar-framing 0.0.15 rejects it. The reporter's 0.12.0 success and 0.13.0–0.13.6 failure align with the documented default-backend switch. Stricter validation documentation does not resolve the compatibility defect. Existing hard-link and uname/gname fixes address different mechanisms; no issue or PR tracking this same regression was found.

This is a regression relative to the previously selected backend, not evidence that the separately fixed hard-link or uname/gname bugs have returned. No earlier fix for this specific v7 failure was found. The new issue can track restoring compatibility.

## Related

- astral-sh/uv#22094 — Pull request, merged: Use tar-codec by default. Switched archive extraction to tar-codec in uv 0.13.0 and introduced UV_LEGACY_TAR_BACKEND. Establishes the backend transition matching this regression's timing; it does not track a v7 compatibility fix.
- astral-sh/uv#22510 — Issue, closed: 0.13.0: `uv build` rejects sdists whose group name is ≥32 characters ("invalid ustar gname field: missing NUL terminator"). Another 0.13.0 tar-decoder compatibility regression with the same legacy-backend workaround. Its confirmed trigger is a full-length ustar uname/gname without a NUL terminator, rather than a v7 header lacking ustar/GNU identity. Maintainer comments point to astral-sh/uv#22522.
- astral-sh/uv#22522 — Pull request, merged: Bump tar-codec to 0.0.15. Fixed astral-sh/uv#22510 by allowing unterminated uname/gname fields; released in uv 0.13.1. The pinned tar-framing 0.0.15 still rejects all-zero identities, so this fix does not cover v7 archives.
- astral-sh/uv#22492 — Issue, closed: v0.13.0 fails to install pybabel-json==0.2.0. A contemporaneous sdist extraction regression resolved by astral-sh/uv#22516. Its error explicitly rejects hard-link members under extraction policy; the new report fails earlier during header identification.
- astral-sh/uv#22516 — Pull request, merged: Allow hard-links when extracting sdists. The reporter's suggested lead relaxes hard-link extraction policy and shipped in uv 0.13.1. It does not change archive identity parsing, so it is related background rather than a duplicate or fix for this failure.

## Supporting evidence

- `CHANGELOG.md:267` documents the default switch to tar-codec in uv 0.13.0, released October 9, 2026, and the legacy-backend opt-out. `crates/uv-static/src/lib.rs:20` selects TokioTar when `UV_LEGACY_TAR_BACKEND` is true.
- `Cargo.toml:316` and `Cargo.lock:4889` pin tar-codec and tar-framing to 0.0.15. The downloaded [published tar-framing crate](https://static.crates.io/crates/tar-framing/tar-framing-0.0.15.crate) matched the lockfile SHA-256 checksum. Its source revision is `147d8d21524d5610e9488c93a7eac4a1cca7f3ab`.
- In that crate, `src/header.rs:43` defines the identity range as bytes 257 through 264. `src/header.rs:66` and `src/header.rs:71` define only ustar and GNU identities. `src/stream.rs:1332` checks these identities and returns `InvalidIdentity` for any other value before checksum validation and extraction.
- The [PyPI PyPDF2 1.26.0 sdist](https://files.pythonhosted.org/packages/b4/01/68fcc0d43daf4c6bdbc6b33cc3f77bda531c86b174cac56ef0ffdb96faab/PyPDF2-1.26.0.tar.gz) matched the SHA-256 checksum supplied by PyPI metadata. Its first header names `PyPDF2-1.26.0/` and contains eight zero bytes at offsets 257–264. Python's `tarfile` successfully parsed all 32 members. This inspection read the archive in memory without extracting files or executing package code.
- `crates/uv-extract/src/stream.rs:742` permits unterminated uname/gname fields; `crates/uv-extract/src/stream.rs:803` configures hard-link extraction. These settings address the two related bugs but cannot bypass the framing identity check.
- `CHANGELOG.md:194` lists both related fixes under uv 0.13.1, released October 14, 2026. Their inclusion predates the reported failure on uv 0.13.6.
- Maintainer comments on astral-sh/uv#22510 identify the full-width string-field problem and its fixing PR. Comments on astral-sh/uv#22492 identify the hard-link restriction, recommend the legacy backend, and confirm its separate fix.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered invalid tar identity, Invalid tar file, pre-POSIX, v7, V7_FORMAT, pypdf2, and UV_LEGACY_TAR_BACKEND. Conceptual searches covered tar headers, magic, identity, old-style archives, compatibility, and sdist extraction; cause/fix searches separately covered tar-codec, tar-framing, ustar, and the 0.13 transition. Supplemented empty PR search results by filtering titles/bodies of the latest 2,000 PRs across all states and all 626 open PRs. Inspected candidate discussions, reviews, linked fixes, and release notes. Ruled out astral-sh/uv#21641 (network reset), astral-sh/uv#20394 (compressed-stream finalization), astral-sh/uv#19883 (future PEP 625 enforcement), and astral-sh/uv#22425 (closed documentation proposal) as duplicates. No existing v7 identity failure or historical v7 fix was found.

The PEP 625 roadmap's non-pax restriction remains an unchecked future task; it does not establish that this specific regression was fixed or is already tracked. The documentation proposal describes stricter behavior without implementing v7 support. The open compressed-stream PR addresses end-of-stream integrity, whereas this report fails on the first member header.

## Recommended follow-up

Add v7 header decoding in tar-framing/tar-codec, then integrate the updated dependency into uv and cover extraction of a minimal v7 sdist with a focused regression test. Retain the documented legacy-backend workaround while the default decoder is fixed. The report already provides a concrete package and command; additional reporter information is not needed for classification.
