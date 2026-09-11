# Rich diagnostics

This document describes an exploration of source-aware diagnostics for uv. The aim is to explain
errors using the relevant declarations, including related locations in other files, without coupling
error handling to a particular terminal renderer. The implemented examples establish the
presentation and provenance boundaries; they are not a public API or a commitment to a particular
output format.

## Existing boundaries

The command layer classifies errors with `UvError`. The entrypoint maps `UvError::User` to exit code
`1` and `UvError::Argument` or `UvError::Unexpected` to exit code `2`. Commands that have already
reported their result return an `ExitStatus`; external command statuses are propagated separately.
Adding presentation data must not change which classification the existing conversions select.

`uv-errors` renders the actual `Error::source()` chain with an `error:` or `warning:` headline and
`cause:` entries. A `DiagnosticFn` can supply a `Diagnostic` for an individual error, replacing its
displayed message or adding ordered `Info` statements. Unrecognized errors use `Display`. An `Info`
supplies context, not another cause or an instruction to the user. Actionable suggestions remain
`Hint` values. `Diagnostic::with_source` can override the presentation of the next actual
`Error::source()` node, but cannot create a cause or discard that source's own hints. The
command-layer renderer uses `Printer::stderr_important()`.

Hint ownership and hint ordering are separate. `Hinted::hints()` retains aggregate collection for
direct printing; `own_hints()` describes suggestions belonging to one error, and
`transparent_source()` exposes an inner root hidden by transparent presentation. Ordinary source
nodes are visited separately. `HintOrdering::First` and `HintOrdering::Any` appear beside their
owner, while `HintOrdering::Last` follows that owner's complete source chain. An outer error's
trailing advice is therefore still owned by the outer error.

The code constructing an error must retain the typed facts needed to choose its suggestions: for
example, the requested package, command invocation, or failed resolution environment. Error
conversions and type-erased wrappers must propagate that context. The renderer decides placement,
not semantic ownership; neither rendered-string comparisons nor `HintOrdering` should be used to
reconstruct information that was discarded earlier.

These boundaries let a parser error display a concise cause and a source excerpt without discarding
the underlying error or changing its exit status. Unrecognized errors continue through the ordinary
formatter.

## Source model

`Diagnostic` and `Info` can contain source snippets:

| Type               | Contents                                                                                                    |
| ------------------ | ----------------------------------------------------------------------------------------------------------- |
| `SourceFile`       | A display name and exact decoded text shared with `Arc<str>`, optionally starting at a later source line.   |
| `SourceSnippet`    | A source snapshot, annotations, and a context-window policy. An annotation-free snippet is a file location. |
| `SourceAnnotation` | A half-open byte range, a primary or contextual role, and an optional short label.                          |

A display name may be a local path, a redacted URL, or a label for an argument or standard input.
Sources are captured when inputs are read or parsed. Formatting should not reopen files: an input
may have changed, disappeared, come from standard input, or been downloaded. Sharing snapshots
avoids copying a complete document into every related error. A source interner or database is not
needed for the initial implementation.

Coordinates are zero-based UTF-8 byte offsets into the retained, decoded source, with an exclusive
end. They are not character indices, display columns, or necessarily offsets into the original file
bytes. Terminal locations use character columns, but stored spans remain byte ranges. Bounds and
UTF-8 character boundaries are checked before rendering. An empty span at the end of the source is
valid. An invalid span loses its highlight, not the error message, and must not make error reporting
panic. `SourceSnippet::without_source_text()` retains a location without exposing its text;
`SourceFile::with_line_start()` identifies an excerpt made of complete original lines.

This distinction matters for `requirements.txt`, which uv can transcode from UTF-16, and for PEP 723
metadata, whose TOML parser receives text with the Python comment prefixes removed. Any conversion
from parser coordinates to a different source snapshot needs an explicit mapping. The same applies
to TOML string escapes, multiline strings, newline normalization, and redaction. When an exact
subspan cannot be mapped, highlighting the containing syntax element is preferable to searching for
the decoded value in the document. `uv_toml::SourceMap` provides syntax-aware locations through
decoded TOML keys and exact array occurrence indices. Semantic producers still need to retain which
declaration participated in the error and confirm that it matches the retained syntax.

## Rendering and safety

ty provides useful examples of diagnostics that combine a primary location, contextual locations,
and additional explanations. Its [diagnostic model][ty-model] separates these facts from source
lookup and rendering. Its [renderer][ty-renderer] groups nearby annotations and handles source
transformations centrally. uv needs the same separation, but not ty's incremental database or its
Python-specific file model.

The adapter uses upstream [`annotate-snippets`][annotate-snippets] `0.12.16`. That API supports
source snippets, primary and contextual annotations, [custom level names][annotate-levels],
title-less groups, location-only origins, and replacement patches. Ruff's [fork
rationale][ruff-fork] illustrates why terminal formatting compatibility should be controlled by uv
rather than exposed throughout the codebase. No requirement for a fork has been established; one
should only be necessary if a concrete formatting requirement cannot be expressed through the
upstream API.

The adapter renders annotations beside the cause or `Info` that owns them, retaining producer order
across snippets. Multiple locations from one source can share a snippet; primary locations should
precede related locations. Selected adjacent windows can be merged within a snippet. Broader merging
needs a separate policy because it may reveal intervening configuration. The adapter uses uv's color
and width policy. Source text is clipped or folded as source, not reflowed as prose.

Source text, paths, labels, and server responses are untrusted terminal input. In particular,
`annotate-snippets` treats [`secondary_title` and `Level::message`][annotate-trust] as potentially
styled text. The adapter normalizes terminal controls and remaps the affected offsets before
rendering. Source excerpts also must not reveal credentials that ordinary error messages redact,
including credentials in index URLs or nearby configuration entries. Surrounding context defaults to
zero lines. `SourceFile::lines_for_span` and `line_range_for_span` expose the same LF-delimited
windows used by the renderer, including CR bytes, so producers can decide whether the entire visible
window is safe. Additional context needs its own safety review.

Privacy decisions remain with the producer; the renderer does not understand credentials. The group
and duplicate-name diagnostics use typed provenance and conservative visibility checks. Generic TOML
parse errors retain the parser's message and show only the first selected physical line, but that
does not establish a general redaction policy for malformed TOML values or parser messages. When
source cannot be shown safely, a location without an excerpt is still useful.

The renderer is not an error classifier. Formatting changes must leave error-chain traversal,
downcasting, hints, command exit codes, quiet-mode behavior, and external-command status propagation
unchanged. Presentation-only provenance must also stay out of resolver identity and cache keys;
`uv-distribution-types::Requirement` already excludes its `origin` from equality, ordering, hashing,
and serialization.

## Implemented examples

The cost of adoption is mostly the cost of retaining accurate provenance, not drawing snippets. Four
examples exercise different boundaries:

- **Requirements files.** Retain decoded inputs for PEP 508 errors and nested `-r` or `-c`
  inclusions. The primary error can point to the invalid requirement, alongside the related include
  locations. Presentation overrides avoid repeating the parser's self-rendered excerpt.
- **TOML parse errors.** Retain source text and use the parser's message and byte span to show a
  concise cause with its location. Full-line excerpts retain their original line numbers.
- **Dependency groups.** Retain normalized group identities and exact include occurrences through
  semantic traversal. Missing groups point to the failed include; cycles point to the closing edge
  and related earlier edges. URL-bearing requirements, arbitrary marker text, and uncertain source
  layouts use location-only output.
- **Duplicate workspace names.** Retain both parsed project sources at the conflict boundary. The
  second declaration is primary and the first is a related location, including when their original
  spellings normalize to the same package name.

For example, the duplicate-name diagnostic shows both declarations:

```text
error: Two workspace members are both named `example`
   --> second/pyproject.toml:2:8
    |
  2 | name = "example"
    |        ^^^^^^^^^ duplicate name
  info: The name was first declared here
   --> first/pyproject.toml:2:8
    |
  2 | name = "example"
    |        --------- first declared here
```

These examples do not cover every standalone PEP 508 error, PEP 723 metadata, or resolver conflict.

## Further adoption

The next changes can remain independently reviewable:

1. **Direct project Python requirements — small to medium.** Attach locations for direct
   `[project].requires-python` declarations while the workspace and parsed metadata are still
   available. The first change should exclude inherited dependency-group bounds: a flattened
   intersection cannot honestly be attributed to one declaration.
2. **Python requests and group constraints — medium.** Retain the original `.python-version` text
   and the selected occurrence. Carry all contributing group declarations through inclusion and
   intersection. An empty intersection involving several declarations may need all contributors or a
   typed incompatibility witness, not an arbitrary pair of locations.
3. **PEP 723 metadata — medium.** Add an extraction map from normalized TOML back to the decoded
   script, accounting for removed comment prefixes, CRLF, UTF-8, and EOF. A synthetic
   script-metadata source is a smaller alternative, but must not claim original-script columns.
4. **Sources, indexes, and marker conflicts — medium.** `SourceMap` already handles dotted keys,
   inline tables, and array-of-table occurrences. Retain the declaration participating in each
   validation error; URL-bearing fields should initially allow location-only output.
5. **Resolver provenance — large.** Carry one-to-many declaration identities through requirement
   lowering, constraints, overrides, merging, marker transformations, and PubGrub explanations.
   `RequirementOrigin` identifies a file, project, or group, but not an exact occurrence, and some
   origins already affect group-scoped source behavior. A separate design must cover provenance
   identity, memory use, caching, explanation simplification, and remote package metadata.
6. **Structured output and suggestions — separate design.** Serialize typed messages, locations, and
   relationships instead of parsing terminal output. Define coordinate units, unavailable source,
   and schema stability. Typed replacements additionally need applicability, overlapping edit, and
   changed-file policies. Displaying a possible replacement does not imply that uv can safely apply
   it.

New transparent or type-erased error owners also need registration and source-contract coverage.
Source-only producers do not need artificial `Hinted` implementations. Full resolver provenance, a
stable structured format, and automatic edits remain distinct adoption decisions.

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
