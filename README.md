# fix: avoid panic on missing lockfile instead of proper error propagation

Issue: astral-sh/uv#21643

Classification: question

## Summary

The reporter asks whether replacing an unspecified `.unwrap()` in lockfile handling with
propagated, contextual errors would be suitable contributor work. They assert that a missing or
malformed lockfile can panic, but provide no exact source location, command, uv version, lockfile,
panic message, stack trace, platform, or project configuration. The suggested
`crates/uv/src/commands/lock.rs` “or similar” path does not exist in the current layout; project
lock handling is under `crates/uv/src/commands/project/`.

Representative missing, malformed, and non-file lockfile cases were tested with the installed uv
and all returned normal user-facing errors. Current source also handles a missing lockfile as
`Ok(None)`, propagates parse and other I/O failures, and converts the absent value to
`ProjectError::MissingLockfile` in locked or frozen modes. No reported panic was reproduced, but
the issue is too underspecified to target a particular alleged panic path.

Maintainer zanieb has now asked whether there is any actual situation in which the alleged panic
can occur, specifically noting that running uv without a lockfile is not expected to panic. The
reporter then confirmed that the report was based on a general `.unwrap()` pattern rather than a
pattern verified in uv, withdrew the concern, and said they intend to close the issue. They will
return with a minimal reproduction if they find a concrete reachable panic.

## Reproduction

Outcome: `needs_more_information`.

Discussion status: the reporter has not reproduced the behavior and now confirms that the alleged
uv code path was never verified. No further reproduction work is indicated unless they return with
a concrete case.

Environment:

- uv 0.12.13 (`x86_64-unknown-linux-gnu`)
- Linux x86_64, kernel 6.17.0-1022-azure
- CPython 3.12.3 at `/usr/bin/python3`
- Isolated project and uv cache under `/tmp`; commands used `--offline`
- Repository commit `c0df400a4cf4aad88f7f34bb2ac3ebb5a8f3839e`

Minimal project:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = []
```

With no `uv.lock`, each of these commands exited 2 and did not panic:

```console
$ uv lock --locked --offline
error: Unable to find lockfile at `uv.lock`, but `--locked` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.

$ uv lock --frozen --offline
error: Unable to find lockfile at `uv.lock`, but `--frozen` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.

$ uv sync --locked --offline
error: Unable to find lockfile at `uv.lock`, but `--locked` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.

$ uv sync --frozen --offline
error: Unable to find lockfile at `uv.lock`, but `--frozen` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.
```

Writing the single line `invalid` to `uv.lock` and rerunning all four commands produced a
`Failed to parse uv.lock` TOML error at line 1, column 8, again with exit code 2 and no panic.
Replacing `uv.lock` with a directory produced
`failed to read from file .../uv.lock: Is a directory (os error 21)` for both lock modes, also
without a panic.

Existing coverage was checked rather than inferred from test names:

- `crates/uv/tests/sync/sync.rs::locked` and `::frozen` construct projects without a lockfile
  and snapshot the same normal missing-lockfile errors from `uv sync --locked` and
  `uv sync --frozen`.
- `crates/uv/tests/lock/lock.rs::lock_frozen_errors_report_source` snapshots normal
  missing-lockfile errors from `uv lock --frozen`, `uv lock --check-exists`, and
  `UV_FROZEN=1 uv lock`.
- `crates/uv/tests/project/check.rs::check_no_sync_errors_on_invalid_lockfile` writes
  `invalid` to `uv.lock` and snapshots a propagated TOML parse error.
- `crates/uv-resolver/src/lock/mod.rs::missing_package_version_registry` verifies that the
  previously panicking malformed registry-package case is rejected during deserialization with
  `Package ... from a registry source has a missing version field`.

To construct a meaningful targeted reproduction, maintainers need the exact `.unwrap()` or panic
site, full uv command and arguments, uv version and installation source, operating system, working
directory and project/workspace configuration, exact `uv.lock` contents or missing-file setup,
and the complete panic output or backtrace. This is also the information requested by maintainer
zanieb before treating the proposed panic as reachable. The reporter has agreed to provide a
minimal reproduction if they discover such a path.

## Classification

Classify as `question`. The issue primarily asks whether a proposed cleanup is suitable
contributor work, while its premise does not establish incorrect current behavior. Current source,
integration coverage, and the representative commands above all show explicit fallible handling,
but the missing report details prevent excluding a different configuration-dependent path.
Maintainer zanieb's follow-up likewise asks the reporter to establish an actual situation where the
panic occurs; it does not confirm a bug or endorse the proposed cleanup as a scoped contribution.
The reporter's subsequent acknowledgment confirms that the premise was not based on verified uv
behavior and that the proposed work is being withdrawn.

This is not established as a duplicate of astral-sh/uv#19854. That issue tracked one precise
malformed-lockfile invariant violation and was closed by astral-sh/uv#19855; astral-sh/uv#21643
does not identify the same trigger, show that it regressed, or establish another reachable panic.

## Related

- astral-sh/uv#19854 — Closed issue with the closest concrete symptom. A registry-source package
  missing its `version` field caused `uv sync --frozen` to panic at
  `expect("version for registry source")`. Unlike astral-sh/uv#21643, it supplied a specific
  lockfile, command, version, and panic site.
- astral-sh/uv#19855 — Merged pull request that closed astral-sh/uv#19854. It added
  `MissingPackageVersion` validation during lockfile deserialization and a regression test,
  turning that precise malformed-lockfile case into a graceful parse error.

## Search and supporting evidence

`LockTarget::read_with_contents` in `crates/uv/src/commands/project/lock_target.rs` maps
`NotFound` to `Ok(None)`, parses present content through `Lock::from_toml`, and propagates
other I/O errors. `LockOperation::execute` in
`crates/uv/src/commands/project/lock.rs` maps `None` to
`ProjectError::MissingLockfile` in frozen and locked modes. The `.unwrap()` calls in
`lock_target.rs` operate on already-established script parent paths or lockfile filenames; no
file-read result is unwrapped there.

The prior context's related-issue search also inspected astral-sh/uv#15459 and ruled it out: it
reported a dynamic-version lockfile parse failure under `--frozen`, not a panic, and the reporter
could no longer reproduce it.
