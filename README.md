# UV recreates the venv on every run

Issue: astral-sh/uv#21066

Classification: bug report; reproduction needs more information

## Summary

On Ubuntu 22.04 amd64 with uv 0.12.3, the report says consecutive `uv sync` and `uv run`
invocations remove and recreate the project `.venv`. The log shows that `.venv` runs Python 3.12.2
while its `pyvenv.cfg` records 3.12.13. uv then discovers `/usr/local/bin/python3.12` as CPython
3.12.13, recreates `.venv` from that executable, but `uv run python --version` reportedly still
prints Python 3.12.2. That last transition is the unexplained part of the report.

The report also shows a separate invocation in `/mnt/hdd/Repositories/autotests_git` while
`VIRTUAL_ENV` points to `/mnt/hdd/Toolkits/Tools/AutotestServices/.venv`. uv warns that it will
ignore that different active environment, then cannot delete the project environment's
`.venv/CACHEDIR.TAG` because of a permission error.

## Classification

Keep this classified as a bug report, but the reported recreation loop is not yet confirmed by a
targeted reproduction. A version conflict between the running interpreter and `pyvenv.cfg` is
expected to trigger one recovery recreation. In the isolated tests below, that recreation repairs
the environment and subsequent commands reuse it.

The supplied output demonstrates the claimed loop but does not reveal why an environment newly
linked to `/usr/local/bin/python3.12`, which uv identifies as 3.12.13, executes 3.12.2. The current
evidence does not establish a uv root cause. Relevant omitted details include the type and link
targets of the system interpreter, how it was installed or upgraded, whether the interpreter or
project is on a copied or mounted filesystem, and the new environment's `pyvenv.cfg` and Python
runtime values immediately after recreation. The `CACHEDIR.TAG` error is consistent with
filesystem permissions but its ownership and mode are also omitted.

## Reproduction

Outcome: `needs_more_information`.

The targeted reproduction used the installed `uv 0.12.3 (x86_64-unknown-linux-gnu)`, matching the
report, on Ubuntu 24.04 x86_64. CPython 3.12.2 and 3.12.13, the project, the Python installation
directory, and all uv caches were isolated under `$RUNNER_TEMP`. The minimal project was:

```toml
[project]
name = "issue-21066-mismatch"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []
```

with `.python-version` containing `3.12`. The reported initial mismatch was reconstructed by
creating `.venv` with 3.12.2 and changing only its `version_info` from 3.12.2 to 3.12.13. A separate
3.12.13 interpreter was placed first on `PATH`, then the following commands were run:

```console
$ uv venv --python "$PYTHON_3122" .venv
$ PATH="$(dirname "$PYTHON_31213"):$PATH" uv sync -v
$ PATH="$(dirname "$PYTHON_31213"):$PATH" uv sync -v
$ PATH="$(dirname "$PYTHON_31213"):$PATH" uv run -v python --version
```

The first `uv sync` produced the same key diagnostic as the report:

```text
The interpreter in the project environment has a different version (3.12.2) than it was created with (3.12.13)
Using CPython 3.12.13 interpreter at: .../bin/python3.12
Removed virtual environment at: .venv
Creating virtual environment at: .venv
```

The second `uv sync` did not remove or create `.venv`; it reported that the environment satisfied
the Python 3.12 request. `uv run python --version` printed `Python 3.12.13`, `pyvenv.cfg` recorded
`version_info = 3.12.13`, and `.venv/bin/python` resolved to the selected 3.12.13 executable. An
ordinary environment created directly with 3.12.13 was likewise reused by two consecutive
`uv run -v python --version` invocations.

A second-project variant set `VIRTUAL_ENV` to the first project's `.venv`. It reproduced the
reported warning that the active and project environments differ, but uv created the second
project's own 3.12.13 environment once and reused it on the next `uv sync`. The warning therefore
does not reproduce the loop. A stale-interpreter-cache variant also did not reproduce it: after an
interpreter at a fixed path was replaced, uv's Unix `ctime` check invalidated the cache and uv
re-queried the executable at its actual version.

Existing integration coverage is
`crates/uv/tests/sync/sync.rs::sync_when_virtual_environment_incompatible_with_interpreter`. It
creates a Python 3.12 environment, makes both the legacy `version` field and the current
`version_info` field incompatible in turn, asserts that `uv sync` removes and recreates the
environment, and asserts that the recreated `pyvenv.cfg` records the actual 3.12 interpreter.
This covers one-time recovery from the mismatch, not a newly recreated environment continuing to
run the old patch version.

To construct the reported loop rather than only its initial state, the following evidence is still
needed immediately after one successful recreation and before another uv invocation:

```console
$ /usr/local/bin/python3.12 -I -c 'import sys; print(sys.version); print(sys.executable); print(sys._base_executable); print(sys.prefix); print(sys.base_prefix)'
$ .venv/bin/python -I -c 'import sys; print(sys.version); print(sys.executable); print(sys._base_executable); print(sys.prefix); print(sys.base_prefix)'
$ readlink -f /usr/local/bin/python3.12
$ readlink -f .venv/bin/python
$ cat .venv/pyvenv.cfg
$ stat -c '%N %i %s %y %z %U:%G %a' /usr/local/bin/python3.12 .venv .venv/bin/python .venv/CACHEDIR.TAG
$ findmnt -T /usr/local/bin/python3.12
$ findmnt -T .venv
```

Maintainers also need to know how `/usr/local/bin/python3.12` was installed or updated, whether the
repository or `.venv` is copied between machines, images, or stages, and whether either relevant
path is a bind mount, network mount, or shared volume. Two complete consecutive `uv -vv` logs from
the same working directory, plus the result with a fresh temporary `UV_CACHE_DIR`, would
distinguish a persistent interpreter/link problem from cached discovery. Ownership and mode of the
second project's `.venv` and `CACHEDIR.TAG` are needed to investigate the separate deletion error.

## Related

- astral-sh/uv#16231 — Closed report with the same version-conflict and remove/create sequence.
  Its environment was copied between container stages with different Python patch versions; it did
  not show a freshly recreated environment continuing to execute the older patch.
- astral-sh/uv#16218 — Closed report about environment recreation failing when `.venv` was a busy
  Docker mount point. Its fix does not cover permission denial while deleting a child
  `.venv/CACHEDIR.TAG`.
- astral-sh/uv#19928 — Merged change establishing that project environments backed by uv-managed
  minor-version links survive transparent patch upgrades. astral-sh/uv#21066 instead discovers a
  system interpreter at `/usr/local/bin/python3.12`.

Searches in the retained issue context covered open and closed issues and pull requests using the
exact conflict, `VIRTUAL_ENV`, `CACHEDIR.TAG`, permission, and recreation diagnostics, plus copied
or mounted environments, system interpreters, `pyvenv.cfg`, and patch upgrades. No related item
establishes the unexplained post-recreation 3.12.2 result in astral-sh/uv#21066.
