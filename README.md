# Location of inline comment not preserved when using `uv add` for an optional dependency

Issue: astral-sh/uv#21006

Classification: bug

## Summary

On uv 0.12.2, updating the final entry in a `[project.optional-dependencies]` array with
`uv add --optional` could move that entry's inline comment onto the preceding dependency when the
final entry did not have a trailing comma. Adding a trailing comma before running the command
avoided the problem.

The behavior was confirmed and fixed by astral-sh/uv#21008. The change preserves the final item's
inline comment while normalizing the array to include a trailing comma and adds an integration
regression test using the report's optional-dependency scenario. The fix was released in uv 0.12.4.

## Reproduction

Outcome: **reproducible** with the reported uv 0.12.2, and no longer present in uv 0.12.4.

The report's complete `pyproject.toml` was copied into two isolated temporary directories on Linux
x86_64 (kernel 6.17.0-1022-azure) with CPython 3.12.3. All caches, tool installations, virtual
environments, and generated lockfiles were kept beneath `$RUNNER_TEMP`. The affected version was
obtained through the installed uv executable and verified as `uv 0.12.2
(x86_64-unknown-linux-gnu)`; the uv executable on `PATH` was `uv 0.12.4
(x86_64-unknown-linux-gnu)`.

For uv 0.12.2, the exact reported project command was run through that isolated version:

```console
$ uv tool run --from uv==0.12.2 uv add --optional=typing 'narwhals>=1.42'
```

It exited successfully and changed the optional dependency array to:

```toml
[project.optional-dependencies]
typing = [
    "pandas-stubs>=2.0.2", # narwhals are toothed whales native to the Arctic
    "narwhals>=1.42",
]
```

This exactly reproduces the reported behavior: the comment originally attached to the final
`narwhals` item, which had no trailing comma, moved to the preceding `pandas-stubs` item.

Running the same command and original fixture with uv 0.12.4 also exited successfully, but produced:

```toml
[project.optional-dependencies]
typing = [
    "pandas-stubs>=2.0.2",
    "narwhals>=1.42", # narwhals are toothed whales native to the Arctic
]
```

Current integration coverage is in `crates/uv/tests/project/edit.rs`, test
`add_preserves_end_of_line_comment_on_updated_optional_dependency`. Its setup uses the same
two-entry `typing` array with the final `narwhals` dependency lacking a trailing comma, runs
`uv add narwhals>=1.42 --optional=typing --frozen`, and snapshots the comment remaining on
`narwhals` while a trailing comma is added.

## Draft response

Thanks for the focused reproduction. This was a bug in how `uv add` handled the inline comment on
the final array item when that item had no trailing comma: the formatter could attach the comment
to the preceding dependency while adding the comma. The fix and an integration regression test
were merged in astral-sh/uv#21008 and released in uv 0.12.4. Please upgrade to uv 0.12.4 or later;
your trailing-comma workaround remains valid on older versions.

## Classification

This is a bug because the uv 0.12.2 reproduction shows that `uv add` changed which dependency an
existing inline comment described. astral-sh/uv#21008 addresses the final-item, no-trailing-comma
case and adds the exact integration regression test; uv 0.12.4 preserves the comment in the same
fixture.

This is not a duplicate. astral-sh/uv#21008 was opened in direct response to this report, and the
earlier comment-position fixes addressed different triggers: inserting a new dependency after a
commented non-terminal item that already had a comma. The final-item, no-trailing-comma update case
reported here was a distinct uncovered edge case rather than the exact previously fixed scenario.

## Related

- astral-sh/uv#21008 — Merged pull request, “Preserve inline comments when updating dependencies.”
  This is the exact fix: it handles updating the final dependency without a trailing comma, retains
  its inline comment, adds the report's scenario as an integration test, and shipped in uv 0.12.4.
- astral-sh/uv#8982 — Closed issue, “Inconsistency with comments on uv add.” This is the closest
  historical symptom: `uv add` moved a non-terminal dependency's inline comment onto a newly
  inserted dependency. It differs because the commented item already had a comma and was not the
  last array item; maintainers confirmed astral-sh/uv#12360 fixed that case.
- astral-sh/uv#12360 — Merged pull request, “Retain end-of-line comment position when adding
  dependency.” This fixed the adjacent mid-array insertion case tracked by astral-sh/uv#12333 and
  later confirmed to fix astral-sh/uv#8982, but it did not cover updating a final item without a
  trailing comma.

## Search evidence

Authenticated searches covered the exact title and report identifiers (`inline comment`,
`trailing comma`, `narwhals`, `pandas-stubs`, and uv 0.12.2), followed by conceptual searches for
moved or shifted comments, TOML comment/decor preservation, final or last array items, `toml_edit`
suffix handling, optional-dependency updates, and `uv add` formatting. Fix-oriented searches
covered open and closed issues, open/closed/merged pull requests, linked timelines and comments,
the current source and integration tests, and release notes.

Several plausible results were inspected but are materially different. astral-sh/uv#8343 and
astral-sh/uv#8384 address broader comment repositioning while inserting dependencies.
astral-sh/uv#16719 and astral-sh/uv#16734 address inline-comment whitespace normalization rather
than reassociation with another dependency. astral-sh/uv#9856 and astral-sh/uv#13966 concern
`uv remove`, while astral-sh/uv#6364 and astral-sh/uv#13447 concern PEP 723 script metadata.
