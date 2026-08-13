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

## Reproduction

Outcome: **reproducible**.

The report specifies uv 0.12.3 on macOS, but does not provide a macOS version, architecture, or
Python version. The same behavior was reproduced with the installed `uv 0.12.3
(x86_64-unknown-linux-gnu)` on Linux 6.17.0-1020-azure x86_64. Python was not selected or invoked:
uv rejected the inline metadata before script execution. All reproduction files, uv cache data,
Python-install data, and XDG state were isolated under a newly created `/tmp` directory.

Minimal `script.py`:

```python
# /// script
# requires-python = ">=3.11"
# dependencies = ["httpx", ""]
# ///

import httpx
```

Command, with `<temp>` referring to the isolated temporary directory:

```console
UV_CACHE_DIR=<temp>/cache \
UV_PYTHON_INSTALL_DIR=<temp>/python \
XDG_CACHE_HOME=<temp>/xdg-cache \
XDG_CONFIG_HOME=<temp>/xdg-config \
uv run --script <temp>/script.py
```

The command exited with status 2 and produced no stdout. Its stderr included the correct outer TOML
span followed by the reported detached caret on a blank PEP 508 input line:

```text
error: TOML parse error at line 2, column 26
  |
2 | dependencies = ["httpx", ""]
  |                          ^^
Empty field is not allowed for PEP508

^
```

Existing coverage is limited to the parser-level unit snapshot
`crates/uv-pep508/src/lib.rs::tests::error_empty`, which parses an empty requirement and explicitly
expects the message, blank input line, and caret. Searches of the integration suites under
`crates/uv/tests/` and `crates/uv-client/tests/it/` found no test for an empty dependency in PEP 723
inline-script metadata. In particular, `crates/uv/tests/project/run.rs::run_pep723_script` covers
valid inline dependencies and malformed PEP 723 tags, but not this nested empty-requirement
diagnostic.

## Draft response

Thanks for the clear reproduction. The empty string is invalid and uv is correct to reject it, but
the standalone caret is not useful. The PEP 508 formatter currently forces a one-character
underline even when the input is empty, which produces this detached caret inside the TOML
diagnostic. This is a diagnostic-formatting bug. A fix should suppress the empty inner source and
underline while retaining the TOML span and PEP 508 message, with regression coverage for inline
script dependencies.

## Classification

This is a reproducible diagnostic-formatting bug: uv 0.12.3 rejects the invalid dependency as
expected, but emits a source marker where there is no source text. Repository source explains the
observed rendering:

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
