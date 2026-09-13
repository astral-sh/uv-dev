# fix: avoid panic on missing lockfile instead of proper error propagation

Issue: astral-sh/uv#21643

Classification: question

## Summary

The reporter asks whether replacing an unspecified `.unwrap()` in lockfile handling with propagated,
contextual errors would be a suitable `good first issue`. They assert that a missing or malformed
lockfile can cause a panic, but provide no exact source location, command, uv version, lockfile,
panic message, stack trace, or reproduction. The suggested path,
`crates/uv/src/commands/lock.rs` “or similar,” does not identify a file in the current layout; the
project lock implementation is under `crates/uv/src/commands/project/`.

Current source and integration tests contradict the missing-lockfile claim as stated. `LockTarget`
reads a missing lockfile as `Ok(None)` and propagates other I/O and parse errors. Lock modes then
convert `None` into `ProjectError::MissingLockfile`. Integration snapshots cover missing `uv.lock`
for `uv sync --locked`, `uv sync --frozen`, `uv lock --locked`, `uv lock --frozen`,
`uv lock --check-exists`, project-run modes, and related commands, all with a normal user-facing
“Unable to find lockfile” error rather than a panic. The lock and sync suites also cover malformed
TOML and semantic lockfile errors as `Failed to parse uv.lock` failures.

The closest historical defect is astral-sh/uv#19854, where a specifically malformed registry
package without a `version` reached `expect("version for registry source")`. That issue included an
exact `uv sync --frozen` reproduction and was fixed by merged astral-sh/uv#19855. Current source
validates that condition during lockfile deserialization and retains its regression test.

## Draft response

Thanks for checking. The current lockfile read path already treats a missing file as `Ok(None)` and
commands such as `uv sync --locked` and `uv sync --frozen` return a normal “Unable to find
lockfile” error; malformed TOML is also propagated as a parse error. The closest concrete panic,
astral-sh/uv#19854, involved a registry package missing its `version` field and was fixed by
astral-sh/uv#19855.

Could you provide the exact `.unwrap()` location, uv command, uv version, lockfile contents, and
panic output for the remaining case? Without a concrete reachable path, there is not yet a scoped
issue to mark as `good first issue`.

## Classification

Classify as `question`. The issue primarily asks whether a proposed cleanup is suitable contributor
work, while its premise does not establish incorrect current behavior. The referenced location is
not specific, and there is no reproduction or observable panic to associate with any remaining
`.unwrap()`. More importantly, the current missing-lockfile read path and integration snapshots
already demonstrate explicit fallible handling and a clear error.

This is not a duplicate of astral-sh/uv#19854. That issue tracked one precise malformed-lockfile
invariant violation and was closed by astral-sh/uv#19855; astral-sh/uv#21643 does not identify the
same trigger, show that it regressed, or establish another reachable panic. If the reporter supplies
a different exact panic path, the classification can be revisited as a bug.

## Related

- astral-sh/uv#19854 — Closed issue with the closest concrete symptom. A registry-source package
  missing its `version` field caused `uv sync --frozen` to panic at
  `expect("version for registry source")`. Maintainer discussion agreed that this malformed input
  should not panic, while noting that no normal uv workflow was known to generate the malformed
  lockfile. Unlike astral-sh/uv#21643, it supplied a specific lockfile, command, version, and panic
  site.
- astral-sh/uv#19855 — Merged pull request that closed astral-sh/uv#19854. It added
  `MissingPackageVersion` validation during lockfile deserialization and a regression test, turning
  that precise malformed-lockfile case into a graceful parse error. It is evidence that the known
  malformed registry-package panic is already fixed, not evidence for the new report's unspecified
  missing-lockfile claim.

## Search and supporting evidence

GitHub searches covered open and closed issues and open, closed, and merged pull requests. Literal
queries included “missing lockfile,” “uv.lock missing panic,” “malformed lockfile,” “lockfile
panic,” “unwrap lock,” “Result::unwrap,” “lockfile not found,” “lockfile does not exist,” and exact
parse-error language. Conceptual queries covered graceful lockfile errors, absent files, malformed
input, panic/error propagation, and `good first issue` cleanup requests. Fix-oriented searches used
the known `version for registry source` panic text and reviewed the closing relationship, body,
comments, and changed files of astral-sh/uv#19854 and astral-sh/uv#19855.

astral-sh/uv#15459 was inspected as a superficially plausible missing-lockfile/parse-error result
and ruled out. It reported a dynamic-version lockfile parse failure under `--frozen`; maintainers
identified inconsistent uv versions as the likely explanation, and the reporter could no longer
reproduce it. It did not report a panic or an unhandled file-operation result.

Repository evidence checked alongside GitHub results:

- `LockTarget::read_with_contents` maps `NotFound` to `Ok(None)`, parses present content through
  `Lock::from_toml`, and propagates other errors.
- `LockOperation::execute` maps a missing lockfile to `ProjectError::MissingLockfile` for frozen and
  locked modes.
- Integration snapshots in the lock, sync, run, audit, and check suites assert clear errors for
  missing lockfiles; malformed lockfile snapshots assert structured parse errors.
- The validation and regression test introduced by astral-sh/uv#19855 remain in
  `crates/uv-resolver/src/lock/mod.rs`.
