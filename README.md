# ZIP extraction rejects DEFLATE-compressed empty directory with data descriptor

Issue: astral-sh/uv#21644

Classification: bug

## Summary

uv 0.12.13 rejects a ZIP source distribution containing an explicit empty directory when the
directory uses DEFLATE and stores its sizes in a data descriptor. The reported synthetic control
without a descriptor succeeds, while the descriptor form fails with `Bad compressed size (got
00000000, expected 00000002) for file: META-INF`. The same failure is reported for an official
`ibapi` direct-URL source archive with a `subdirectory` fragment.

The current streaming extractor supports the reported mechanism. Its directory branch records a
computed CRC and uncompressed size of zero, which are appropriate for an empty directory, but also
records a compressed size of zero without consuming and measuring the entry's DEFLATE stream. The
later data-descriptor validation compares that zero with the descriptor's compressed size. An empty
DEFLATE stream still occupies compressed bytes, so this rejects a supported ZIP representation.

No existing open issue or pull request tracks this exact empty-directory failure. The closest
history is a prior false rejection of data-descriptor ZIPs in astral-sh/uv#12677, its fix in
astral-sh/uv#12722, and the comprehensive streaming ZIP validation added by astral-sh/uv#15136.

## Draft response

Thanks for the self-contained reproduction. The current source confirms this is a bug in streaming
ZIP validation: directory entries are assigned a computed compressed size of zero, and that value
is later compared with the data descriptor, even though an empty DEFLATE stream has a nonzero
compressed length.

The earlier descriptor-related false positive in astral-sh/uv#12677 was fixed by
astral-sh/uv#12722, but that fix does not cover directory compressed-size accounting; the current
validation path was added in astral-sh/uv#15136. The next step is to add integration coverage for
the descriptor and no-descriptor cases, along with the malformed CRC, size, and nonempty-directory
controls, then make the directory path consume and measure the entry without weakening descriptor
validation.

## Classification

This is a bug rather than an enhancement or question because uv rejects a valid instance of a ZIP
representation that its streaming extractor supports. The source confirms the key mismatch:
directory entries receive a computed compressed size of zero, while descriptor validation compares
that value with the actual compressed size. The report's two-byte value is consistent with an empty
DEFLATE stream having nonzero encoded length.

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
