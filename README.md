# Support a "latest Python 3.x" version request (like setup-python's "3.x")

Issue: astral-sh/uv#21274

Classification: duplicate

## Summary

The report requests a rolling Python selector, analogous to `actions/setup-python`'s `3.x`, that
always selects the newest stable Python 3 release rather than accepting an already-installed Python
that satisfies a broad major-version request. The desired selector would work with `uv python
install`, `uv sync --python`, and `.python-version`; today `3.x` is rejected as an invalid download
request, while `3` does not express the requested newest-release policy.

A maintainer has pointed out that bare `uv python install` is already the documented way to install
the latest stable Python. This supplies an existing solution for the install-only case and asks the
reporter to confirm whether it meets their needs. It does not by itself provide a persistent or
argument-level latest selector for `uv sync --python` or `.python-version`, so the key remaining
scope is whether those consumers should accept an explicit rolling request.

The same underlying capability is already tracked by astral-sh/uv#13535. Although that issue's
example is `uvx python@latest`, it points to uv's generic Python request parser, where current source
explicitly records the intended addition of a `PythonRequest::Latest` variant. The pre-existing open
implementation in astral-sh/uv#13873 modifies that shared parser and discovery layer. Its maintainer
discussion also addresses the central semantics raised here: use the newest stable downloadable
Python when downloads are enabled, and otherwise the newest stable interpreter present on the
machine. The broader command and `.python-version` coverage from this report should be captured on
that canonical discussion.

## Maintainer follow-up

The maintainer asked whether bare `uv python install` works for the reporter and cited the
documented “install the latest Python version” behavior. A reporter response would help establish
whether the request is satisfied by a two-step workflow or specifically requires a selector that
can be passed through `uv sync --python` and stored in `.python-version`.

## Classification

This is a duplicate because astral-sh/uv#13535 already tracks an explicit latest-stable Python
request, and astral-sh/uv#13873 is an open implementation created before this report. The existing
discussion covers the same underlying selection-policy question rather than merely sharing Python
version terminology. The new issue broadens the concrete consumers from `uvx` to `uv python
install`, `uv sync --python`, and `.python-version`, but that additional scope can be centralized in
the existing issue.

This is not a regression or a correctness bug: current broad version requests intentionally accept
an existing satisfying interpreter, and current source explicitly reports that a latest request is
not yet supported. Bare `uv python install` already handles installing the latest stable Python; the
requested reusable rolling selector remains new functionality.

## Related

- astral-sh/uv#13535 — Open issue, “Add support for `uvx python@latest`.” This is the canonical
  request for an explicit latest Python selector. It links directly to the shared parser's latest
  request TODO, and its discussion asks the same newest-download-versus-newest-installed question.
- astral-sh/uv#13873 — Open pull request, “Support python@latest for newest, stable, discoverable
  Python.” It closes astral-sh/uv#13535 and adds latest-stable handling in the shared Python request,
  discovery, and download code. Review remains open because the precise user experience and
  selection semantics need resolution.

## Search evidence

The report was decomposed into three claims: rejection of the literal `3.x` download request,
reuse of an installed interpreter for the broad `3` request, and a desired latest-stable selector
across installation, project sync, and version files. Searches covered open and closed issues plus
open, closed, and merged pull requests. Literal queries included `3.x`, `not a valid Python download
request`, `python install 3`, `sync --python 3`, `.python-version`, and `setup-python`; conceptual
queries included latest/newest stable Python, Python request and version-range terminology,
interpreter resolution, highest-version selection, and installed-versus-download preference.
Fix-oriented searches covered Python upgrade work and latest-patch behavior.

Two plausible adjacent discussions were inspected but ruled out as canonical matches.
astral-sh/uv#18284 asks for a command that rewrites a concrete `.python-version` pin within a
project's `requires-python` range, rather than a rolling selector. astral-sh/uv#7892 and its merged
implementation astral-sh/uv#13954 concern upgrading already-managed Python installations across
patch releases, explicitly not selecting the newest Python minor release. astral-sh/uv#16333 is
about lowest-versus-highest interpreter resolution from package metadata, not an explicit rolling
latest request.
