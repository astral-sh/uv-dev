# uv run resolves a console-script to a different, already-deleted project's stale cached venv

Issue: astral-sh/uv#21062

Classification: bug

## Summary

`uv run --project slack-watcher slack-watcher-poll --help` discovers the requested live project at path B, but then selects interpreter and environment metadata for a deleted worktree at path A. It recreates part of A's `.venv`, chooses A's console script, and fails because that recreated environment has no working Python. The failure is deterministic in the affected checkout, while `--no-cache` or deleting only `interpreter-v4` makes the command use B correctly.

The closest current reports, astral-sh/uv#13444 and astral-sh/uv#18510, establish that `interpreter-v4` can retain per-venv information after an environment is deleted and recreated. Their known triggers are changes at the same venv path, especially `system-site-packages`; they do not establish why metadata for one absolute project path would be returned for another. A historical `uv run` failure in astral-sh/uv#14160 also used cached metadata for the wrong interpreter and recreated an environment. astral-sh/uv#14331 fixed that collision by adding the canonical interpreter path to the absolute path in the cache key. This report is against uv 0.12.2, which contains that fix, and specifically observes two different absolute paths, so it is best treated as a new bug or regression in the same failure class rather than a confirmed duplicate.

## Draft response

This is incorrect behavior: `uv run --project` should not select or recreate an environment belonging to a different project path.

There are related interpreter-cache bugs in astral-sh/uv#13444 and astral-sh/uv#18510, and astral-sh/uv#18537 is adding `pyvenv.cfg` invalidation for the latter. Those cases reuse stale metadata after replacing a venv at the same path. We also previously fixed a wrong-interpreter cache collision in astral-sh/uv#14331 by including both the absolute and canonical interpreter paths in the key. The evidence here does not yet show that either known mechanism explains metadata crossing from A to B, so we should keep this report separate.

Before clearing the affected cache again, could you attach a redacted full trace from one failing invocation, the path and decoded contents of the cache entry reported as the hit, and the `readlink`/`realpath` results plus `pyvenv.cfg` for B's `.venv/bin/python3`? The cache-entry filename and its `sys_executable`, `sys_prefix`, `sys_path`, and timestamp fields are the most useful pieces. Please remove private URLs or other sensitive values. That should let us determine whether the lookup selected the wrong key or whether the correct key contains the wrong payload.

## Classification

This is a bug because the requested live project is discovered correctly, yet `uv run` executes a console script from an unrelated deleted project and recreates that deleted environment. The `--no-cache` and targeted `interpreter-v4` workarounds, together with decoded cached paths into A, provide strong evidence that the interpreter cache participates in the incorrect behavior. They do not prove the report's proposed cache-collision or freshness root cause.

It is not classified as a duplicate. astral-sh/uv#13444 and astral-sh/uv#18510 concern stale interpreter attributes after replacing an environment at the same path and have narrower known invalidation gaps. astral-sh/uv#14160 was closed after astral-sh/uv#14331 added the canonical path to the key; the current report occurs after that fix and across distinct absolute paths. If it is a recurrence of the older wrong-interpreter-cache failure, the repository's triage rules call for a new bug classification unless an open issue already tracks the regression.

## Related

- astral-sh/uv#18510 — Open bug. `uv run` reuses `interpreter-v4` metadata from an earlier `--system-site-packages` venv after deletion and recreation, and clearing the interpreter cache fixes it. It is the closest current symptom match, but its confirmed cache dimension differs from the cross-project path association here.
- astral-sh/uv#18537 — Open pull request. Adds `pyvenv.cfg` freshness to the interpreter cache to fix astral-sh/uv#18510. It is relevant to stale venv metadata but does not claim to address metadata crossing two distinct absolute paths.
- astral-sh/uv#13444 — Open issue. Shows deleted/recreated venvs retaining stale interpreter attributes while sharing an underlying interpreter, with `interpreter-v4` deletion as the workaround. The effect is stale manylinux/system-site-package state at one path, not selection of another project.
- astral-sh/uv#14160 — Closed issue. A historical `uv run` flake read cached information for the wrong Python and unnecessarily removed and recreated `.venv`, matching the broad failure class.
- astral-sh/uv#14331 — Merged pull request. Fixed astral-sh/uv#14160 by including the canonical path in the interpreter query cache key. Because uv 0.12.2 contains this change, the present distinct-path failure is regression-like but its exact mechanism remains unconfirmed.

## Search evidence

Literal searches covered `interpreter-v4`, `query_cached`, `uv run --project`, console scripts, `No such file or directory`, `UV_PROJECT_ENVIRONMENT`, `--no-cache`, `sys_prefix`, `sys_executable`, and worktrees across open and closed issues and open, closed, and merged pull requests. Conceptual searches covered wrong or different virtual environments, stale interpreter metadata, cache pollution and invalidation, venv deletion/recreation, shared or symlinked base interpreters, moved environments, cache-key collisions, and parallel environment mutation. Fix-oriented review included the history of `crates/uv-python/src/interpreter.rs`, the issue chains closed by astral-sh/uv#14331, and the open fix astral-sh/uv#18537.

Several plausible adjacent results were inspected and ruled out. astral-sh/uv#11454 is about console-script shebangs after relocating a venv; astral-sh/uv#13198 is about `--prefix` generating scripts for the selected base interpreter; astral-sh/uv#18163 and astral-sh/uv#18166 concern an omitted effective `platform_machine` cache dimension; astral-sh/uv#17780 concerns project discovery versus `--active`; and astral-sh/uv#13883 plus astral-sh/uv#14153 concern concurrent mutation of the same environment. None selects stale metadata from a separate deleted project path under the reported conditions.
