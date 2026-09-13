# ZIP extraction rejects DEFLATE-compressed empty directory with data descriptor

Issue: astral-sh/uv#21644

Classification: bug

## Summary

The failure is reproducible with installed uv 0.12.13. A minimal valid wheel containing an explicit
empty `META-INF/` entry compressed with DEFLATE fails when the entry uses a data descriptor. An
otherwise equivalent archive without descriptors installs successfully. The observed error exactly
matches the report: `Bad compressed size (got 00000000, expected 00000002) for file: META-INF`.
The issue additionally reports the same failure for an official `ibapi` direct-URL source archive
with a `subdirectory` fragment; that external archive was not needed for the targeted reproduction.

The root cause was confined to the streaming extractor's directory branch: it recorded a compressed
size of zero without consuming and measuring the entry's DEFLATE stream, then compared that zero
with the descriptor's compressed size. The fix consumes the checked directory reader and validates
its actual CRC and sizes, so an empty directory's nonzero compressed representation is accepted.

No existing open issue or pull request tracks this exact empty-directory failure. The closest
history is a prior false rejection of data-descriptor ZIPs in astral-sh/uv#12677, its fix in
astral-sh/uv#12722, and the comprehensive streaming ZIP validation added by astral-sh/uv#15136.

## Reproduction

Outcome: **reproducible**.

The reproduction ran on Linux 6.17.0-1022-azure x86_64 with installed `uv 0.12.13
(x86_64-unknown-linux-gnu)` and CPython 3.12.3. All archives, targets, and the uv cache were created
under a fresh temporary directory and removed afterward.

Python 3.12's `zipfile.ZipFile` was used to construct two valid `fixture-0.0.1-py3-none-any.whl`
archives with identical members. Both included an explicit empty `META-INF/` directory using
DEFLATE. The descriptor archive was written to a non-seekable in-memory stream, while the control
was written to a seekable stream. `ZipFile.testzip()` returned `None` for both. In both central
directories, `META-INF/` had compressed size 2 and uncompressed size 0; only the descriptor
archive had general-purpose flag bit 3 set.

Each archive was served from a loopback HTTP server and installed with this command shape (using
separate target directories):

```console
UV_CACHE_DIR=/tmp/uv-21644/cache uv pip install --no-cache \
  --target /tmp/uv-21644/target \
  http://127.0.0.1:<port>/<variant>/fixture-0.0.1-py3-none-any.whl
```

The descriptor variant exited 1 before installation with:

```text
Failed to extract archive: fixture-0.0.1-py3-none-any.whl
Bad compressed size (got 00000000, expected 00000002) for file: META-INF
```

The no-descriptor control exited 0, reported `Installed 1 package`, and placed `fixture.py` in the
target. This isolates the descriptor flag as the relevant difference and reproduces the reported
ZIP extraction behavior independently of package building or the external `ibapi` archive.

The parent regression in `crates/uv/tests/pip_install/pip_install.rs` now covers this exact
combination. The neighboring `crates/uv/tests/build/extract.rs::malo_accept_data_descriptor` and
`malo_accept_deflate` tests stream external fixtures but do not identify a DEFLATE-compressed empty
directory with a data descriptor. Rejection tests in that module cover malformed descriptor CRC and
size fields for regular entries.

## Fix

Outcome: **fixed**.

The parent regression in `crates/uv/tests/pip_install/pip_install.rs` now requires the valid
descriptor wheel to install successfully. Before the production change, that desired snapshot
failed with the reported zero-versus-two compressed-size error.

The streaming ZIP extractor in `crates/uv-extract/src/stream.rs` now consumes directory entry
payloads through the checked entry reader. It verifies that the decompressed payload and CRC remain
empty, records the reader's actual compressed byte count, and compares that measurement with local
header and data-descriptor metadata. This accepts the two-byte DEFLATE representation without
copying descriptor metadata or weakening malformed-size validation. The seekable extractor is a
separate central-directory-based implementation and did not contain the streaming accounting bug;
the content-addressed streaming mode uses the corrected branch and also passes the regression.

Focused debug-profile validation passed for the updated pip-install regression in normal and
content-addressed preview modes. Existing streamed data-descriptor and ZIP64 descriptor acceptance
tests passed, as did the malformed descriptor compressed-size rejection test. Focused clippy for
`uv-extract`, Rust formatting, and `git diff --check` also passed.

## Draft response

Thanks for the self-contained reproduction. I reproduced the exact error with uv 0.12.13 using a
minimal valid wheel, and the equivalent no-descriptor control installed successfully. The streaming
extractor now consumes directory entries and uses the checked reader's actual compressed byte count
for descriptor validation while continuing to require an empty payload and zero CRC.

The updated integration regression confirms that the descriptor archive now installs. Existing
descriptor conformance tests continue to accept valid regular and ZIP64 forms and reject a malformed
compressed size.

## Classification

This is a bug rather than an enhancement or question because the targeted reproduction shows uv
rejecting a valid ZIP representation accepted by Python's ZIP integrity check, while the equivalent
no-descriptor archive installs. Before the fix, source inspection confirmed the observed mismatch:
directory entries received a computed compressed size of zero, while descriptor validation compared
that value with the descriptor's recorded two-byte compressed size.

This is not a duplicate. No open issue or pull request covers the directory-specific size mismatch.
The earlier data-descriptor reports cover broader missing support or CRC placeholder handling, and
astral-sh/uv#15136 is the merged hardening change that introduced the relevant validation rather
than an existing tracker for this regression.

## Related

- astral-sh/uv#12677 (closed issue), “UV fails installing a python library with CRC mismatch” — The
  closest prior false-positive validation report. A valid data-descriptor wheel was rejected because
  uv treated an inline zero placeholder as authoritative CRC data. It differs from astral-sh/uv#21644
  because it concerns a regular file's CRC rather than an empty directory's compressed size.
- astral-sh/uv#12722 (merged pull request), “only warn if CRC appears to be missing” — Fixed the
  earlier descriptor-related CRC false positive by recognizing zero placeholder metadata. It does
  not address compressed-size accounting for directory entries.
- astral-sh/uv#15136 (merged pull request), “Harden ZIP streaming to reject repeated entries and
  other malformed ZIP files” — Added explicit validation of descriptor CRC and sizes. The
  directory branch supplied zero as its computed compressed size to that validation before this
  fix.

## Search evidence

Searches covered the exact error, zero-versus-two size fragments, `META-INF`, `ibapi`, and
`ComputedEntry`; literal ZIP, data-descriptor, DEFLATE, and empty-directory terms; and conceptual
streaming, local-header, central-directory, CRC/size-validation, `async_zip`, direct-URL, and
subdirectory-install terminology. Open and closed issues and open, closed, and merged pull requests
were included, with closed fixes inspected for version-specific history.

The generic descriptor-support issue astral-sh/uv#2216 and native-support change
astral-sh/uv#2809 predate and do not cover this directory-only validation regression.
astral-sh/uv#15158 appeared because it names descriptor ZIP conformance tests, but it only concerns
those tests requiring network access. astral-sh/uv#19528 is also in ZIP extraction but concerns
conflicting duplicate entries in a different extraction path. Neither is a close behavioral match.

Pull request: https://github.com/astral-sh/uv-dev/pull/1747
