# Rich diagnostics

This document proposes source-aware diagnostics for uv. The aim is to explain errors using the
relevant declarations, including related locations in other files, without coupling error handling
to a particular terminal renderer. The source model and adoption phases below are not a public API
or a commitment to a particular output format.

## Existing boundaries

The command layer classifies errors with `UvError`. The entrypoint maps `UvError::User` to exit code
`1` and `UvError::Argument` or `UvError::Unexpected` to exit code `2`. Commands that have already
reported their result return an `ExitStatus`; external command statuses are propagated separately.
Adding presentation data must not change which classification the existing conversions select.

`uv-errors` renders the actual `Error::source()` chain with an `error:` or `warning:` headline and
`cause:` entries. A `DiagnosticFn` can supply a `Diagnostic` for an individual error, replacing its
displayed message or adding ordered `Info` statements. Unrecognized errors use `Display`. An `Info`
supplies context, not another cause or an instruction to the user. Actionable suggestions remain
`Hint` values, collected through `Hinted` and ordered with `HintOrdering`. The command-layer
renderer uses `Printer::stderr_important()`; a source-aware renderer must retain that output policy.

With source annotations, these boundaries would let a parser error display a concise cause and a
source excerpt without discarding the underlying error, changing its exit status, or comparing
rendered strings to decide what to display.

## Source model

The proposed extension adds annotations to `Diagnostic` and `Info`. It needs three concepts:

| Concept         | Contents                                                                                                                                        |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Source snapshot | A display name and the exact decoded text, shared with `Arc<str>` or equivalent ownership.                                                      |
| Source span     | A source snapshot and an optional half-open `Range<usize>`. No range means a file-level location; an empty range identifies an insertion point. |
| Annotation      | A source span, a primary or contextual role, and an optional short label.                                                                       |

A display name may be a local path, a redacted URL, or a label for an argument or standard input.
Sources are captured when inputs are read or parsed. Formatting should not reopen files: an input
may have changed, disappeared, come from standard input, or been downloaded. Sharing snapshots
avoids copying a complete document into every related error. A source interner or database is not
needed for the initial implementation.

Coordinates are zero-based UTF-8 byte offsets into the retained, decoded source, with an exclusive
end. They are not character indices, display columns, or necessarily offsets into the original file
bytes. Bounds and UTF-8 character boundaries must be checked before rendering. An empty span at the
end of the source is valid. An invalid span should lose its highlight, not the error message, and
must not make error reporting panic.

This distinction matters for `requirements.txt`, which uv can transcode from UTF-16, and for PEP 723
metadata, whose TOML parser receives text with the Python comment prefixes removed. Any conversion
from parser coordinates to a different source snapshot needs an explicit mapping. The same applies
to TOML string escapes, multiline strings, newline normalization, and redaction. When an exact
subspan cannot be mapped, highlighting the containing syntax element is preferable to searching for
the decoded value in the document.

## Rendering and safety

ty provides useful examples of diagnostics that combine a primary location, contextual locations,
and additional explanations. Its [diagnostic model][ty-model] separates these facts from source
lookup and rendering. Its [renderer][ty-renderer] groups nearby annotations and handles source
transformations centrally. uv needs the same separation, but not ty's incremental database or its
Python-specific file model.

An adapter around upstream [`annotate-snippets`][annotate-snippets] is a reasonable starting point.
The `0.12.16` API supports source snippets, primary and contextual annotations, [custom level
names][annotate-levels], and replacement patches. Ruff's [fork rationale][ruff-fork] illustrates why
terminal formatting compatibility should be controlled by uv rather than exposed throughout the
codebase. A fork should only be necessary if a concrete formatting requirement cannot be expressed
through the upstream API.

The adapter should render annotations beside the cause or `Info` that owns them. Multiple locations
from one source can share a snippet; primary locations should precede related locations. Unannotated
errors continue through the ordinary formatter. The adapter must use uv's color and width policy,
rather than silently adopting the renderer's defaults. Source text should be clipped or folded as
source, not reflowed as prose.

Source text, paths, labels, and server responses are untrusted terminal input. In particular,
`annotate-snippets` treats [`secondary_title` and `Level::message`][annotate-trust] as potentially
styled text, so callers must normalize untrusted data before using those interfaces. Any control
character replacement must retain a mapping to the original span. Source excerpts also must not
reveal credentials that ordinary error messages redact, including credentials in index URLs or
nearby configuration entries. Narrow context windows help, but do not replace redaction. If a source
cannot be shown safely, a location without an excerpt is still useful.

The renderer is not an error classifier. Formatting changes must leave error-chain traversal,
downcasting, hints, command exit codes, quiet-mode behavior, and external-command status propagation
unchanged. Presentation-only provenance must also stay out of resolver identity and cache keys;
`uv-distribution-types::Requirement` already excludes its `origin` from equality, ordering, hashing,
and serialization.

## Adoption

The cost of adoption is mostly the cost of retaining accurate provenance, not drawing snippets. The
following phases can be reviewed independently:

1. **Parser errors.** Add the source model and renderer, then attach snapshots at TOML and PEP 508
   parse boundaries. `toml::de::Error` exposes a message and byte span, and `Pep508Error` retains
   its input and byte offsets. Their existing self-rendered excerpts need a presentation override to
   avoid printing the same source twice. This is a relatively small, localized change.
2. **Nested requirements files.** `RequirementsTxtParserError` already records spans for
   requirements and `-r` or `-c` inclusions. Retaining the corresponding source snapshots would
   allow an invalid requirement to point back through the files that included it. The additional
   work is source ownership, decoded-input coordinates, and useful ordering across files.
3. **Semantic TOML fields.** Add field and occurrence provenance for dependency groups,
   `tool.uv.sources`, indexes, and related declarations. Missing groups, inclusion cycles, and
   overlapping source markers are useful examples. This requires syntax-aware spans, not matching
   normalized names or decoded values against the raw document. Keeping a compact source map beside
   the parsed metadata avoids making every semantic type renderer-dependent.
4. **Cross-file Python requirements.** Retain locations alongside the project and dependency-group
   constraints collected by `RequiresPythonSources`, and retain the source of a Python request.
   Compatibility errors could then show a `.python-version` entry alongside the declarations in
   workspace members that reject it. Group inclusion and intersected constraints make this more
   involved than a parser diagnostic.
5. **Resolver provenance.** Carry declaration origins through requirement lowering, merging, marker
   transformations, and PubGrub explanations. One resulting requirement may have several origins; a
   single path or source span is not sufficient. This is the largest change and needs a separate
   design for provenance identity, memory use, caching, and diagnostics for remote package metadata.
6. **Structured output and suggestions.** A machine-readable format should serialize typed messages,
   locations, and relationships instead of parsing terminal output. Its schema must define
   coordinate units and source availability. Typed replacements additionally need applicability,
   overlapping edit, and changed-file policies. Displaying a possible replacement does not imply
   that uv can safely apply it.

The parser and nested-file phases can establish whether the model produces useful diagnostics with a
modest amount of plumbing. Semantic TOML and workspace constraints exercise richer relationships.
Full resolver provenance, a stable structured format, and automatic edits remain distinct adoption
decisions.

[ty-model]:
  https://github.com/astral-sh/ruff/blob/6c8eb98a0295a31b90df12d2307f910376f3bbcc/crates/ruff_db/src/diagnostic/mod.rs#L630-L760
[ty-renderer]:
  https://github.com/astral-sh/ruff/blob/6c8eb98a0295a31b90df12d2307f910376f3bbcc/crates/ruff_db/src/diagnostic/render.rs#L304-L406
[annotate-snippets]:
  https://github.com/rust-lang/annotate-snippets-rs/blob/2d2643cf7c64abbf014ff1df736865074a8b94de/src/snippet.rs
[annotate-levels]:
  https://github.com/rust-lang/annotate-snippets-rs/blob/2d2643cf7c64abbf014ff1df736865074a8b94de/src/level.rs#L146-L172
[ruff-fork]:
  https://github.com/astral-sh/ruff/blob/6c8eb98a0295a31b90df12d2307f910376f3bbcc/crates/ruff_annotate_snippets/README.md
[annotate-trust]:
  https://github.com/rust-lang/annotate-snippets-rs/blob/2d2643cf7c64abbf014ff1df736865074a8b94de/src/level.rs#L70-L125
