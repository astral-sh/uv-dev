# error: No download found for request: cpython-3.6.0-windows-x86_64-none

Issue: astral-sh/uv#21208

Classification: duplicate

## Summary

The reporter asks uv to provide a managed CPython 3.6.0 download for 64-bit Windows after receiving
`error: No download found for request: cpython-3.6.0-windows-x86_64-none`. No command, uv version, or
additional reproduction was supplied.

The current managed-download metadata contains no CPython 3.6 entry for Windows x86_64; its oldest
CPython Windows x86_64 entries begin at 3.8.2. This is not evidence of a newly returned bug. The
closest open discussion, astral-sh/uv#20088, already requests expansion of the managed-download
matrix to older Python versions and additional platform combinations after the same lookup error.
Maintainers state there that they do not plan to expand historical coverage because the build matrix
already carries substantial compute, hosting, platform-limit, and maintenance costs.

The new maintainer response resolves the support question directly: uv does not support an exact
version as old as CPython 3.6.0 because it is far past end of life. The maintainer also directed the
reporter to astral-sh/uv#9452, the repository's issue-reporting guidance. That reference is procedural
rather than a canonical discussion of Python download coverage.

## Maintainer decision

Maintainer zanieb states that uv does not support versions as old as CPython 3.6.0 because it is very
end of life. No additional reproduction or investigation was requested. The response points to
astral-sh/uv#9452, which asks reporters to search for existing discussions and include the uv
version, operating system, command, verbose output, and a minimal reproduction when applicable.

## Classification

Duplicate of astral-sh/uv#20088. Both reports ask uv to add managed-download coverage for a
historical CPython version/platform tuple after `uv python install` cannot find a matching download.
The new report contributes the specific CPython 3.6.0 Windows x86_64 tuple, but it does not describe
a different underlying capability or establish that a previously supported build has regressed.

This is substantively an enhancement request, rather than a correctness bug, but the open canonical
request makes `duplicate` take precedence. uv's documented Tier 2 support for Python 3.6 means uv is
expected to work with that interpreter; it does not establish that uv provides managed builds for
every 3.6 patch/platform combination, and the maintainer has now explicitly said that the exact
3.6.0 version is too old to be supported. Historical evidence also cuts against a regression: the
separate Python 3.7 metadata regression in astral-sh/uv#8213 was restored by astral-sh/uv#8216,
while astral-sh/uv#13022 later intentionally removed old Python 3.7 managed downloads. No
corresponding history was found showing that uv previously offered
cpython-3.6.0-windows-x86_64-none.

## Related

- astral-sh/uv#20088 — Open issue and canonical match. It requests managed-download coverage for
  older CPython versions across additional architectures after the same `No download found for
  request` failure. Maintainer comments explain why expanding the historical build matrix is not
  currently planned.
- astral-sh/uv#9394 — Closed issue with the same observable failure for an exact CPython patch on
  Windows x86_64. Maintainers explained that the requested patch build had been skipped and was
  unlikely to be added, recommending a nearby patch release instead. Its Python 3.11 case differs
  because it did not concern an EOL minor line.
- astral-sh/uv#13022 — Merged pull request that intentionally omitted managed downloads for Python
  3.7. It provides historical policy evidence for pruning older EOL distributions; it did not add
  support for the new report and is not being treated as a fixing pull request.

## Search evidence

Literal searches covered the complete error text, `cpython-3.6.0-windows-x86_64-none`,
`cpython-3.6`, Python 3.6, Windows x86_64, and `uv python install`. Conceptual searches covered
managed-Python coverage, old and end-of-life Python support, skipped patch releases, platform
matrices, and python-build-standalone. Fix-oriented searches covered closed issues and merged pull
requests that restored, removed, or refreshed download metadata.

Several superficially similar results were excluded. astral-sh/uv#8213 and astral-sh/uv#8216 cover
a temporary Python 3.7 metadata regression, not absent CPython 3.6.0 coverage. astral-sh/uv#6089 was
release-metadata lag for a newly available upstream build. astral-sh/uv#4848 addressed the rendering
of this error, not whether a requested distribution exists.
