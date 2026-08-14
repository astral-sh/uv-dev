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

## Draft response

Thanks for the focused reproduction. This was a bug in how `uv add` handled the inline comment on
the final array item when that item had no trailing comma: the formatter could attach the comment
to the preceding dependency while adding the comma. The fix and an integration regression test
were merged in astral-sh/uv#21008 and released in uv 0.12.4. Please upgrade to uv 0.12.4 or later;
your trailing-comma workaround remains valid on older versions.

## Classification

This is a bug because `uv add` incorrectly changed which dependency an existing inline comment
described. The merged implementation in astral-sh/uv#21008 confirms that, without a trailing comma,
`toml_edit` stores the comment in the final value's suffix and uv's array formatter previously
treated that suffix as a prefix, allowing the comment to move to the preceding dependency. The pull
request adds the exact integration regression test and is labeled `bug`.

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
