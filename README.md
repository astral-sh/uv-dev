# PEP-263 source encoding line is not preserved in the correct position (2nd line) for a data script when using the `#!/bin/sh` re-director trick.

Issue: astral-sh/uv#21276

Classification: duplicate

## Summary

When `uv pip install` installs a wheel data script beginning with `#!python`, a sufficiently long
POSIX interpreter path causes uv to replace that placeholder with a three-line shell/Python
polyglot prelude. If the original second line is a PEP 263 encoding declaration, that declaration
moves to line 4. Python therefore does not recognize it, and non-UTF-8 source bytes produce a
`SyntaxError: Non-UTF-8 code ... but no encoding declared` error.

The implementation confirms the observable transformation: data scripts whose first bytes match
`#!python` are rewritten with `format_shebang`; for a POSIX shebang longer than 127 bytes, containing
a space, or made relocatable, `format_shebang` emits the three-line `#!/bin/sh` prelude before
copying the remainder of the original script.

The same underlying correctness problem is already tracked by astral-sh/uv#6489. That issue shows
the identical prelude breaking another position-sensitive Python construct (`from __future__` after
a module docstring). Its discussion explicitly identifies long shebangs and paths containing spaces
as triggers and considers a separate wrapper that would leave the original Python source intact.

## Draft response

Thanks for the detailed reproduction. This is another manifestation of astral-sh/uv#6489. In both
cases, uv's POSIX fallback replaces a non-simple Python shebang with a three-line `/bin/sh`/Python
polyglot prelude, which changes Python source whose placement at the beginning of the file is
significant. astral-sh/uv#6489 demonstrates that with `from __future__`; here, the PEP 263 encoding
declaration is moved from line 2 to line 4. That discussion also explicitly covers long shebangs and
paths with spaces. Let's centralize the launcher design and fix there, so I'm marking this as a
duplicate; the encoding-cookie reproduction is useful additional evidence for that issue.

## Classification

This is a duplicate of astral-sh/uv#6489. The immediate Python rule differs—PEP 263 requires an
encoding declaration on line 1 or 2, while the earlier report concerns the permitted position of a
future import—but both failures result from the same generated POSIX prelude changing the meaning
of position-sensitive source at the start of an installed script. The new report's long-path trigger
is also explicitly within the trigger set discussed in astral-sh/uv#6489, so that open issue is the
appropriate place to centralize the design and fix.

Without that existing issue, this would be a bug: the installed script is valid before uv rewrites
it and fails afterward. Duplicate takes precedence because the underlying open problem is already
tracked.

## Related

- astral-sh/uv#6489 — Open bug, “Scripts that use \"from future\" can result in syntax error when
  used with uvx.” It tracks the same three-line shell/Python prelude breaking source whose location
  at the beginning of the file matters. Maintainer discussion covers long shebangs, paths with
  spaces, relocatable environments, and preserving the original source behind a separate wrapper.

## Search evidence

Searches covered the exact identifiers and error fragments (`PEP-263`, `coding=windows-1252`, and
`no encoding declared`), observable behavior (`source encoding`, `#!python`, data scripts, and
`#!/bin/sh`), conceptual repository vocabulary (`shebang`, trampoline, polyglot, relocatable, long
interpreter paths, wrapper scripts, docstrings, and future imports), and fix-oriented searches of
open and closed issues and open, closed, and merged pull requests.

astral-sh/uv#16209 was a plausible adjacent result because its reproduction displays the same
generated prelude followed by a now-line-4 UTF-8 cookie. It was excluded from the related list
because it tracks BusyBox's handling of `realpath --`, not Python source-position semantics.
astral-sh/uv#12122 and astral-sh/uv#11847 were also excluded: their similar “no encoding declared”
errors occur on Windows when launcher executable bytes are interpreted as Python, not while
rewriting a POSIX wheel data script. No fixing pull request was linked from astral-sh/uv#6489 or
found as a closer match.
