# Avoid orphaned caret when formatting PEP 508 errors for empty input

Issue: astral-sh/uv#21089

Classification: bug

## Summary

An inline-script `dependencies` array containing an empty string is correctly rejected as an
invalid PEP 508 requirement, but the nested error ends with a caret on a blank line. The outer TOML
diagnostic already highlights the empty string; the additional PEP 508 caret points to no source
text and is misleading.

The same rendered artifact appears incidentally in a comment on astral-sh/uv#11215. That issue is
about `--no-deps`-style behavior and dependency metadata overrides, so it does not track this
formatting problem. The closest prior formatter work is astral-sh/uv#19781, which fixed a different
PEP 508 span-rendering failure for multibyte input. No existing issue or pull request was found that
tracks or fixes the empty-input case.

## Draft response

Thanks for the clear reproduction. The empty string is invalid and uv is correct to reject it, but
the standalone caret is not useful. The PEP 508 formatter currently forces a one-character
underline even when the input is empty, which produces this detached caret inside the TOML
diagnostic. This is a diagnostic-formatting bug. A fix should suppress the empty inner source and
underline while retaining the TOML span and PEP 508 message, with regression coverage for inline
script dependencies.

## Classification

This is a bug because repository source confirms that uv intentionally emits a source marker where
there is no source text:

- `parse_name` constructs the empty-input error with an empty `input`, `start: 0`, and `len: 1`.
- `Pep508Error` formatting treats `start == input.len()` as a one-character underline and always
  writes the message, input line, and caret line.
- The existing `error_empty` snapshot explicitly records a blank input line followed by `^`.
- PEP 508 requirements are deserialized by converting this formatted parser error into the TOML
  deserializer error, which explains why the detached inner caret appears after the correctly
  positioned outer TOML span.

The invalid dependency should continue to fail, but the detached marker is incorrect user-facing
output. This is not a duplicate: the only exact prior occurrence is incidental to a different open
request, and the related merged formatter change covers a different triggering condition. The
current issue is also labeled `bug` on GitHub.

## Related

- astral-sh/uv#11215 — A 2026 comment independently shows an empty `requires-dist` entry producing
  the identical `Empty field is not allowed for PEP508` message and detached caret. The issue itself
  tracks dependency-metadata and `--no-deps` semantics, not diagnostic formatting, so it is related
  evidence rather than a duplicate.
- astral-sh/uv#19781 — This merged pull request fixed correctness in the same PEP 508 error-rendering
  path by recording UTF-8 byte lengths so a multibyte span could be formatted without panicking. It
  is useful fix-oriented precedent, but it does not address zero-length input; the current
  empty-input snapshot still expects the orphaned caret.

## Search evidence

Searches covered open and closed issues plus open, closed, and merged pull requests. Literal queries
included the exact error text, `orphaned caret`, `points nowhere`, `TOML parse error` with `caret`,
and the current issue number. Conceptual queries covered `PEP508`/`PEP 508` errors, empty, blank, or
invalid dependencies and requirements, empty input and spans, underlines, diagnostic formatting,
and error highlighting. Fix-oriented review included recent commits and merged pull requests that
changed `crates/uv-pep508/src/lib.rs` and its span rendering.

The most plausible false matches were inspected with their comments and references.
astral-sh/uv#13183 concerns a setuptools error caused by a misplaced TOML table,
astral-sh/uv#12007 concerns lockfile invalidation for a valid empty dependency group, and
astral-sh/uv#8281 with astral-sh/uv#8282 concerns a leading-whitespace parser panic. Later formatter
fixes astral-sh/uv#19779 and astral-sh/uv#19796 address trailing separators and multibyte trailing
marker spans. None covers a detached caret for an empty requirement.
