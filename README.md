# Relative indexes in PEP 723 scripts are resolved against the current working directory

Issue: astral-sh/uv#21096

Classification: bug

## Summary

The report demonstrates that `uv run --offline /path/to/main.py` interprets a relative local flat
index declared in `main.py`'s PEP 723 metadata against the process working directory. The same
script succeeds when invoked from its own directory, but from another directory it attempts to read
that directory's `./links` and fails with `Failed to read --find-links directory`. The expected
behavior is for `./links` to refer to the directory beside `main.py`, independent of where the user
invokes `uv run`.

No existing issue or pull request tracks this exact PEP 723 runtime case. The closest precedent is
merged astral-sh/uv#10827, which fixed the analogous behavior for relative indexes and find-links
loaded from `pyproject.toml` and `uv.toml`. Open astral-sh/uv#15055 concerns the separate problem of
serializing relative flat indexes as absolute paths in `uv.lock`. Merged astral-sh/uv#9208 added
PEP 723 index support, but its test uses an HTTPS index and does not exercise relative local paths.

## Draft response

Thanks for the clear reproduction. This is a bug: a relative index declared in a file-backed PEP
723 block should be resolved from the script's directory, just as relative index and find-links
paths in `pyproject.toml` and `uv.toml` are resolved from their containing configuration file. The
current script-index path does not apply that rebasing. I could not find an existing issue or pull
request tracking this specific PEP 723 case. A focused regression test should run the script by path
from another working directory and verify that its relative flat index is still loaded from beside
the script.

## Classification

This is a correctness bug, not a request for new functionality. The meaning of metadata embedded in
a file should not change merely because the caller invokes that file from a different directory,
and the reproduction shows a valid local index becoming an unrelated missing path.

The checkout supports the reported mechanism. `Pep723ItemRef::directory` computes the containing
directory for a file-backed script, and dependency lowering receives that directory for relative
requirements and sources. In contrast, `Pep723ItemRef::indexes` returns the parsed script indexes
without calling `Index::relative_to` with the script directory. File-backed `pyproject.toml` and
`uv.toml` options do receive this rebasing when loaded. This asymmetry matches the exact path in the
reported error.

This is not a duplicate. astral-sh/uv#10827 fixed the analogous behavior only for filesystem
configuration, astral-sh/uv#15055 tracks lockfile representation rather than runtime path
resolution, and no open issue or pull request tracks PEP 723 relative-index rebasing. It is also not
a regression of a known PEP 723 fix: the original script-index coverage in astral-sh/uv#9208 tested
only an HTTPS URL.

## Related

- astral-sh/uv#10827 (merged pull request) — The closest precedent. It fixed the same
  `Failed to read --find-links directory` class of failure for relative paths in `pyproject.toml`
  and `uv.toml` by resolving them against the containing configuration file. Its stated scope and
  implementation did not include PEP 723 script metadata.
- astral-sh/uv#15055 (open issue) — Also concerns a relative `format = "flat"` index, but its
  failure is that `uv.lock` stores an absolute path and loses portability. It does not track choosing
  the wrong base directory while running a script.
- astral-sh/uv#9208 (merged pull request) — Added support for `[[tool.uv.index]]` in PEP 723
  scripts. Its regression test establishes the subsystem but uses `https://test.pypi.org/simple`,
  so it could not catch local path rebasing errors.

## Search coverage

Literal and conceptual searches covered PEP 723 and inline script metadata, `uv run` with a script
path, `[[tool.uv.index]]`, `format = "flat"`, `--find-links`, the exact read-directory error,
relative and local indexes, and current-working-directory versus script- or configuration-directory
resolution. Searches included open and closed issues and open, closed, and merged pull requests,
including historical fixes for script indexes, config-relative find-links, target workspace
discovery, and lockfile path preservation.

astral-sh/uv#11302 and merged astral-sh/uv#17423 were inspected but concern workspace and virtual
environment discovery from the script target. Open astral-sh/uv#12193 is specifically about
shebang/project discovery and command-line option paths. Closed astral-sh/uv#18687 concerns reading
configuration from an inaccessible current directory. These share the broader `uv run` invocation
shape but not the PEP 723 index-rebasing defect, so they are not listed as closest related items.
