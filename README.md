# `uv lock` errors if `.venv` is not valid

Issue: astral-sh/uv#19832

Classification: bug

## Summary

`uv lock` fails when the project environment path, normally `.venv`, exists as a file or as a
nonempty directory that is not itself a virtual environment. The minimal reproduction places an
unrelated file in `.venv`; the reporter's real layout uses `.venv` as a container for separate
Python-version environments. In both cases, locking stops with an invalid-environment error instead
of discovering a compatible system or managed Python.

This is the canonical report for that behavior. The direct proposed fix is astral-sh/uv#19833. A
later request to bypass the failure with `uv lock --active`, astral-sh/uv#21009, was closed because a
maintainer preferred fixing the invalid-environment behavior here.

## Draft response

Thanks for the clear reproduction. `uv lock` should not fail just because the project environment
path exists but is not a valid virtual environment; locking can fall back to a compatible system or
managed Python without synchronizing `.venv`. We'll keep astral-sh/uv#19832 as the canonical
tracker. astral-sh/uv#19833 is the open proposed fix for that behavior. We do not plan to add
`--active` as the workaround; astral-sh/uv#21009 was closed in favor of fixing the
invalid-environment handling directly.

## Classification

This is a bug. The current project interpreter discovery code checks the configured project
environment before falling back to another Python. A nonempty directory with no Python executable,
or a project environment path that is not a directory, produces an error from that check. `uv lock`
uses this discovery path even though it does not synchronize the project environment. A maintainer
also stated in astral-sh/uv#21009 that `uv lock` should not fail when the environment is invalid.

No earlier issue was found that canonically tracks this exact `uv lock` failure. astral-sh/uv#19833
was opened in response to this report, so its existence does not make astral-sh/uv#19832 a
duplicate.

## Related

- astral-sh/uv#19833 (open pull request) — Directly proposes making `uv lock` treat an invalid
  project environment as unavailable, allowing interpreter discovery to fall back to a system or
  managed Python. It explicitly closes astral-sh/uv#19832.
- astral-sh/uv#21009 (closed issue) — Reproduces the same failure with versioned environments nested
  under `.venv` and requests the reporter's alternative `--active` bypass. A maintainer rejected
  that direction and closed it because astral-sh/uv#19832 is sufficient as the canonical tracker.
- astral-sh/uv#21010 (open pull request) — Implements the `--active` bypass proposed by
  astral-sh/uv#21009 for the same reproduction. It differs from the preferred direction recorded by
  the maintainer, which is to ignore an invalid environment during locking.

## Reproduction

Outcome: **reproducible**.

The report's minimal case was reconstructed under `$RUNNER_TEMP` with all uv cache and managed
Python paths confined to the same temporary directory. The installed executable was uv 0.12.3
(`x86_64-unknown-linux-gnu`) on Ubuntu Linux x86_64, with system CPython 3.12.3 available at
`/usr/bin/python3`. The reporter used uv 0.11.19 on Arch Linux x86_64 and did not provide a Python
version.

The temporary project contained:

```toml
[project]
name = "example"
version = "0"
requires-python = ">=3"
```

An otherwise unrelated nonempty `.venv` was then created and locking was run in the isolated
environment:

```console
mkdir .venv
touch .venv/bad
env -u VIRTUAL_ENV \
  UV_CACHE_DIR="$CASE/cache" \
  UV_PYTHON_INSTALL_DIR="$CASE/python" \
  UV_PYTHON_DOWNLOADS=never \
  uv lock
```

`uv lock` exited with status 2 and the reported error:

```text
error: Project virtual environment directory `$CASE/.venv` cannot be used because it is not a valid Python environment (no Python executable was found)
```

As a control, renaming `.venv` and rerunning the same command succeeded, selected
`/usr/bin/python3`, and resolved the one-package project. This confirms that the invalid `.venv`,
not an unavailable compatible interpreter or the project metadata, triggers the observed failure.

No integration test under `crates/uv/tests/lock/` currently covers locking with a nonempty invalid
project environment. The nearby `crates/uv/tests/sync/sync.rs` test
`sync_invalid_environment` uses the same kind of invalid `.venv`, but it asserts that `uv sync`
fails to protect unrelated directory contents; it does not cover the reported `uv lock` fallback
behavior.

## Supporting evidence

Literal searches covered the full error text and its distinctive fragments, including “Project
virtual environment directory,” “not a valid Python environment,” and “no Python executable was
found.” Conceptual searches covered `uv lock` interpreter discovery, fallback to system or managed
Python, active environments, nested or multiple virtual environments, and invalid project
environment paths. Searches included open and closed issues and open, closed, and merged pull
requests.

The strongest linked chains were inspected through their issue comments and associated pull
requests. astral-sh/uv#9423 and merged astral-sh/uv#9427 concern `uv sync` with an empty mounted
directory, not `uv lock` with a deliberately nonempty directory. astral-sh/uv#13986 and
astral-sh/uv#11219 produce similar invalid-environment errors after interrupted deletion or
concurrent creation, but their triggers and affected operations differ. astral-sh/uv#13235 concerns
`uv run --no-sync` invalidating an incompatible environment. astral-sh/uv#9906 discusses workflows
for multiple named environments but does not report this locking failure.
