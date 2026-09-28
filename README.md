# Varying order of resolution-markers from the same `pyproject.toml`

Issue: astral-sh/uv#22040

Classification: bug

## Summary

The reported lockfile churn is reproducible. Starting from the lock produced by the maintenance
run in DigitalEarthSweden/digital-earth-sweden-community#167, a package-specific rioxarray upgrade
reorders one of four top-level `resolution-markers` without changing any package, version, or marker
expression. The resulting lockfile is byte-for-byte identical to the commit from
DigitalEarthSweden/digital-earth-sweden-community#168.

The behavior begins in uv 0.12.4. On the same input, uv 0.12.3 leaves the lockfile unchanged, while
uv 0.12.4 and the installed uv 0.12.13 produce the exact reported two-line diff. The uv 0.12.4
release notes associate the relevant initial-fork ordering change with astral-sh/uv#21000.

## Classification

This is a reproducible bug and a regression in uv 0.12.4. A package-specific operation creates
lockfile churn while preserving the exact same resolution markers and resolved packages. The issue
does not depend on the reporter's guessed uv 0.12.19 or Python 3.14: it reproduces with uv 0.12.4
and 0.12.13 using CPython 3.12.3 on Linux x86_64.

It is not a duplicate of astral-sh/uv#17747 based on the available evidence. That issue concerns a
freshly written lockfile being considered stale after a package-specific upgrade, whereas this
issue is a stable but command-dependent ordering difference.

## Reproduction

Outcome: **reproducible**.

Environment used for the primary replay:

- Linux x86_64
- installed `uv 0.12.13 (x86_64-unknown-linux-gnu)`
- CPython 3.12.3 at `/usr/bin/python3`
- isolated temporary worktrees and `UV_CACHE_DIR` directories

The fixture is the `pyproject.toml` and pre/post lock states from
DigitalEarthSweden/digital-earth-sweden-community#167 and
DigitalEarthSweden/digital-earth-sweden-community#168. Its relevant project settings are
`requires-python = ">=3.12"`, `rioxarray >= 0.21`, and `[tool.uv] exclude-newer = "7 days"`.

From the parent of the lock-maintenance commit, the following command reproduced the complete lock
from DigitalEarthSweden/digital-earth-sweden-community#167 byte-for-byte:

```console
$ UV_CACHE_DIR=/tmp/uv-issue-22040/cache-full uv lock --upgrade
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 220 packages in 182ms
```

Starting from that maintained lock, the targeted operation reproduced the complete lock from
DigitalEarthSweden/digital-earth-sweden-community#168 byte-for-byte:

```console
$ UV_CACHE_DIR=/tmp/uv-issue-22040/cache-partial uv lock --upgrade-package rioxarray
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 220 packages in 103ms
```

The second command changed only the order of one marker:

```diff
 resolution-markers = [
     "python_full_version >= '3.14' and platform_machine == 'ARM64' and sys_platform == 'win32'",
-    "python_full_version < '3.14' and platform_machine == 'ARM64' and sys_platform == 'win32'",
     "(python_full_version >= '3.14' and platform_machine != 'ARM64') or (python_full_version >= '3.14' and sys_platform != 'win32')",
+    "python_full_version < '3.14' and platform_machine == 'ARM64' and sys_platform == 'win32'",
     "(python_full_version < '3.14' and platform_machine != 'ARM64') or (python_full_version < '3.14' and sys_platform != 'win32')",
 ]
```

A version comparison used fresh copies of the maintained lock and the same command:

```console
$ uv-0.12.3 lock --upgrade-package rioxarray
Resolved 220 packages in 898ms
# no diff

$ uv-0.12.4 lock --upgrade-package rioxarray
Resolved 220 packages in 890ms
# the exact two-line marker reorder above
```

The installed uv 0.12.13 produced the same result as 0.12.4. No Python 3.14 interpreter was
needed; universal lock resolution generated the Python-version markers while uv itself ran under
Python 3.12.3.

Existing test coverage is adjacent but does not cover this failure. The gated integration test
`crates/uv/tests/lock/lock.rs::lock_fork_strategy_with_python_environments` verifies scheduling and
serialized order for initial forks supplied through `environments`. It does not compare a clean
full resolution with a package-specific resolution seeded from an existing lockfile, so it would
not detect this command-dependent reorder.

## Draft response

Thanks for the paired Renovate examples. We reproduced the marker-only diff from
DigitalEarthSweden/digital-earth-sweden-community#167 and
DigitalEarthSweden/digital-earth-sweden-community#168.

Starting from the maintenance lock, `uv lock --upgrade-package rioxarray` with uv 0.12.4 and 0.12.13
moves the same marker and changes nothing else. uv 0.12.3 leaves that lock unchanged, so this is a
regression in uv 0.12.4. The uv 0.12.4 release notes point to the initial-fork ordering change in
astral-sh/uv#21000. The existing fork-strategy integration test does not exercise stability between
full and package-specific upgrades.

## Related

- astral-sh/uv#21000 is the confirmed release boundary's directly associated change. It made
  initial forks from an existing lockfile follow Python-bound scheduling. The observed version
  comparison confirms the regression boundary, although source inspection alone is not being used
  to claim a more specific root cause.
- astral-sh/uv#20999 prompted astral-sh/uv#21000 and explains the intended fork-strategy behavior.
  It does not discuss stable marker serialization across full and partial upgrade modes.
- astral-sh/uv#17747 is the closest open adjacent report. A package-specific upgrade also affects
  resolution markers, but its lockfile remains perpetually stale and the proposed work in
  astral-sh/uv#17752 concerns marker-tree canonicalization rather than this observed ordering-only
  regression.

## Search coverage and ruled-out candidates

Searches covered `resolution-markers`, marker order and sorting, `--upgrade-package`, stable
lockfiles, initial-fork scheduling, `fork-strategy`, and full-versus-partial resolution. The
relevant uv 0.12.4 release notes, implementation, and integration test were inspected.

The following plausible results are not the same report:

- astral-sh/uv#10988 involved materially different marker expressions from different uv versions.
- astral-sh/uv#9296 involved duplicate `resolution-markers`, not a different order of the same set.
- astral-sh/uv#16839 involved a false dry-run message without a written lockfile diff.
- astral-sh/uv#17752 targets astral-sh/uv#17747's compound-disjunction canonicalization failure.
