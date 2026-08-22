# Support a "latest Python 3.x" version request (like setup-python's "3.x")

Issue: astral-sh/uv#21274

Classification: duplicate

## Summary

The report requests a rolling Python selector, analogous to `actions/setup-python`'s `3.x`, that
always selects the newest stable Python 3 release rather than accepting an already-installed Python
that satisfies a broad major-version request. The desired selector would work with `uv python
install`, `uv sync --python`, and `.python-version`; today `3.x` is rejected as an invalid download
request, while `3` does not express the requested newest-release policy.

The same underlying capability is already tracked by astral-sh/uv#13535. Although that issue's
example is `uvx python@latest`, it points to uv's generic Python request parser, where current source
explicitly records the intended addition of a `PythonRequest::Latest` variant. The pre-existing open
implementation in astral-sh/uv#13873 modifies that shared parser and discovery layer. Its maintainer
discussion also addresses the central semantics raised here: use the newest stable downloadable
Python when downloads are enabled, and otherwise the newest stable interpreter present on the
machine. The broader command and `.python-version` coverage from this report should be captured on
that canonical discussion.

## Draft response

Thanks — the underlying latest-Python request is already tracked in astral-sh/uv#13535, with an open
implementation in astral-sh/uv#13873. Although the existing issue uses `uvx python@latest` as its
example, the implementation is in the shared Python request and discovery layer, and the review is
already discussing whether `latest` means the newest stable download or the newest stable local
interpreter. With downloads enabled, the current maintainer direction is the newest available stable
download; otherwise, the newest stable interpreter on the machine.

The `uv python install`, `uv sync --python`, and `.python-version` coverage you describe is useful
scope for that work. Please add those use cases to astral-sh/uv#13535 so the syntax and behavior can
be settled in one place.

## Classification

This is a duplicate because astral-sh/uv#13535 already tracks an explicit latest-stable Python
request, and astral-sh/uv#13873 is an open implementation created before this report. The existing
discussion covers the same underlying selection-policy question rather than merely sharing Python
version terminology. The new issue broadens the concrete consumers from `uvx` to `uv python
install`, `uv sync --python`, and `.python-version`, but that additional scope can be centralized in
the existing issue.

This is not a regression or a correctness bug: current broad version requests intentionally accept
an existing satisfying interpreter, and current source explicitly reports that a latest request is
not yet supported. The requested rolling selector is new functionality.

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
