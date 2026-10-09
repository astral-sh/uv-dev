# Separate runtime workspace ownership from cache cleanup

Issue: astral-sh/uv#22338

Classification: bug

## Summary

The reporter describes live PEP 517 backends losing their execution files. Killing only uv and then running plain `uv cache clean` removes a surviving backend's environment; its next write fails with `ENOENT`. A separate parallel-build failure reportedly drops another live backend's environment without any signal or cache-maintenance command.

astral-sh/uv#22171 adds the relevant build-directory pruning; astral-sh/uv#11694 and astral-sh/uv#15990 document earlier protection for running environments. astral-sh/uv#20116 and astral-sh/uv#19321 discuss related locking designs. None establishes safe workspace retention after uv dies or parallel builds are canceled.

## Reported conditions and requested behavior

- **Parent termination and cleanup:** macOS 26.6.2 arm64, Python 3.14.7, uv 0.12.23; parent-only `SIGKILL`, followed by plain `uv cache clean`, while the backend remains alive.
- **Pruning:** the reported reproduction used CI revision `013216c409a2ce74821f25d9d84ce0747a0301ba` of astral-sh/uv#22171. That is a PR revision, not a claimed released-version prune reproduction.
- **Parallel cancellation:** one build fails while another backend still needs its environment. The issue provides the observed outcome but does not attach the fixtures or exact invocation.
- **Storage and ownership proposal:** move execution sources, temporary environments, and reusable executable environments outside `UV_CACHE_DIR`; use an asynchronous owner for process shutdown, output collection, and workspace retention. Avoid runtime symlinks into evictable cache entries, create environments at their final paths, and retain uncertain legacy entries during migration.
- **Descendants:** retain a workspace when descendant lifetime is unknown, including detached descendants with closed output pipes. This is an acceptance criterion, not a separately documented reproduction. Automatic crash recovery is explicitly outside the proposal's scope.

The issue supplies no implementation or Linux/Windows runtime validation. Its macOS reproduction results have not been independently rerun for this handoff.

## Draft response

Your report exposes a lifetime-safety gap: build workspaces live in the cache, but its lock protects them only while uv holds it. astral-sh/uv#22171 adds these directories to pruning; the earlier locking fixes do not establish that surviving children have stopped. astral-sh/uv#20116 addresses pruning around running commands but is not a confirmed fix for these cases.

Please attach the minimal PEP 517 fixtures and commands for the parent-only SIGKILL and parallel-build failure reproductions for regression coverage. Separating runtime storage from the cache remains a design proposal, including how to retain workspaces when descendant lifetime is uncertain.

## Classification

Deleting a live backend's environment is incorrect behavior, even though the report proposes an architectural solution. Source inspection confirms cache-resident temporary build environments, unconditional pruning, and build futures whose cancellation can drop their workspace without awaiting child shutdown. The historical locking fix behind astral-sh/uv#11694 does not cover descendants outliving uv; no open duplicate for these lifetime failures was found. This is not recurrence of the abandoned-directory leak fixed by astral-sh/uv#22171: cache clean already exhibits the reported failure on 0.12.23, while that PR adds the prune exposure.

The correctness finding does not establish that the proposed architecture is the only acceptable fix. Keeping this issue open as a bug allows the lifetime failure to be tracked while storage layout, descendant retention, and migration receive separate design review.

## Related

- astral-sh/uv#22171 (pull request, merged) — Prune temporary build environments in `uv cache prune`. Merged on 2026-10-07 to resolve astral-sh/uv#22167. The final implementation removes every builds-v0 entry without checking child-process lifetime, extending the reported exposure to prune. Its test covers an abandoned directory, not a surviving backend.
- astral-sh/uv#22167 (issue, closed) — uv build leaves a temporary build environment after Ctrl+C. The precursor reports leftover build environments after both uv and its backend stop. Closed by astral-sh/uv#22171. The new issue concerns deleting environments while their users remain alive, so it is not a duplicate of this cleanup request.
- astral-sh/uv#11694 (issue, closed) — uv cache prune removes files from a running process executed with `uvx`. Historical report of the same harmful outcome: pruning a live tool environment breaks subsequent imports. A maintainer closed it in November 2025 because uv now held a cache lock. The new parent-death and build-cancellation cases expose a lifetime gap beyond that protection.
- astral-sh/uv#15990 (pull request, merged) — Retain the cache lock and temporary caches during `uv run` and `uvx`. The September 2025 fix retains cache resources throughout command execution, following the shared-lock introduction in astral-sh/uv#15888. It explains the protection behind closing astral-sh/uv#11694, but does not provide ownership that survives uv termination or canceled build futures.
- astral-sh/uv#20116 (pull request, open) — Add automatic pruning of unreferenced cache archives. Proposes releasing the global lock while protecting running environments and their cache symlinks with per-entry locks. Its description explicitly releases claims on process death; it does not establish retention for surviving descendants or cancellation of PEP 517 builds. Related design work, not a confirmed fix or duplicate.
- astral-sh/uv#19321 (issue, open) — Fine-grained cache locking. Canonical discussion for reducing cache-lock contention, linked by maintainers from astral-sh/uv#19317. Comments examine long-lived commands, external environments, cache-backed environments, and symlink dependencies. It addresses maintenance while uv remains alive, rather than the new ownership failures after termination or cancellation.

## Supporting evidence

Source inspected at checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`:

- `crates/uv-cache/src/lib.rs:336`: `Cache::venv_dir` and `Cache::build_dir` allocate `TempDir` instances inside `builds-v0`.
- `crates/uv-cache/src/lib.rs:531`, `crates/uv/src/commands/cache_clean.rs:33`, and `crates/uv/src/commands/cache_prune.rs:31`: normal operations hold a shared cache lock; clean/prune acquire an exclusive lock unless forced. This coordinates uv processes, not the lifetime of every descendant. No `--force` is needed in the reported parent-death case.
- `crates/uv-cache/src/lib.rs:583` clears the cache contents. At line 750, prune iterates over every build-bucket entry and removes it without a child-liveness check. The final implementation therefore retains the relevant behavior even though the reporter tested an earlier PR revision.
- `crates/uv-build-frontend/src/lib.rs:246` stores the temporary directory in `SourceBuild`. `PythonRunner::run_script` spawns a Tokio child at line 1301, drains its output at line 1341, and waits for the direct child at line 1355. It has no explicit cancellation shutdown owner or `kill_on_drop` configuration. Dropping a Tokio child handle does not terminate the process by default, while dropping the build's `TempDir` removes the environment.
- `crates/uv-installer/src/preparer.rs:64` uses `FuturesUnordered` for parallel preparation; `Preparer::prepare` uses `try_collect` at line 100. An error can drop pending build futures. This supplies a source-backed cancellation path consistent with the reported second failure; the reporter's exact invocation remains needed for a targeted regression fixture.
- `crates/uv/tests/build/cache_prune.rs:577` tests removal of a fabricated abandoned build directory. It does not exercise surviving children. The nearby `build_all_with_failure` test at `crates/uv/tests/build/build.rs:979` checks build outcomes, not continued workspace access by an orphaned backend.

Historical context: astral-sh/uv#15888 introduced shared/exclusive cache locking in September 2025, and astral-sh/uv#15990 retained the lock and temporary caches during command execution. A maintainer subsequently closed astral-sh/uv#11694 because the cache lock should prevent deletion of running environments. Those changes protect the uv lifetime; they do not demonstrate a previously working guarantee for orphaned descendants. astral-sh/uv#22171 merged on 2026-10-07 and closed the abandoned-environment report astral-sh/uv#22167. The plain-clean failure is reported on uv 0.12.23 from before that merge.

## Search coverage and exclusions

Used authenticated gh to search open/closed issues with literal terms including cache clean, cache prune, builds-v0, ENOENT, SIGKILL, parent, parallel build, cancellation, and 0.12.23; conceptual searches covered running-process deletion, orphaned backends, detached descendants, runtime storage, temporary environments outside the cache, and fine-grained locking. REST search was rate-limited and PR search returned unusable results, so also retrieved all 12,901 open/closed/merged PR titles and filtered for cleanup, pruning, locking, cancellation, subprocesses, signals, and temporary environments, then inspected candidate bodies, comments, reviews, and referenced discussions. Historical fixes included astral-sh/uv#15888, astral-sh/uv#15990, and astral-sh/uv#13017. Ruled out astral-sh/uv#22387 as a duplicate: it fixes deletion of the lock file during concurrent cleanup, not surviving-child ownership. Also distinguished astral-sh/uv#18962 (orphan cleanup/global venv management), astral-sh/uv#12003 (external symlink installations), and astral-sh/uv#3095 (exec-based command launching).

Candidate chains were followed through astral-sh/uv#19317 to astral-sh/uv#19321, through the closed autoprune proposal astral-sh/uv#17211 to astral-sh/uv#20116, and through signal-handling issue astral-sh/uv#12830 to astral-sh/uv#13017 and astral-sh/uv#3095. The signal-forwarding fix explicitly excludes parent-only `SIGKILL`; it is not evidence that this uncatchable-signal case was fixed and later regressed. The open age-based pruning prototype astral-sh/uv#21374 retains the main cache lock and changes eviction policy, without establishing descendant ownership.

In particular, astral-sh/uv#22387 preserves the cache directory and lock file to fix an unlink/acquisition race between concurrently running uv commands. It does not retain execution files after their owning uv process has died, so it should not be used to close this report as a duplicate.

## Next steps and validation limits

Request the minimal PEP 517 fixtures and exact commands for parent-only termination and sibling-build failure. Use separate regression scenarios for those failures and for detached descendants; verify delayed imports and writes as well as directory existence. Before implementation, decide how workspace ownership ends when descendant lifetime cannot be established and how existing cache-resident execution entries will be retained during migration.

This handoff is based on source inspection and GitHub discussion history. No runtime reproduction, build, or test suite was executed, and no implementation or cross-platform fix is claimed.
