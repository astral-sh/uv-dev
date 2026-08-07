# Unable to install `redis[hiredis]` correctly

Issue: astral-sh/uv#20996

Classification: question

## Summary

The reporter says that `uv sync` always downloads dependency version 2.0.0 when the project declares
`redis[hiredis]>=8.1.0`. The report is for uv 0.12.2 on Windows 11 x86-64 with Python 3.12.13 and
configures the Tsinghua PyPI mirror as the default index.

The attachment does not show uv resolving redis 2.0.0. Its project dependency annotation identifies
redis 8.1.0, while the open `redis/__init__.py` declares `__version__ = '2.0.0'`. That file content
exactly matches redis 2.0.0, but the adjacent package tree contains modules from modern redis, so the
visible environment or editor state appears mixed. The public redis 8.1.0 wheel declares
`__version__ = "8.1.0"` and has distribution metadata version 8.1.0. The Tsinghua simple index lists
the same wheel with the same SHA-256 hash as PyPI. This rules out the published redis 8.1.0 wheel
itself declaring version 2.0.0, but it does not establish how the reporter's state arose.

No duplicate or matching fix was found. The closest repository reports concern metadata remaining
present while installed package contents are incomplete, but their triggering sequences and final
file states differ materially.

## Draft response

The screenshot's dependency annotation shows redis 8.1.0, but the open
`redis/__init__.py` matches redis 2.0.0. The published redis 8.1.0 wheel contains
`__version__ = "8.1.0"`, and the configured mirror indexes that same artifact, so the screenshot
alone does not show uv resolving redis 2.0.0.

Could you reproduce this in a newly created environment and share the complete output of
`uv sync -vv` and
`uv run python -c "from importlib.metadata import version; import redis; print(version('redis')); print(redis.__version__); print(redis.__file__)"`?
Please also report whether `uv cache clean redis` followed by recreating the environment changes the
result. That will distinguish an on-disk install or cache problem from stale editor state.

## Classification

This is classified as a question because the available evidence does not yet establish incorrect uv
behavior. Confirmed facts are that the dependency asks for redis 8.1.0 or newer, the attachment's
dependency annotation reports 8.1.0, and the source displayed in the editor is redis 2.0.0's old
`__init__.py`. The requested redis 8.1.0 artifact is internally consistent on PyPI and the configured
mirror points to that same artifact.

The report does not include `uv sync` output, a reproduction from a new environment, an interpreter
check of the imported file and both version values, or the sequence that produced the environment.
An inconsistent environment, cache state, and stale editor content therefore remain distinct
possibilities; none is a confirmed root cause. If a fresh reproduction demonstrates that uv creates
the mixed on-disk package, the issue should be reclassified as a bug.

## Related

- astral-sh/uv#16468 (open), “uv remove followed by uv add leaves package in incomplete state with
  missing RECORD file and package files.” This is the closest active report of the same broad state
  mismatch: distribution metadata remains while the package contents are incomplete, preventing a
  normal reinstall. Its confirmed trigger is remove followed by add, and its result has missing
  files and a missing `RECORD`, not the mixture of old and new redis files shown here. The reports
  are not close enough to centralize without the missing reproduction.

- astral-sh/uv#16116 (closed), “Receive an incomplete environment when trying to install
  packaging==25.0.” This historical report also had distribution metadata alongside incomplete
  package contents, and `uv cache clean` repaired it. It reported missing files and a missing-RECORD
  warning rather than a stale older `__init__.py`, so it is related evidence rather than a duplicate.

## Search and supporting evidence

Literal searches covered `redis`, `hiredis`, `redis[hiredis]`, redis 8.1.0, version 2.0.0, and the
reported wrong-version wording across open and closed issues and open, closed, and merged pull
requests. Conceptual searches covered resolver candidate selection, custom indexes and mirrors,
distribution metadata versus package contents, stale or corrupted caches, hard-link behavior,
incomplete installations, missing `RECORD` files, reinstall behavior, and file or module collisions.
Fix-oriented searches included closed issues and merged or closed pull requests; no redis-specific
historical fix or matching pull request was found.

Especially plausible candidates inspected but excluded from the related list were:

- astral-sh/uv#8512 selects versions with incompatible `Requires-Python` metadata from Nexus. Here,
  the distribution metadata identifies the requested redis version and the discrepancy is in one
  displayed source file, so the observable behavior differs.
- astral-sh/uv#15357 and merged astral-sh/uv#13437 track collisions where two different
  distributions provide the same module; merged astral-sh/uv#15253 placed that warning behind
  preview. No second distribution providing `redis/__init__.py` is identified here.
- Closed astral-sh/uv#19430 proposed preserving files claimed by overlapping distributions on
  uninstall, but was not merged and has the same unestablished collision prerequisite.
- astral-sh/uv#14479 demonstrates that deliberately editing an installed file hard-linked from uv's
  cache can corrupt the cached package and survive `--reinstall`; this report gives no comparable
  file-mutation trigger.
- astral-sh/uv#10320 and astral-sh/uv#10738 mention `redis[hiredis]` only in unrelated project-build
  and dependency-sorting reports.
