# UV recreates the venv on every run.

Issue: astral-sh/uv#21066

Classification: bug

## Summary

On Ubuntu 22.04 with uv 0.12.3, `uv run` and `uv sync` repeatedly remove and recreate the project
`.venv`. The verbose log says the environment interpreter reports Python 3.12.2 while the
environment was created with 3.12.13. uv then discovers `/usr/local/bin/python3.12` as CPython
3.12.13 and recreates the environment from it, but `uv run python --version` still reports Python
3.12.2. The unresolved, incorrect behavior is that recreation does not eliminate the mismatch, so
the next invocation repeats it.

The report also contains two secondary observations. One invocation has an active environment at
`/mnt/hdd/Toolkits/Tools/AutotestServices/.venv` while the project being operated on is under
`/mnt/hdd/Repositories/autotests_git`; uv warns when `VIRTUAL_ENV` and the project environment are
different. Removal of the latter environment then fails because the process cannot delete
`.venv/CACHEDIR.TAG` (`Permission denied (os error 13)`). That permission failure can prevent
recovery, but it does not by itself explain why a successfully recreated environment still runs
Python 3.12.2.

## Draft response

The log confirms that uv recreates `.venv` because the interpreter in it reports Python 3.12.2
while the environment metadata says it was created with 3.12.13. What is not yet explained is why
recreating it from `/usr/local/bin/python3.12`, which uv identifies as 3.12.13, still leaves
`uv run python --version` reporting 3.12.2.

Could you run the following immediately after one recreation and share the output:
`/usr/local/bin/python3.12 -c 'import sys; print(sys.version); print(sys.executable); print(sys._base_executable)'`,
`.venv/bin/python -c 'import sys; print(sys.version); print(sys.executable); print(sys._base_executable)'`,
`readlink -f .venv/bin/python`, and `cat .venv/pyvenv.cfg`? Please also say whether `.venv` or the
repository is copied or mounted, and share the owner and mode of `.venv` and
`.venv/CACHEDIR.TAG`; the permission error means the current process cannot remove that file.

astral-sh/uv#16231 shows the same version-conflict message, but that case involved copying a
virtual environment between container stages with different Python patch versions. The paths in
this report also show an active environment under `/mnt/hdd/Toolkits/Tools/AutotestServices` while
another run operates under `/mnt/hdd/Repositories/autotests_git`; the `VIRTUAL_ENV` warning is
expected when those are different. Use `--active` only if the active environment is intentionally
the target.

## Classification

This is a bug because the supplied `uv run python --version` output establishes a recreation loop:
uv selects `/usr/local/bin/python3.12` as 3.12.13, recreates the project environment, and the
resulting command still runs 3.12.2. The repository code confirms that the recorded-versus-running
interpreter conflict is what makes project environment compatibility fail and triggers recreation.
The report does not yet establish why the new environment executes the older patch, and the
permission problem may have a separate environmental cause.

This is not a duplicate. No open issue or pull request found in the searches tracks a freshly
recreated environment continuing to execute the old Python patch. The closest closed report,
astral-sh/uv#16231, involved a virtual environment copied between container stages and was resolved
by ensuring the stage and final Python patch versions matched.

## Related

- astral-sh/uv#16231 — Closed issue with the identical interpreter-version conflict and
  remove/create sequence. Its environment had been copied between container stages with different
  Python patch versions; unlike astral-sh/uv#21066, it did not show a freshly recreated environment
  continuing to run the older interpreter.
- astral-sh/uv#16218 — Closed issue about environment recreation failing when `.venv` was a busy
  Docker mount point. Its fix avoided treating failure to remove the now-empty mount directory as
  fatal; it does not cover permission denial while deleting the child `.venv/CACHEDIR.TAG` here.
- astral-sh/uv#19928 — Merged pull request establishing that project environments backed by
  uv-managed minor-version links should survive transparent patch upgrades. The interpreter in
  astral-sh/uv#21066 is discovered at `/usr/local/bin/python3.12`, so that managed-Python mechanism
  does not directly cover this report.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used the interpreter-version conflict, `VIRTUAL_ENV` warning, `CACHEDIR.TAG`, permission denial, and
remove/create messages. Conceptual queries covered repeated environment recreation, `pyvenv.cfg`
metadata, Python patch upgrades, system interpreters, copied or mounted environments, and active
versus project environment paths. Fix-oriented searches checked recent transparent-upgrade and
`version_info` changes.

astral-sh/uv#19100 and its fix astral-sh/uv#19102 were ruled out because they concern an exact patch
pin incorrectly following an uv-managed mutable minor-version link. astral-sh/uv#19920,
astral-sh/uv#19925, and astral-sh/uv#1689 concern `pyvenv.cfg` formatting or truncation observed by
pre-commit, not a recreated environment whose interpreter remains on the old patch.
astral-sh/uv#7073 concerns an unnecessary warning for `--no-sync`; this report invokes `sync` and
`run` and shows different active and project locations.
