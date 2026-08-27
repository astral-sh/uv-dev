# uv-pep508 evaluates python_version in a,b,c as always true

Issue: astral-sh/uv#21310

Classification: bug

## Summary

The report demonstrates that uv-pep508 parses a valid PEP 508 marker such as
`python_version in "3.8,3.9,3.13"` into an unconditional `true` tree. Evaluation therefore returns
true for Python versions that are not substrings of the right-hand value. The crate-level
reproduction covers both comma-separated and comma-plus-space forms on uv-pep508 0.12.6; the
reporter has not yet identified a specific wheel containing this metadata or demonstrated the
downstream uv resolver behavior.

Current source confirms the behavior. `parse_version_in_expr` implements a deliberately limited,
version-aware interpretation by splitting the right-hand string on whitespace and parsing every
token as a PEP 440 version. A token such as `3.8,3.9,3.13` fails that parse, the expression is
ignored, and the resulting marker tree is `true`. The nearby marker-simplification tests explicitly
expect comma-delimited forms to simplify to true. This differs from PEP 508's substring semantics
and makes the reported evaluation incorrect.

The limitation originated in astral-sh/uv#6172. Its discussion explicitly considered
`python_full_version in "3.11,3.12,3.13"`, then chose strict whitespace-only support until more
edge cases arose. This report supplies such a case. No newer issue or pull request was found that
already tracks comma-delimited version membership.

## Draft response

Thanks for the focused reproduction. The current uv-pep508 implementation only supports a
whitespace-delimited, version-aware subset of `in` for version markers. That behavior was
introduced in astral-sh/uv#6172; its discussion explicitly considered comma-delimited values but
deferred them pending concrete ecosystem cases. For a comma-delimited value today, parsing the
specialized version list fails and the condition is discarded, producing `true`, which does not
preserve PEP 508 substring semantics.

This should remain open as a bug for the comma-delimited case. It is related to astral-sh/uv#21309,
but that report uses reversed operands and takes a different parser path. If possible, please add
the exact package name and version—or wheel URL—whose `Requires-Dist` contains this form; the crate
reproduction establishes the behavior, while that metadata would document the resolver impact.

## Classification

Classify astral-sh/uv#21310 as a bug. PEP 508 defines `in` for this expression as substring
matching, but uv-pep508's specialized version-list parser accepts only whitespace-separated PEP 440
versions. When commas make that parser fail, ignoring the condition and returning true for every
environment is incorrect behavior. The source comment documenting narrower semantics and the test
recording the fallback establish that this is a known limitation, not that the result is correct.

This is not a regression: astral-sh/uv#6172 never supported comma-separated strings. It is also not
a duplicate of astral-sh/uv#21309. That open issue has the same unconditional-true result and is
part of the broader limitation around valid PEP 508 version membership, but its trigger is a quoted
value on the left (`"3.11" in python_version`) and it follows the inverted-version parser path.
Supporting comma-delimited right-hand strings is a distinct case with separate parsing choices.

## Related

- astral-sh/uv#6172 — **Add support for `python_version in ...` markers** (merged pull request).
  This is the historical implementation of whitespace-delimited, version-aware membership. Its
  maintainer discussion explicitly raised comma-delimited version strings and deferred them while
  asking to see whether additional edge cases arose, making it the strongest design evidence for
  this report.
- astral-sh/uv#3683 — **Support `in` operators with `python_version` marker** (closed issue). This
  original bug tracked version membership being treated as arbitrary/true and was closed by
  astral-sh/uv#6172. Its example uses whitespace-delimited versions, so the implemented fix does
  not cover the present comma-delimited trigger.
- astral-sh/uv#21309 — **uv-pep508 parses value in python_version as universally true** (open
  issue). This adjacent report also shows a valid version-membership expression becoming
  unconditionally true, but it uses reversed operands and exercises a different parser path.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `python_version in`, `python_full_version in`, comma-delimited version examples,
`always true`, uv-pep508, and relevant parser identifiers. Conceptual queries covered version
membership and list markers, PEP 508 substring behavior, invalid markers becoming true,
version-aware normalization, and marker algebra. Fix-oriented inspection followed the historical
chain from astral-sh/uv#3675 through astral-sh/uv#3683 to astral-sh/uv#6172 and checked for newer
work mentioning astral-sh/uv#21310.

astral-sh/uv#3675 was inspected but omitted from the related list because it is a downstream
resolver symptom involving the whitespace-delimited `pathlib2` marker already fixed by
astral-sh/uv#6172. astral-sh/uv#20816 was also inspected after a literal comma-version search, but
it concerns conjunction semantics for comma-separated `project.requires-python` specifiers, not
PEP 508 marker membership. No current fixing or tracking pull request was found.
