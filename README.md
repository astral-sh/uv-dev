# `uv run --active --script` deletes the contents of a non-virtual-environment `VIRTUAL_ENV`

Issue: astral-sh/uv#21364

Classification: bug

## Summary

With `VIRTUAL_ENV` pointing to a non-empty directory that is not a virtual environment,
`uv run --active --script` and `uv sync --script --active` delete the directory contents and create
a script virtual environment in its place. This was reproduced with the installed uv 0.12.7 on
Linux, independently confirming the reporter's uv 0.12.7 result on macOS. The reporter identified
conda prefixes as a consequential real-world trigger. `uv run` gives no replacement or deletion
notice at normal verbosity; `uv sync` says that it is updating the script environment but does not
say that the existing directory contents were deleted.

The pre-fix source supports the report. When script interpreter discovery decided that the selected
environment needed replacement, `ScriptEnvironment::get_or_init` passed its root directly to
`uv_fs::remove_virtualenv`. That helper handles links and removal ordering safely, but it does not
verify that a directory contains `pyvenv.cfg` before recursively removing it. In contrast,
`ProjectEnvironment::get_or_init` rejects a non-empty, non-virtual-environment directory before
replacement. The fix now applies equivalent protection specifically to externally selected script
environments while retaining replacement for uv-owned cache environments.

No existing issue or pull request was found that tracks this script-specific safety gap. The closest
precedent is the safe-clear work for `uv venv`, while the open conda-support issue explains how users
are led to put a conda prefix in `VIRTUAL_ENV`.

## Reproduction

Outcome: **reproducible**.

The report used uv 0.12.7 on Darwin 25.5.0 arm64 with Python 3.13.13. A targeted reproduction used
the installed `uv 0.12.7 (x86_64-unknown-linux-gnu)` on Linux 6.17.0 x86_64 with system Python
3.12.3. All fixture, cache, and Python-install paths were isolated below `$RUNNER_TEMP`; Python
downloads were disabled.

The minimal fixture was a PEP 723 script with `requires-python = ">=3.11"`, no dependencies, and an
ordinary `target` directory containing `important.txt` and `subdir/nested.txt`. The essential
invocation was:

```console
VIRTUAL_ENV="$PWD/target" \
UV_CACHE_DIR="$PWD/cache" \
UV_PYTHON_INSTALL_DIR="$PWD/python" \
UV_PYTHON_DOWNLOADS=never \
uv run --active --script script.py
```

The command exited 0 and printed only `script completed`. Both sentinel files and `subdir` were
gone afterward, while `target` contained a new virtual environment including `bin`, `lib`, and
`pyvenv.cfg`. Repeating the fixture with a separate populated `sync-target` and
`uv sync --script script.py --active` also exited 0, deleted both sentinels, and replaced the
directory. Its normal output began `Updating script environment at: sync-target`, without stating
that the pre-existing contents had been deleted.

The expected safe behavior is to refuse to replace a non-empty directory that is not recognizable
as a virtual environment and preserve its contents, matching the project-environment protection
described in the report.

At reproduction time, nearby integration coverage did not exercise this unsafe input:

- `crates/uv/tests/project/run.rs`, `run_active_script_environment`, verifies that
  `uv run --active --script` creates a missing active environment and later replaces that valid
  environment for a different Python request.
- `crates/uv/tests/sync/sync.rs`, `sync_active_script_environment`, verifies the analogous create,
  reuse, and valid-environment replacement behavior for `uv sync --script --active`.

Neither test then pre-populated `VIRTUAL_ENV` as an ordinary non-virtual-environment directory or
asserted that unrelated contents were preserved. The parent regression subsequently added that
fixture and was updated as part of the fix described below.

## Fix

Outcome: **fixed**.

`ScriptInterpreter::root` now reports whether the selected root is uv-managed. Before removing an
incompatible script environment, `ScriptEnvironment::get_or_init` checks externally selected roots:
missing paths and empty directories remain usable, and paths containing `pyvenv.cfg` remain
replaceable, but a non-empty path that is not a virtual environment is rejected with a dedicated
script-environment error. Roots in uv's script-environment cache retain their existing managed
replacement behavior.

The parent regression in `crates/uv/tests/project/run.rs`, `run_active_script_environment`, was
updated from asserting deletion to snapshotting the refusal and asserting that `important.txt`
remains present. Its later explicit Python-version request was also corrected to expect the same
refusal and preservation, since the fixture remains an ordinary directory after the first command.
The neighboring `crates/uv/tests/sync/sync.rs` test `sync_active_script_environment` was inspected;
it covers missing-path creation, valid-environment reuse, and valid-environment replacement through
the same initializer, so it required no fixture or snapshot change and continues to pass.

A maintainer follow-up explicitly expressed interest in the reporter's broader alternative: moving
the virtual-environment validation into `remove_virtualenv` itself. This is useful implementation
direction, but not yet a final design decision. The current fix in astral-sh/uv-dev#924 instead
guards only externally selected script roots. Review should therefore determine whether the helper
can enforce the check for every caller without preventing intended removal of uv-owned environments,
or whether its API needs an explicit policy for validated external paths versus known managed paths.

Successful focused validation:

- `cargo test --package uv --test project run::run_active_script_environment -- --exact`
- `cargo test --package uv --test sync sync::sync_active_script_environment -- --exact`
- `cargo clippy --package uv --lib -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

## Draft response

Thanks for the detailed report. This is a bug. The current script-environment path can select
`VIRTUAL_ENV` with `--active` and, when it needs a different interpreter, remove that path without
first verifying that it is a virtual environment. That differs from project environment replacement
and from the non-virtual-directory protection added for `uv venv` in astral-sh/uv#19595.
astral-sh/uv#11315 makes the conda case particularly relevant, but it tracks `CONDA_PREFIX`
discovery rather than this deletion behavior.

The implemented fix makes both `uv run --active --script` and `uv sync --script --active` refuse to
replace a non-empty path that cannot be identified as a virtual environment. The validation is
scoped to externally selected script environments because the low-level removal helper is also used
for uv-owned environments that must remain replaceable.

## Classification

This is a reproducible correctness and data-loss bug, not an enhancement or support question. The
observed pre-fix commands deleted unrelated files. The confirmed cause was unconditional removal by
the script path after discovery rejected the existing environment, despite the removal helper not
validating the virtual-environment marker. The checkout now guards externally selected script roots
before that removal. GitHub currently also labels astral-sh/uv#21364 as `bug`.

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

Pull request: astral-sh/uv-dev#924
