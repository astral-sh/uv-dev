# Make `uv cache size` human readable by default

Issue: astral-sh/uv#20984

Classification: enhancement

## Summary

The report asks for the preview-gated `uv cache size` command to print a human-readable value such
as `113.0GiB` by default instead of an unadorned raw-byte count such as `121341640704`. It also asks
for scripting use to remain available through an explicit inverse option such as `--bytes`, and
raises whether the existing `--human` option should remain as a compatibility no-op, become hidden,
or be removed.

No earlier issue or pull request was found that already tracks this default-output change. The
closest history is the original scripting-oriented command request in astral-sh/uv#15821 and its
implementation in astral-sh/uv#16032, which intentionally selected raw bytes by default and added
`--human`. The open astral-sh/uv#17884 is adjacent because a maintainer points users to
`uv cache size --human`, but it does not ask to reverse the default.

## Report details

- Subsystem and command: cache management, specifically the preview `uv cache size` command and
  `cache-size` preview feature.
- Observed behavior: default output is a bare integer representing bytes; `-H`/`--human` formats it
  using units.
- Requested capability: make formatted units the human-facing default and retain exact raw bytes
  behind an explicit machine-oriented option such as `--bytes`.
- Triggering condition: invoking the command without an output-format flag, demonstrated with uv
  0.12.2 on macOS arm64.
- Exact identifiers searched: `uv cache size`, `cache-size`, `--human`, `--human-readable`,
  `--bytes`, `raw bytes`, and `human-readable`.

## Draft response

Thanks for laying out the human and scripting use cases. The current interface comes from
astral-sh/uv#16032, which implemented astral-sh/uv#15821 with raw bytes as the default and
`--human` for formatted output.

Because `uv cache size` is still preview-gated, this is the appropriate stage to evaluate changing
that interface. Any change should preserve an unambiguous raw-byte mode for scripts. The remaining
design decision is whether `--human` stays as a hidden compatibility alias once human-readable
output is the default, or is removed before stabilization. Once maintainers agree on that CLI shape,
the prepared change can be reviewed against astral-sh/uv#20984.

## Classification

This is an enhancement. The raw-byte default is not an accidental or source-confirmed correctness
failure: astral-sh/uv#16032 explicitly describes raw bytes as the default, current CLI documentation
says the same, the implementation branches on the `human` flag, and integration snapshots cover
both raw default output and `--human` output. The requested human-first default and inverse raw-byte
flag improve the command's interface while retaining existing capability.

It is not a duplicate. astral-sh/uv#15821 requested a script-parseable size command, not a
human-readable default; astral-sh/uv#16032 implemented that earlier request; and
astral-sh/uv#17884 asks more generally for a cache-size tool. None already centralizes discussion of
reversing the output default.

## Related

- astral-sh/uv#15821 (closed issue), “uv cache size”: the original feature request asked for a
  built-in size command that scripts could parse to decide when to clear an unbounded cache. It is
  the direct requirement history, but it did not specify that raw bytes must be the no-flag default.
- astral-sh/uv#16032 (merged pull request), “Add a `uv cache size` command”: closed
  astral-sh/uv#15821 and explicitly implemented raw bytes by default with `--human` for formatted
  output. This is the strongest evidence for why the current interface exists, but it does not track
  the newly requested reversal.
- astral-sh/uv#17884 (open issue), “Cache tool to show size”: an adjacent request for cache-size
  visibility. A maintainer comment identifies `uv cache size --human` as the existing solution. It
  neither requests a default-format change nor an inverse raw-byte flag, so it is not a duplicate.

## Search scope and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches
used the command and feature names plus the output flags and phrases above. Conceptual searches used
`cache usage`, `disk usage`, `cache info`, `machine-readable`, scripting, output defaults, cache
pruning, and human-readable size. Fix-oriented searches followed the closure and cross-reference
chain from astral-sh/uv#15821 to merged astral-sh/uv#16032 and inspected their comments and review
discussion.

Several plausible-looking results were ruled out as canonical matches:

- astral-sh/uv#12854 asks for usage broken down by package, not a formatting default.
- astral-sh/uv#5731 asks for bounded or automatic cache eviction; it only led to
  astral-sh/uv#15821 as a scripting building block.
- astral-sh/uv#1655 asks for broader `pip cache info` and `pip cache list` equivalents.
- astral-sh/uv#18779 concerns whether `uv cache prune` overstates space actually freed, a size
  accounting/correctness question rather than raw-versus-formatted presentation.

The checkout independently confirms the current design: `SizeArgs` exposes only the `human` boolean,
the command prints raw `total_bytes` unless that flag is set, and nearby integration snapshots cover
the raw default and human-readable opt-in. The issue itself currently carries the repository's
`enhancement` label, consistent with this evidence.
