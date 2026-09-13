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

The current streaming extractor supports the reported mechanism. Its directory branch records a
computed CRC and uncompressed size of zero, which are appropriate for an empty directory, but also
records a compressed size of zero without consuming and measuring the entry's DEFLATE stream. The
later data-descriptor validation compares that zero with the descriptor's compressed size. An empty
DEFLATE stream still occupies compressed bytes, so this rejects a supported ZIP representation.

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

No existing integration test covers this exact combination. The closest tests are
`crates/uv/tests/build/extract.rs::malo_accept_data_descriptor` and
`crates/uv/tests/build/extract.rs::malo_accept_deflate`; each streams an external fixture and asserts
successful extraction, but neither test setup or assertion identifies a DEFLATE-compressed empty
directory with a data descriptor. The rejection tests in the same file cover malformed descriptor
CRC and size fields for regular entries.

## Draft response

Thanks for the self-contained reproduction. I reproduced the exact error with uv 0.12.13 using a
minimal valid wheel, and the equivalent no-descriptor control installed successfully. The current
source is consistent with the observation: directory entries are assigned a computed compressed
size of zero, and that value is later compared with the data descriptor, even though the tested
empty DEFLATE stream has a two-byte compressed payload.

The earlier descriptor-related false positive in astral-sh/uv#12677 was fixed by
astral-sh/uv#12722, but that fix does not cover directory compressed-size accounting; the current
validation path was added in astral-sh/uv#15136. The next step is to add integration coverage for
the descriptor and no-descriptor cases, along with the malformed CRC, size, and nonempty-directory
controls, then make the directory path consume and measure the entry without weakening descriptor
validation.

## Classification

This is a bug rather than an enhancement or question because the targeted reproduction shows uv
rejecting a valid ZIP representation accepted by Python's ZIP integrity check, while the equivalent
no-descriptor archive installs. Source inspection is consistent with the observed mismatch:
directory entries receive a computed compressed size of zero, while descriptor validation compares
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
  other malformed ZIP files” — Added explicit validation of descriptor CRC and sizes. Its streaming
  extractor changes introduced the exact combination visible today: directory entries receive a
  computed compressed size of zero, then descriptor compressed sizes are checked against it.

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
