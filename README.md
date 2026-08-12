# uv run resolves a console-script to a different, already-deleted project's stale cached venv

Issue: astral-sh/uv#21062

Classification: bug

## Summary

The reporter observes that `uv run --project slack-watcher slack-watcher-poll --help` discovers the
requested live project at path B, but then selects interpreter and environment metadata for a
deleted worktree at path A. In the affected cache, uv reportedly recreates part of A's `.venv`,
chooses A's console script, and fails because that recreated environment has no working Python.
The in-place failure is deterministic for the reporter, while `--no-cache` or deleting only
`interpreter-v4` makes the command use B correctly.

This behavior has not been independently reproduced from a fresh cache. The issue itself reports
that a new worktree create/sync/delete attempt behaved correctly and does not identify the event
that associated A's interpreter data with a lookup from B.

## Classification

The reported behavior would be a bug: a command targeting the live project B must not select or
recreate an environment belonging to a different, deleted project A. The reporter's trace and
cache workarounds implicate the interpreter cache, but they do not establish a cache-key collision,
freshness bug, or other root cause.

The report is not currently classified as a duplicate. astral-sh/uv#13444 and
astral-sh/uv#18510 concern stale interpreter attributes after replacing an environment at the same
path. astral-sh/uv#14160 was closed after astral-sh/uv#14331 added the canonical interpreter path to
the cache key. astral-sh/uv#21062 reports uv 0.12.2, which contains that fix, and two distinct
absolute project paths.

## Reproduction

Outcome: `needs_more_information`.

The report used uv 0.12.2 on macOS 26.6 x86_64 with Homebrew Python 3.14 and a project-local
`.python-version`. Triage used isolated files, configuration, and caches on Linux x86_64 with
system CPython 3.12.3. The scenario was checked with both uv 0.12.2 and the installed uv 0.12.3;
the 0.12.3 release notes do not list an interpreter-cache fix between those versions.

The fixture contained independent A and B projects with the same package name, an editable local
package, and a `[project.scripts]` console entry point. Both virtual environments symlinked through
their local `python3` to the same system interpreter. The essential sequence was:

```console
uv sync --project A/slack-watcher --python /usr/bin/python3
uv run --project A/slack-watcher stale-demo
uv sync --project B/slack-watcher --python /usr/bin/python3
uv run --project B/slack-watcher stale-demo
mv A/slack-watcher A/removed-slack-watcher
uv run --project B/slack-watcher stale-demo
```

With uv 0.12.2, the final trace read B's `.python-version`, reported a cached interpreter hit for
`B/slack-watcher/.venv/bin/python3`, used B's interpreter, and successfully ran B's console script.
The script shebang named `B/slack-watcher/.venv/bin/python`, and the original A path remained absent.
uv 0.12.3 behaved the same way. Five additional package projects were then independently created,
synced, run, and deleted while sharing the uv 0.12.2 cache; a subsequent run still used B and did
not recreate any deleted path.

This does not disprove the report because the triggering history or corrupt-cache state is missing.
A meaningful targeted reproduction now needs a redacted failing trace that identifies the cache
hit, the cache-entry filename and decoded timestamp/`sys_executable`/`sys_prefix`/`sys_path` fields,
and the absolute and canonical paths used to derive B's lookup key. The `readlink`/`realpath` results
and `pyvenv.cfg` for both venv interpreters, plus whether any paths were reused and whether uv
operations overlapped during worktree churn, would help reconstruct the state. A copy of the
affected cache entries, with private data removed, would permit checking whether B's key contains
A's payload; deliberately overwriting a fresh B entry with A's payload would not reproduce the
unknown trigger.

No existing integration test was found that exercises two unrelated project environments across a
shared interpreter cache after one project is deleted. The adjacent
`crates/uv/tests/project/run.rs` test `run_from_directory` verifies that `--project` uses the target
project's console script and `.python-version`, including recreation of that same project's removed
`.venv`; it does not cover cross-project stale interpreter metadata.

## Draft response

This would be incorrect behavior, but we could not recreate the cross-project cache association
from the described setup on uv 0.12.2 or 0.12.3. A fresh two-project cache and a bounded sequence of
additional project create/sync/run/delete cycles continued to select B's interpreter and console
script after A was removed.

Before clearing an affected cache again, could you provide a redacted full failing trace, the cache
entry reported as the hit, its decoded timestamp and interpreter path fields, and the absolute and
canonical paths for B's `.venv/bin/python3`? The two `pyvenv.cfg` files and any path reuse or
overlapping uv operations in the preceding worktree history would also be useful. This should show
whether B's lookup selected an unexpected key or whether the expected key already contains A's
payload.

## Related

- astral-sh/uv#18510 — Open bug. `uv run` reuses `interpreter-v4` metadata from an earlier
  `--system-site-packages` venv after deletion and recreation, and clearing the interpreter cache
  fixes it. Its confirmed cache dimension differs from the cross-project path association here.
- astral-sh/uv#18537 — Open pull request. Adds `pyvenv.cfg` freshness to the interpreter cache to
  fix astral-sh/uv#18510. It is relevant to stale venv metadata but does not claim to address
  metadata crossing two distinct absolute paths.
- astral-sh/uv#13444 — Open issue. Shows deleted and recreated venvs retaining stale interpreter
  attributes while sharing an underlying interpreter. The effect is at one venv path rather than
  selection of another project.
- astral-sh/uv#14160 — Closed issue. A historical `uv run` flake read cached information for the
  wrong Python and unnecessarily removed and recreated `.venv`.
- astral-sh/uv#14331 — Merged pull request. Fixed astral-sh/uv#14160 by including both the absolute
  and canonical interpreter paths in the query cache key.

## Search evidence

Repository searches covered `interpreter-v4`, `query_cached`, `uv run --project`, console scripts,
environment deletion and recreation, stale interpreter metadata, and worktrees. The current
`InterpreterInfo::query_cached` implementation derives the entry name from both the absolute and
canonical executable paths and validates the canonical executable timestamp. That explains why A
and B normally receive distinct entries, but source inspection alone does not explain or reproduce
the reported bad association.
