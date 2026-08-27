# uv_pep508 parses value in python_version as universally true

Issue: astral-sh/uv#21309

Classification: bug

## Summary

The report demonstrates that `uv-pep508` evaluates the valid PEP 508 marker `"3.11" in
python_version` as true for Python 3.9, 3.10, and 3.11, while `packaging.markers` evaluates it as a
substring containment test and returns false, false, and true respectively. The reporter also notes
that the opposite operand order, `python_version in "3.11"`, varies with the marker environment.

The pre-fix parser explains the result. A version key on the left had specialized `in`/`not in`
handling, but a quoted value on the left and version key on the right was sent through inverted PEP
440 comparison handling. `in` and `not in` are not PEP 440 comparison operators, so that path
reported the expression as ignored and returned no expression. An ignored standalone expression
became the true marker tree, producing the reported universal result.

The closest historical work is astral-sh/uv#3683 and its implementation in astral-sh/uv#6172,
which added version-aware handling for `python_version in "..."`. That work deliberately treated a
whitespace-separated right-hand value as a list of exact versions for marker algebra. It did not
cover the reversed, quoted-value-left containment form reported here. No open issue or pull request
was found that already tracks this case.

## Reproduction

Outcome: **reproducible**.

The report's direct API example was reconstructed in `/tmp` with `uv-pep508` loaded from the
`0.12.6` Git tag. On Linux x86_64 with Rust 1.98.0, this command (with Cargo's home and target
directory also isolated under `/tmp`) reproduced the reported result:

```console
$ CARGO_HOME=/tmp/uv-21309/cargo-home \
  CARGO_TARGET_DIR=/tmp/uv-21309/target \
  cargo run --quiet
python_version=3.9  -> true
python_version=3.10 -> true
python_version=3.11 -> true
```

The fixture parsed `"3.11" in python_version` with `MarkerTree::from_str`, constructed a
`MarkerEnvironment` for each displayed Python version, and called `MarkerTree::evaluate`. Changing
only the dependency to the current checkout's `crates/uv-pep508` at commit `0697445cfef3839748907ae52e3fba14de31e3da`
produced the same three `true` results.

The behavior is also visible through the installed `uv 0.12.6 (x86_64-unknown-linux-gnu)` executable
without relying solely on the direct API fixture:

```console
$ printf '%s\n' 'does-not-exist ; "3.11" in python_version' \
  | UV_CACHE_DIR=/tmp/uv-21309/cache uv pip compile --no-index --python-version 3.9 -
  × No solution found when resolving dependencies:
  ╰─▶ Because does-not-exist was not found in the provided package locations
      and you require does-not-exist, we can conclude that your requirements
      are unsatisfiable.
```

The same command failed by trying to resolve `does-not-exist` for targets 3.9, 3.10, and 3.11. As a
control, `does-not-exist ; python_version in "3.11"` was omitted for 3.9 and 3.10 (exit status 0)
and resolved for 3.11 (the expected no-index failure). Python 3.12.3 with `packaging` 24.0 evaluated
the reported reversed expression as false, false, and true for marker environments 3.9, 3.10, and
3.11 respectively.

There is no exact existing test for a quoted value on the left of `in` with a version marker.
`crates/uv-pep508/src/marker/tree.rs::test_version_in_evaluation` covers `python_version in
"..."` and `not in` with the version key on the left. The same file's
`test_marker_version_inverted` covers an ordered reversed comparison (`'3.6' > python_version`),
but not reversed containment. Those assertions were read and do not cover the behavior in
astral-sh/uv#21309.

## Fix

Outcome: **fixed**.

The parser now recognizes quoted-left `in` and `not in` expressions with version markers before
falling back to inverted PEP 440 comparisons. A dedicated version-containment marker node retains
the original version string for specification-level substring evaluation, preserves the expression
when converting marker trees back to text, and keeps the existing version-list and ordered PEP 440
paths unchanged. Resolver traversal treats this boolean expression conservatively for
`Requires-Python` bounds and includes its version parameter when generating universal lock markers.

The parent `compile_reversed_python_version_in_marker` integration test was first changed to expect
the Python 3.9 requirement to be omitted and failed with the reported unsatisfiable resolution. It
now passes. Existing direct marker coverage was extended for matching and non-matching `in`, `not
in`, substring behavior, and marker text round trips. The existing `lock_multiple_markers` test now
also confirms that the reversed marker is retained in dependency edges and `package.metadata`
rather than being erased from `uv.lock`.

Successful focused validation:

- `cargo test --package uv-pep508 test_marker_version_inverted`
- `cargo test --package uv --test pip_compile compile_reversed_python_version_in_marker`
- `cargo test --package uv --test lock --features test-universal lock_multiple_markers`
- `cargo check --package uv`
- `cargo +stable clippy --package uv-pep508 --package uv-resolver --all-targets -- -D warnings`
- `cargo +stable fmt --all -- --check`
- `git diff --check`

## Draft response

Thanks for the clear reproduction. This was reproducible as a bug in `uv-pep508` 0.12.6 and the
parent main checkout.

The specialized version-membership handling added for astral-sh/uv#3683 by astral-sh/uv#6172 only
applies when `python_version` is on the left, as in `python_version in "..."`. With the operands
reversed, the old parser routed the expression through inverted PEP 440 comparison handling. Since
`in` is not a PEP 440 comparison operator, the expression was discarded, and a discarded standalone
marker evaluated as true.

The reversed form is valid PEP 508 containment and should not be dropped. The fix gives reversed
version containment its own marker representation, evaluates it against the original environment
string, and preserves it through marker and lockfile serialization. Regression coverage now checks
both command behavior and the direct marker API.

## Classification

This is a `bug`: the pre-fix implementation accepted a valid marker but evaluated it incorrectly.
It is not a duplicate of astral-sh/uv#3683 because that closed issue and its merged fix only handle
the opposite operand order. There is also no evidence that this is a regression of that fix; the
reversed form was outside its implemented scope.

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

Pull request: https://github.com/astral-sh/uv-dev/pull/890
