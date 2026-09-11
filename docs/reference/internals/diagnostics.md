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
`Error::source()` node, but cannot create a cause or discard that source's own information or hints.
The command-layer renderer uses `Printer::stderr_important()`.

Hint ownership and hint ordering are separate. `Hinted::hints()` retains aggregate collection for
direct printing; `own_hints()` describes suggestions belonging to one error, and
`transparent_source()` exposes an inner root hidden by transparent presentation. Ordinary source
nodes are visited separately. All hints appear at the top level after the complete error, cause, and
information chain. `HintOrdering::First`, `HintOrdering::Any`, and `HintOrdering::Last` set priority
within that final section; equal-priority hints follow source-chain order and then any explicit
report-level hints. An outer error's advice is still owned by the outer error even though the
terminal renderer collects actions into one place.

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
| `SourceFile`       | A display name and shared decoded text, optionally starting at a later source line.                         |
| `SourceSnippet`    | A source snapshot, annotations, and a context-window policy. An annotation-free snippet is a file location. |
| `SourceAnnotation` | A half-open byte range, a primary or contextual role, and an optional short label.                          |
| `SourceEdit`       | A half-open byte range and exact replacement text in one source snapshot.                                   |
| `SourceSuggestion` | Validated, non-overlapping edits, explicit applicability, and permission to display the affected lines.     |

A display name may be a local path, a redacted URL, or a label for an argument or standard input.
Sources are captured when inputs are read or parsed. Formatting should not reopen files: an input
may have changed, disappeared, come from standard input, or been downloaded. Sharing snapshots
avoids copying a complete document into every related error. A source interner or database is not
needed for the initial implementation. Clones also share an immutable line index built when the
source is retained, so checking many declarations in one file does not repeatedly scan the complete
input.

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
title-less groups, location-only origins, and [replacement patches][annotate-patches]. Ruff's [fork
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
and serialization. `RequirementOrigin` is not itself a presentation-only field, however:
`RequirementOrigin::Group` participates in group-scoped explicit-index selection. Exact diagnostic
occurrences and any future solver-origin arena must remain separate from that semantic context.

## Structured reports and suggestions

The experimental `ErrorReport` uses the same resolved error-chain walk as the text renderer. Its
`errors` array contains the outer error followed by each actual cause; every entry owns its source
locations, `info` statements, and ordered hints. Keeping these owners in the structured report does
not change the terminal rule that every hint is a final, top-level call to action. Locations declare
one-based lines and zero-based UTF-8 byte columns. Source excerpts use the same explicit windows as
terminal output, and location-only sources do not serialize hidden annotations or retained file
contents.

The hidden `--error-format=json` option writes one complete JSON object per formatted error chain.
Transport escaping makes terminal controls and layout controls visible without changing the decoded
source or replacement text. This does not yet turn stderr into a uniform event stream: standalone
warnings, progress, Clap argument errors, and subprocess output retain their separate contracts.
Text remains the default. Neither selecting JSON nor constructing a report changes exit-status
classification.

`Hint::with_suggestion` attaches edits to the actual hint owner. Applicability is separate from
mechanical validity: `DisplayOnly` requires manual interpretation, `Unsafe` may change behavior, and
`Safe` represents the producer's confidence in the intended result. Ruff's [fix model][ruff-fixes]
illustrates this distinction and the separate question of grouping mutually exclusive edits. uv's
initial model accepts one retained source per suggestion, rejects invalid or overlapping edits, and
does not expose an apply operation. Equal hint messages cannot establish edit identity; duplicate
hints retain edits only when they refer to the same immutable suggestion, using the stricter source
visibility permission.

The terminal adapter includes a replacement-preview path through `annotate-snippets`. Current
production edit producers keep their source text private, so previews remain internal until a
source-safe producer needs that API. The hint retains its exact locations and the JSON
representation retains the explicit replacement. A source display name is not a file URI, and the
retained text may have been decoded or redacted. Even a `Safe` edit therefore needs an independently
verified document identity, version, and byte mapping before an editor or future uv command can
apply it.

## Implemented examples

The cost of adoption is mostly the cost of retaining accurate provenance, not drawing snippets. The
implemented examples exercise several boundaries:

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
- **Python requirements.** Direct `project.requires-python` errors retain their parsed declaration.
  `.python-version` requests retain the selected original occurrence. Dependency-group intersections
  carry all contributing declarations and include edges, including conflicts that cannot be
  explained by a single pair of bounds.
- **PEP 723 metadata.** An extraction map translates normalized TOML spans back to the original
  commented script, accounting for CRLF, UTF-8, removed prefixes, and EOF. Unknown mappings fall
  back instead of inventing script columns.
- **Sources and indexes.** Validation and lowering retain exact original array occurrences before
  marker, extra, or group filtering. Source-marker conflicts and missing named indexes use
  location-only output. Source validation checks every pair with matching extra and group selectors.
  A nonempty marker remainder can carry a display-only edit for the verified TOML value; a fully
  covered source receives disjointness or removal advice instead of an impossible replacement.
- **Configured environments.** Supported and required environment lists also check every pair for
  overlap. Exact workspace-owned arrays can identify both conflicting declarations and offer a
  display-only edit for a nonempty remainder. Merged configuration and scripts remain unlocated.
  Fully covered required environments receive no deletion edit because removing an entry can weaken
  artifact-coverage requirements.
- **Requirements in resolver errors.** `RequirementProvenance` identifies an exact parse occurrence
  independently of requirement equality and cache keys. Named registry requirements and nested
  constraints retain it through lowering, overrides, marker filtering, and the empty-range failure
  witness. Static PEP 621 metadata can opt in after checking the complete authored dependency and
  optional-extra sequence; built or cached metadata does not gain guessed source locations.
- **Unnamed-root dependency explanations.** An insertion ledger records the exact typed clauses
  passed to PubGrub in each fork. A location is shown only if all clauses for that dependency name
  are identical and agree on the authored occurrence and semantic source context, and the same
  clause appears in both the original proof and the final reduced report. A distinct or unlocated
  contributor leaves the explanation unannotated.

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

For example, an unambiguous requirements-file root can identify the declaration behind a normal
nonempty dependency clause:

```text
error: No solution found when resolving dependencies
  cause: Because pypyp was not found in the provided package locations and you require pypyp>=1, we can conclude that your requirements are unsatisfiable.
   --> requirements.in:2:1
    |
  2 | pypyp>=1
    | ^^^^^^^^ this dependency was declared here
```

## Remaining adoption decisions

The source model and renderer are no longer the main unknowns. Further work has distinct provenance
and product boundaries:

| Work                          | Current boundary                                                                           | What is still needed                                                                                                                                                                        |
| ----------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| More source-bearing errors    | Representative TOML, requirements, project Python, group, script, source, and index errors | Retain exact occurrences for each new semantic validator; decide whether its complete source lines are safe. Standalone or transformed inputs need explicit coordinate mappings.            |
| More project metadata         | Known-static PEP 621 dependency order                                                      | Explicit provenance for dependency groups, build requirements, dynamic/backend metadata, `PKG-INFO`, wheels, and remote `METADATA`; never infer declarations from cached normalized values. |
| General resolver explanations | Empty-range witnesses and unambiguous unnamed-root clauses                                 | Merge-aware one-to-many PubGrub origin identities, with unlocated contributors and source-aware report rewrites.                                                                            |
| Stable structured output      | Hidden experimental error-chain JSON                                                       | Decide schema evolution, stable codes, source/document identity, and the relationship to warnings, progress, Clap, and child-process output.                                                |
| Applying suggestions          | Validated single-source edits and explicit applicability                                   | Define source version verification, decoded-to-file byte mappings, multi-file atomicity, conflicting edit groups, and opt-in behavior-changing edits.                                       |

The resolver boundary is substantial. The pinned `astral-pubgrub` `0.6.1` [insertion
API][pubgrub-insertion] accepts package/range dependencies, while its [exported dependency
leaves][pubgrub-report] retain only the semantic parent and child clauses. [Coalescing
dependencies][pubgrub-merge] can combine several parent versions into a new incompatibility. Package
names, overlapping ranges, and formatted explanations cannot recover which authored occurrences
contributed to that merged leaf.

A general prerequisite should accept an opaque client origin with each inserted dependency, preserve
immutable unions of those origins when incompatibilities merge, and expose the resulting origin set
or complete merge ancestry in the report. Unlocated contributors must remain explicit. uv can then
map solver-local origins to shared source occurrences and effective marker/source context, without
changing package/range equality, hashing, solving, or cache keys. Origin unions should be shared or
interned, and solver-local IDs must stay paired with their fork-local arena. Report simplification
must drop, retain, or union those origins alongside the actual explanation it rewrites. Remote
package metadata and Python-compatibility edges can be adopted after that contract exists.

New transparent or type-erased error owners still need registration and source-contract coverage.
Source-only producers do not need artificial `Hinted` implementations. Full resolver provenance, a
stable structured format, and automatic edits remain independent changes.

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
[annotate-patches]:
  https://github.com/rust-lang/annotate-snippets-rs/blob/2d2643cf7c64abbf014ff1df736865074a8b94de/src/snippet.rs#L411-L444
[ruff-fixes]:
  https://github.com/astral-sh/ruff/blob/6c8eb98a0295a31b90df12d2307f910376f3bbcc/crates/ruff_diagnostics/src/fix.rs#L8-L55
[pubgrub-insertion]:
  https://github.com/astral-sh/pubgrub/blob/233ec98313aa2e5c25dd579c505f3d8594eda2e9/src/internal/core.rs#L123-L205
[pubgrub-report]:
  https://github.com/astral-sh/pubgrub/blob/233ec98313aa2e5c25dd579c505f3d8594eda2e9/src/report.rs#L31-L64
[pubgrub-merge]:
  https://github.com/astral-sh/pubgrub/blob/233ec98313aa2e5c25dd579c505f3d8594eda2e9/src/internal/incompatibility.rs#L241-L276
