# uv_pep508 parses value in python_version as universally true

Issue: astral-sh/uv#21309

Classification: bug

## Summary

The report demonstrates that `uv-pep508` evaluates the valid PEP 508 marker `"3.11" in
python_version` as true for Python 3.9, 3.10, and 3.11, while `packaging.markers` evaluates it as a
substring containment test and returns false, false, and true respectively. The reporter also notes
that the opposite operand order, `python_version in "3.11"`, varies with the marker environment.

The current parser explains the result. A version key on the left has specialized `in`/`not in`
handling, but a quoted value on the left and version key on the right is sent through inverted PEP
440 comparison handling. `in` and `not in` are not PEP 440 comparison operators, so that path
reports the expression as ignored and returns no expression. An ignored standalone expression
becomes the true marker tree, producing the reported universal result.

The closest historical work is astral-sh/uv#3683 and its implementation in astral-sh/uv#6172,
which added version-aware handling for `python_version in "..."`. That work deliberately treated a
whitespace-separated right-hand value as a list of exact versions for marker algebra. It did not
cover the reversed, quoted-value-left containment form reported here. No open issue or pull request
was found that already tracks this case.

## Draft response

Thanks for the clear reproduction. This is a bug in `uv-pep508`.

The specialized version-membership handling added for astral-sh/uv#3683 by astral-sh/uv#6172 only
applies when `python_version` is on the left, as in `python_version in "..."`. With the operands
reversed, the parser routes the expression through inverted PEP 440 comparison handling. Since `in`
is not a PEP 440 comparison operator, the expression is discarded, and a discarded standalone
marker currently evaluates as true.

The reversed form is valid PEP 508 containment and should not be dropped. The next step is to add
regression coverage for quoted-value-left `in` and `not in` expressions and preserve their
containment semantics during parsing and evaluation. The reproduction here is sufficient for that
work.

## Classification

This is a `bug`: a valid marker is accepted but evaluates incorrectly, and the current source
confirms why. It is not a duplicate of astral-sh/uv#3683 because that closed issue and its merged fix
only handle the opposite operand order. There is also no evidence that this is a regression of that
fix; the reversed form was outside its implemented scope.

## Related

- astral-sh/uv#3683 — Closed issue that canonically tracked support for `in` with
  `python_version`. It concerns `python_version in "2.6 2.7 3.2 3.3"`, so it is closely related but
  does not track the reversed expression in astral-sh/uv#21309.
- astral-sh/uv#6172 — Merged pull request that fixed astral-sh/uv#3683 by adding specialized,
  version-aware handling for a version key on the left of `in`/`not in`. Its discussion explicitly
  distinguishes that exact-version-list model from specification-level substring matching, and its
  implementation does not cover a quoted value on the left.

## Search evidence

Literal searches covered `"3.11" in python_version`, `in python_version`, `python_version in`, the
reported universal-true result, and the `uv-pep508`/`MarkerTree::evaluate` identifiers. Conceptual
searches covered operand inversion, containment, substring matching, arbitrary or ignored markers,
PEP 508 compatibility, version-aware marker handling, and marker simplification. Fix-oriented
searches covered closed issues and merged pull requests for Python-version `in` handling.

The chain from astral-sh/uv#3675 through astral-sh/uv#3681, astral-sh/uv#3683, and
astral-sh/uv#6172 was inspected. The first pair changed the fallback for arbitrary markers and the
second pair implemented the variable-left membership form; none covers quoted-value-left version
containment. astral-sh/uv#6168 is another reproduction of the same variable-left case and points
back to astral-sh/uv#3683. astral-sh/uv#3917 was also inspected because it compares uv and
`packaging` environment-marker evaluation, but it concerns ordered comparison of
`platform_release`, not membership or operand order, so it is not included as related.
