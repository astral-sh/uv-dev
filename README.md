# 0.13.0: `uv build` rejects sdists whose group name is ≥32 characters ("invalid ustar gname field: missing NUL terminator")

Issue: astral-sh/uv#22510

Classification: bug

## Summary

No duplicate found. astral-sh/uv#22094 establishes the 0.13.0 backend switch; astral-sh/uv#22492 tracks a separate hard-link rejection.

The reporter observes that `uv build --wheel demo-0.1.0.tar.gz` rejects a CPython `tarfile` PAX-format sdist when its ASCII group name reaches 32 bytes. A 31-byte name succeeds. At 32 bytes, extraction reports `at byte 0: invalid ustar gname field: missing NUL terminator`. At 40 bytes, a PAX group-name record precedes the file header and the same rejection occurs at byte 1024.

The report identifies uv 0.13.0 as the regression boundary and reports successful builds with uv 0.12.24 and, for the 40-byte case, uv 0.13.0 with `UV_LEGACY_TAR_BACKEND=1`. It arose in Azure Pipelines containers with long domain group names and was reported on Linux and Windows with Python 3.13.12. The requested behavior is acceptance of full 32-byte owner/group fields and correct handling of PAX overrides. The demonstrated trigger is an ASCII `gname`; the analogous `uname` case still needs verification.

## Draft response

Your example points to a tar-header compatibility regression following the switch to tar-codec in astral-sh/uv#22094. This is distinct from the hard-link rejection in astral-sh/uv#22492. For now, setting `UV_LEGACY_TAR_BACKEND=1` selects the previous parser. The next step is to verify and fix full-width `uname`/`gname` handling, including entries with PAX overrides; your 32- and 40-byte examples provide concrete regression cases.

## Classification

Rejecting CPython-generated sdists at the 32-byte group-field boundary is a compatibility regression, not a new capability request. An independent in-memory tarfile check confirmed the reported header encoding and PAX override; repository source confirms the default-backend switch and fallback. No existing issue tracks the same failure. The hard-link report and previously fixed PAX streaming bug concern different mechanisms, so neither establishes a duplicate or a recurrence of that historical bug. The uv failure itself was not independently rerun.

The documented move to stricter archive validation does not, by itself, establish that rejection of this CPython-generated input is correct. The backend-switch PR introduces a broader implementation change; it is not an existing discussion of this specific defect. The new issue currently has no comments or cross-referenced follow-up PR in its timeline.

## Related

- astral-sh/uv#22094 — Use tar-codec by default (pull request, merged). Merged October 1 and included in uv 0.13.0: switches archive extraction to tar-codec and adds UV_LEGACY_TAR_BACKEND=1. Confirms the release change and compatibility fallback, but does not track or fix the reported owner/group-field rejection.
- astral-sh/uv#22492 — v0.13.0 fails to install pybabel-json==0.2.0 (issue, open). Also reports sdist extraction breaking between 0.12.24 and 0.13.0, with maintainers recommending the legacy backend. Its explicit error rejects hard-link members, whereas this report rejects a non-NUL-terminated gname field; these are distinct validation failures.

## Supporting evidence

- `CHANGELOG.md:66` documents the default switch from `astral-tokio-tar` to `tar-codec` in uv 0.13.0, released October 9, 2026, and the compatibility fallback. astral-sh/uv#22094 merged October 1, before the reported regression.
- `crates/uv-static/src/lib.rs:12` defines `TarCodec` as the default backend; `TarBackend::from_env` selects `TokioTar` when the legacy setting is true. `crates/uv-static/src/env_vars.rs:428` documents that setting as a compatibility fallback added in 0.13.0.
- `crates/uv-extract/src/stream.rs:729` constructs the tar-codec reader with its default decoding policy plus specific PAX vendor-extension allowances. `crates/uv-extract/src/stream.rs:876` dispatches to the selected decoder. `Cargo.toml:309` and `Cargo.lock:5006` identify tar-codec 0.0.14; the lockfile also records tar-framing 0.0.14. The dependency's precise owner-field validation and PAX application order were not inspected, so the exact implementation change remains to be determined.
- An independent, in-memory CPython `tarfile` check created and read PAX-format archives containing an empty file. The 31-byte group field contained a NUL; the 32-byte field contained no NUL and no PAX group override; the 40-byte case contained no NUL in the ustar group field and had a PAX `gname` override. The ordinary file headers were at byte 0, byte 0, and byte 1024 respectively. Python read back the full intended group names. This confirms the encoding and offsets without requiring a CI environment or a real long-named system group.
- No uv build reproduction was executed. The uv success/failure matrix remains reporter-provided; the independent check validates the archive-format portion only.
- In astral-sh/uv#22492, a maintainer explicitly recommends the legacy parser. Its error is `extraction policy rejected input: hard-link members are not allowed`; `tar_extract_policy` at `crates/uv-extract/src/stream.rs:791` explicitly retains hard-link rejection. That differs from the reported field-decoding error.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using exact error fragments, gname, uname, ustar, PAX, tarfile, owner/group names, header length, and null termination; separately searched tar-codec, tar-framing, UV_LEGACY_TAR_BACKEND, and 0.13.0 for backend changes and fixes. PR keyword searches omitted even the known backend-switch PR, so also reviewed 600 recent PR listings and inspected relevant PRs directly. Read candidate discussions and followed historical fix references. Ruled out astral-sh/uv#5450 and its fix astral-sh/tokio-tar#40: lost PAX filename data during pending reads, fixed in uv 0.5.30, differs from this deterministic header rejection. Also ruled out astral-sh/uv#12762 (writing long paths), astral-sh/uv#21641 (download connection resets), and astral-sh/uv#19883 (format-conformance roadmap). No matching existing report or verified fix was found.

The historical PAX discussion was followed through astral-sh/tokio-tar#40 and astral-sh/tokio-tar#1. Those changes retain partially read extension data across pending reads; they do not address NUL termination of owner/group fields. astral-sh/uv#22492 also references astral-sh/uv#11223, an older hard-link path-resolution failure, which likewise does not match this report.

Other inspected PRs were astral-sh/uv#22143, a merged code-sharing change for archive readers, and astral-sh/uv#22425, a documentation proposal closed without merging. Neither implements a fix for this error. The PEP 625 roadmap and astral-sh/uv#16911 concern archive formats, naming, and compression support; this reproduction already uses a gzip-compressed PAX archive.

## Recommended next step

Validate the supplied 32-byte and PAX-override cases against the current tar-codec decoder, check the analogous `uname` boundary, and determine the required validation change. Add focused regression coverage for full-width fields and PAX overrides when implementing the fix. The existing `build_sdist_with_long_path` test at `crates/uv/tests/build/build_backend.rs:544` exercises archive creation and path extensions, not owner/group-field decoding. The documented legacy-backend setting is the immediate workaround; the reporter also notes that building a wheel directly from the source tree avoids the sdist extraction path.
