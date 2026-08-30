# `uv run --active --script` deletes the contents of a non-virtual-environment `VIRTUAL_ENV`

Issue: astral-sh/uv#21364

Classification: bug

## Summary

With `VIRTUAL_ENV` pointing to a non-empty directory that is not a virtual environment,
`uv run --active --script` and `uv sync --script --active` can delete the directory contents and
create a script virtual environment in its place. The reporter demonstrated this with an ordinary
directory and identified conda prefixes as a consequential real-world trigger. The replacement is
not reported at normal verbosity.

The current source supports the report. When script interpreter discovery decides that the selected
environment needs replacement, `ScriptEnvironment::get_or_init` passes its root directly to
`uv_fs::remove_virtualenv`. That helper handles links and removal ordering safely, but it does not
verify that a directory contains `pyvenv.cfg` before recursively removing it. In contrast,
`ProjectEnvironment::get_or_init` rejects a non-empty, non-virtual-environment directory before
replacement. Existing integration tests cover creating and reusing a missing `VIRTUAL_ENV` for an
active script environment, but do not cover a pre-existing non-venv directory.

No existing issue or pull request was found that tracks this script-specific safety gap. The closest
precedent is the safe-clear work for `uv venv`, while the open conda-support issue explains how users
are led to put a conda prefix in `VIRTUAL_ENV`.

## Draft response

Thanks for the detailed report. This is a bug. The current script-environment path can select
`VIRTUAL_ENV` with `--active` and, when it needs a different interpreter, remove that path without
first verifying that it is a virtual environment. That differs from project environment replacement
and from the non-virtual-directory protection added for `uv venv` in astral-sh/uv#19595.
astral-sh/uv#11315 makes the conda case particularly relevant, but it tracks `CONDA_PREFIX`
discovery rather than this deletion behavior.

The fix should make both `uv run --active --script` and `uv sync --script --active` refuse to replace
a non-empty path that cannot be identified as a virtual environment, with integration coverage
confirming the existing contents are preserved. I would keep the validation scoped to externally
selected script environments unless an audit shows every `remove_virtualenv` caller can safely adopt
the same rule, since that low-level helper is also used for uv-owned environments.

## Classification

This is a correctness and data-loss bug, not an enhancement or support question. The mechanism is
confirmed by the checked-out source: the script path performs unconditional removal after discovery
rejects the existing environment, the removal helper does not validate the virtual-environment
marker, and the corresponding project path has an explicit guard. GitHub currently also labels
astral-sh/uv#21364 as `bug`.

This is not a duplicate. astral-sh/uv#19395 and astral-sh/uv#19595 cover explicit `uv venv --clear`
behavior and apply their validation only to user-requested clearing in virtual-environment creation.
They do not cover the managed replacement performed directly by `ScriptEnvironment`. This is also
not a return of the relative-managed-Python bug in astral-sh/uv#16631: that issue concerned repeated
replacement of an otherwise valid venv due to path normalization and was fixed by
astral-sh/uv#18398.

## Related

- astral-sh/uv#19395 — Closed bug with the same destructive outcome for a non-venv directory, but
  triggered by explicit `uv venv --clear` rather than implicit script-environment replacement.
- astral-sh/uv#19595 — Merged fix for astral-sh/uv#19395. It established refusal, with an explicit
  force escape hatch, when `uv venv --clear` targets a non-virtual-environment directory. Its guard
  is scoped to `RemovalReason::UserRequest`, so it does not protect the script path.
- astral-sh/uv#11315 — Open enhancement for native conda support in `--active`. Its discussion
  documents the `VIRTUAL_ENV=$CONDA_PREFIX` workaround and the risk of pruning conda environments,
  but it does not track arbitrary-directory deletion by the script interface.
- astral-sh/uv#15007 — Merged internal change at the exact script removal site. It replaced a raw
  `remove_dir_all` with `remove_virtualenv` but retained unconditional replacement, so it explains
  the current helper call without having fixed this safety gap.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `--active --script`, `VIRTUAL_ENV`, `Removed virtual environment`, `script environment`,
`pyvenv.cfg`, `remove_virtualenv`, `non-virtual-environment`, and the reported invalid-environment
fragments. Conceptual queries covered arbitrary-directory deletion, data loss, unsafe virtualenv
clearing, active-environment recreation, and conda prefixes. Fix-oriented searches followed the
comments and linked changes for astral-sh/uv#11315, astral-sh/uv#14985, astral-sh/uv#15474,
astral-sh/uv#16631, and astral-sh/uv#19395.

astral-sh/uv#16631 was the most plausible command-level false positive. It was ruled out because it
recreated a valid active venv after a relative `UV_PYTHON_INSTALL_DIR` caused managed-interpreter
detection to fail; astral-sh/uv#18398 fixed that path-normalization bug. astral-sh/uv#14985 and its
closing astral-sh/uv#16203 concerned consistently using the removal helper, not validating arbitrary
script roots.
