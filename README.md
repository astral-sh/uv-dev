# `uv build --all-packages` deadlocks with a cold cache: wheel-lock waiters exhaust the tokio blocking pool

Issue: astral-sh/uv#22348

Classification: bug

## Summary

The reported cold-cache workspace build stall is reproducible with the installed uv 0.12.13 on a four-CPU Linux x86_64 runner using Python 3.12.3. The reporter used uv 0.12.23; that exact binary was not executed. A generated virtual workspace contains 54 independent packages, each using `hatchling` and `hatch-vcs` as build requirements. Delaying wheel responses by three seconds from a local index makes the otherwise timing-dependent failure reproducible without CPU stress or changes to uv.

The failing run printed all 54 source-distribution announcements and 530 wheel-lock wait messages, then settled at 514 threads, including 512 in the kernel's `locks_lock_inode_wait`, with no child processes, 541 open lock descriptors, and 11 held locks. All ten wheel responses were sent. With the default five-minute lock timeout, it remained unchanged until externally killed after 370.67 seconds (over six minutes), without printing a timeout error. An additional run with `UV_LOCK_TIMEOUT=5` also stalled until killed after 45.5 seconds. A control with 52 packages and the same response delay completed in 16.0 seconds. The original 54-package fixture also completed against public PyPI in 14.1 seconds, consistent with the report's timing dependence.

The report's six-hour CI duration and exact uv 0.12.23 environment were not independently tested. The reporter's workaround, separate uv processes with bounded parallelism and a shared cache, remains untested. No duplicate was established; the most relevant implementation history is astral-sh/uv#21379, astral-sh/uv#21372, and astral-sh/uv#16342.

## Classification

Bug: a valid workspace build stalls after wheel responses have completed, with all 512 blocking workers waiting on file locks. The 52-package control succeeds with the same packages and server behavior, supporting blocking-pool capacity as the relevant boundary. This is observed behavior, not a conclusion based only on source inspection.

Source inspection explains how contended wheel-lock waits and wheel extraction compete for the same pool, and how `join_all` can retain per-package errors while other packages remain pending. The missing visible timeout does not establish a failed Tokio timer. The exact queued extraction tasks were not captured in a runtime stack trace, so that part of the mechanism remains a source-supported explanation rather than directly observed task state.

The initial investigation found no comments or cross-referenced PRs on the issue, no duplicate, and no confirmed previous fix for this starvation failure. The reproduction predates the reporter's version but does not establish the first affected release or a regression boundary.

## Reproduction

**Outcome: reproducible.** Expected: finish all 54 source distributions and wheels, or return a visible error. Actual: no build backend starts and the command remains pending beyond its configured lock timeout until externally killed.

### Environment and fixture

- Installed executable: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`, discovered on `PATH`; `uv --version` reports `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Linux `6.17.0-1022-azure`, x86_64, four online CPUs; `/usr/bin/python` is Python 3.12.3. The report did not specify a Python version.
- All reproduction files, output directories, configuration directories, and caches are under `/tmp/uv-22348-3mh4q4yc`. Each scenario uses its own initially absent `UV_CACHE_DIR`; the `UV_NO_CACHE=1` alternative was not separately tested.
- Commands run with an explicit minimal environment (`PATH=/bin:/usr/bin`, `LANG=C.UTF-8`, temporary `TMPDIR`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, and `UV_CACHE_DIR`, and `UV_PYTHON_DOWNLOADS=never`), plus `--no-config`. No existing credentials or user configuration are required.
- The root `pyproject.toml` contains only `[tool.uv.workspace]` with `members = ["packages/*"]`. Each of 54 members, `package_00` through `package_53`, has an empty `src/package_NN/__init__.py` and the following manifest (substitute its own name):

```toml
[project]
name = "package_00"
version = "0.1.0"
requires-python = ">=3.12"

[build-system]
requires = ["hatchling", "hatch-vcs"]
build-backend = "hatchling.build"
```

There are no runtime dependencies, dependency groups, workspace sources, uv lockfile, dynamic VCS-version configuration, or build constraints. The command runs from the virtual root and selects all members. Group selection and frozen execution are not involved.

Resolved build requirements, identical in the successful PyPI baseline and delayed-index reproduction:

```text
hatchling==1.32.4
hatch-vcs==0.5.0
setuptools-scm==10.3.4
vcs-versioning==2.6.0
packaging==26.3
pathspec==1.1.1
pluggy==1.6.0
trove-classifiers==2026.9.21.13
setuptools==84.0.0
tomlkit==0.15.1
```

### Commands and timing control

The unmodified installed binary runs the equivalent of this command from the fixture root, with an empty cache outside the workspace:

```sh
UV_CACHE_DIR=/tmp/uv-22348-3mh4q4yc/new-cache UV_LOCK_TIMEOUT=5   uv --no-config build --all-packages --out-dir /tmp/uv-22348-3mh4q4yc/new-dist   --python /usr/bin/python --index-url http://127.0.0.1:PORT/simple -v
```

The local threaded HTTP server serves the ten authentic PyPI wheels above, downloaded using public PyPI JSON metadata and checked against its SHA-256 digests. Each `/simple/<name>/` page advertises its wheel and a PEP 658 `.metadata` sidecar extracted from that wheel. Index and metadata requests return immediately. Only GET requests ending in `.whl` sleep for three seconds **before sending response headers**, then serve the complete unmodified wheel. This allows lock waiters to accumulate before the lock holders can start extraction. All ten completed wheel responses were logged, excluding an indefinitely stalled server as the cause.

Retained replay harnesses and artifacts are in `/tmp/uv-22348-3mh4q4yc`: `workspace/`, `workspace-52/`, `mirror/`, `run.py`, `mirror_run.py`, `build-constraints.txt` (a record of resolved versions, not a command input), per-scenario `.log` and `.json` files, and `*-locks.txt` fdinfo captures. Use a new label for a new cold cache:

```sh
python /tmp/uv-22348-3mh4q4yc/mirror_run.py replay-short 45 3 --test-lock-timeout 5
python /tmp/uv-22348-3mh4q4yc/mirror_run.py replay-default 370 3
python /tmp/uv-22348-3mh4q4yc/mirror_run.py replay-control 45 3   --test-workspace /tmp/uv-22348-3mh4q4yc/workspace-52
```

The harness starts the index on an ephemeral local port, invokes the installed uv, records thread wait channels, children, open lock descriptors and log size every second, and kills only its own process group at the specified observation deadline. The deadline is an external observation bound, not a uv exit.

### Observed results

| Scenario | Observed result |
| --- | --- |
| 54 members, cold cache, public PyPI | Exit 0 in 14.08 seconds; 108 artifacts; 530 lock-wait messages. |
| 54 members, cold cache, three-second delayed wheels, `UV_LOCK_TIMEOUT=5` | Still hung at 45.50 seconds; externally killed. All 54 sdist announcements, 530 lock waits, no backend children or artifacts, no timeout error. Settled at 514 threads, with 512 in `locks_lock_inode_wait`, one in `futex_do_wait`, and one in `ep_poll`. |
| 54 members, cold cache, three-second delayed wheels, default five-minute timeout | Still hung at 370.67 seconds; externally killed. Same 514 stable threads, 512 kernel file-lock waiters, zero children, 541 lock descriptors, 54 sdist announcements, 530 lock waits, zero artifacts and zero timeout errors. |
| 52 members, cold cache, same three-second delayed wheels, default timeout | Exit 0 in 16.01 seconds; 104 artifacts; 510 lock-wait messages. Only workspace membership differs. |

In the failed run, 541 `.lock` descriptors remained open. `/proc/<pid>/fdinfo` showed 11 held advisory flock locks: one shared cache lock and ten exclusive wheel locks, all owned by the same uv PID. The short-timeout command log stayed at exactly 490,233 bytes after setup. The default-timeout command log similarly stayed at 496,118 bytes, including after five and six minutes; both runs held the same eleven locks. Short-lived extra threads expired, leaving the report's stable total of 514. These measurements directly establish exhausted lock-wait capacity; identifying every queued extraction task would require additional runtime instrumentation.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for workspace builds, lock timeouts, deadlocks, wheel locking and concurrent extraction. Build integration tests are currently under `crates/uv/tests/build/`, rather than `crates/uv/tests/it/build.rs`.

- `crates/uv/tests/build/build.rs::build_workspace` checks successful all-package source and wheel builds for a root and one packaged member, plus a virtual member that is not built. Its all-package invocation follows a successful single-member build and reuses that cache; it does not exercise cold-cache pool exhaustion.
- `crates/uv/tests/build/build.rs::build_all_with_failure` checks that a three-package workspace builds the valid packages and reports a setuptools backend failure for another. It does not cover a permanently pending build future or saturated blocking pool.
- `crates/uv/tests/build/cache_clean.rs::cache_timeout` holds an exclusive global cache lock and snapshots a visible error from `uv cache clean` with `UV_LOCK_TIMEOUT=1`. It covers an ordinary contended lock timeout, not wheel-lock starvation during concurrent workspace builds.
- `crates/uv/tests/build/cache.rs::all_files_except_record_use_archive_file_store_streaming_concurrent` installs one locally served wheel with installation concurrency set to four and verifies content-addressed file sharing. It does not create hundreds of waiters.

No existing test covering this starvation scenario was found. These modules are gated by `test-python` and `test-pypi` in `crates/uv/tests/build/main.rs`. No repository tests were added or run and no uv binary was built.

## Draft response

Reproduced with 54 generated workspace packages sharing hatchling and hatch-vcs, a fresh cache, and three-second wheel-response delays. The installed uv 0.12.13 settles at 514 threads, including 512 blocked on file locks, and remains stuck for over six minutes with the default five-minute lock timeout; shortening that timeout to five seconds also fails to restore progress or print an error. The same fixture reduced to 52 members completes. This confirms the reported stall and supports blocking-pool starvation; absence of a printed timeout does not prove Tokio's timer failed. The reporter's exact uv 0.12.23 binary and separate-process workaround were not tested.

## Related

- astral-sh/uv#21379 — Avoid duplicate concurrent wheel downloads (pull request, merged). Added the reported remote-wheel locking path on every platform: acquire the per-wheel lock before checking the HTTP cache and retain it through download, extraction, and publication. Released in uv 0.12.8, before the reported 0.12.23. This is relevant implementation history, not a fix for blocking-pool starvation.
- astral-sh/uv#21372 — Run streaming ZIP extraction in a blocking task (pull request, merged). Moved streaming ZIP extraction into one blocking task per archive, including wheel downloads; released in uv 0.12.9. Establishes that extraction needs the same pool used by contended file-lock waiters. It does not establish that the reporter's particular scheduling failure has been reproduced.
- astral-sh/uv#16342 — Add a 5 min default timeout for deadlocks (pull request, merged). Introduced the five-minute file-lock timeout in uv 0.9.16 to stop indefinite cache clean/prune lock waits. Directly relevant to the missing-timeout symptom, but wrapping a blocking task in a timeout does not cancel its running lock operation or bound wheel extraction.
- astral-sh/uv#17512 — Investigate why uv holds so many file handles open (issue, open). Discusses excessive simultaneous cache-lock handles and limiting work before the build semaphore. This is useful concurrency context, but tracks file-descriptor exhaustion and 'Too many open files', rather than wheel-lock waiters starving Tokio's blocking pool.

## Supporting evidence

Source observations refer to this checkout, whose changelog includes uv 0.13.0. Behavioral observations above use the installed uv 0.12.13; the reporter's uv 0.12.23 binary was not executed.

- `crates/uv-build-commands/src/lib.rs:448` starts package futures with `join_all`, without a bound at that fan-out. At line 670, each `build_package` initializes its own `SharedState`; the preparer's in-process download deduplication is therefore not shared across all package builds.
- `crates/uv-installer/src/preparer.rs:138` onward uses the supplied `InFlight` state to deduplicate preparation. The build command passes each package's separate state through its build dispatch.
- `crates/uv-fs/src/locked_file.rs:210` schedules even the initial nonblocking lock probe on the blocking pool and awaits it outside the timeout. At lines 231–239, the contended blocking lock operation is separately scheduled with `spawn_blocking` and its join handle is wrapped in `tokio::time::timeout`. The default is five minutes. Timing out that await does not terminate an already-running blocking lock operation.
- `crates/uv-distribution/src/distribution_database.rs:183` defines the per-wheel advisory lock. The streaming and downloaded-wheel paths acquire it at lines 787 and 982 and retain it through cache population. Seekable extraction uses `spawn_blocking` at line 1358.
- `crates/uv-extract/src/stream.rs:122` schedules streaming ZIP extraction on the blocking pool too. This means the dependency on that pool applies to streaming as well as seekable extraction.
- `crates/uv-build-commands/src/lib.rs:495` onward awaits the complete `join_all` result before rendering each package's error. Thus timed-out waiters can coexist with a still-pending command and no visible timeout error.
- `crates/uv/src/lib.rs:3115` constructs a current-thread Tokio runtime without overriding its blocking-thread limit. The reproduction directly observed 512 threads in the kernel's file-lock wait channel; this is stronger evidence than the total thread count alone.

Together with the controlled reproduction, these explain a credible starvation path and why the per-lock timeout is not an overall command deadline. The kernel lock waits and failed forward progress were observed directly; the identity of queued extraction tasks is inferred from these code paths. This does not establish the exact scheduling sequence in the reporter's original process.

## Release context

- astral-sh/uv#16342 merged on 2025-12-04; the file-lock timeout is listed in uv 0.9.16.
- astral-sh/uv#21379 merged on 2026-08-31; all-platform remote-wheel locking is listed in uv 0.12.8.
- astral-sh/uv#21372 merged on 2026-09-01; blocking streaming extraction is listed in uv 0.12.9.
- astral-sh/uv#21400 also shipped in uv 0.12.9 and extended all-platform locking to local and source-built wheel extraction. It is adjacent implementation history; the report's observed locks are for remote build requirements.
- The reported uv 0.12.23 was released on 2026-10-03, after these changes. Their presence does not identify a first affected release or establish a regression from an earlier starvation fix.

## Candidates inspected and distinguished

- astral-sh/uv#19321, astral-sh/uv#19317, and astral-sh/uv#20116 concern global cache locks held by long-running child processes and safe cache pruning. This report has no child processes or concurrent cache cleanup.
- astral-sh/uv#10255 and its merged fix astral-sh/uv#10258 concern cycles through build requirements. No dependency cycle is reported here.
- astral-sh/uv#6691 and its merged fix astral-sh/uv#6790 concern Git-resource deadlocks when synchronous lock acquisition blocks the async runtime. The fix made acquisition async; the new report concerns capacity exhaustion inside the blocking pool.
- astral-sh/uv#3724 and its merged fix astral-sh/uv#3987 concern missed notifications in `OnceMap`, a different synchronization mechanism.
- astral-sh/uv#11444 uses the same all-packages command, but maintainers identified a cargo-zigbuild wrapper race after backends had already started. It was fixed upstream.
- astral-sh/uv#14462 concerns failure to create Rayon's installation thread pool under resource limits, with an explicit error, rather than successful thread creation followed by a wheel-lock stall.
- astral-sh/uv#21851 and astral-sh/uv#21607 address file-descriptor exhaustion, following the adjacent discussion in astral-sh/uv#17512. Limiting concurrency is shared vocabulary; their stated failures and limits differ from the blocking-pool deadlock.
- astral-sh/uv#22051 reverted astral-sh/uv#21675 while investigating ext4 HTTP-cache revalidation stalls; its description explicitly says causality and effectiveness were unconfirmed. The revert shipped before uv 0.12.23 and is not evidence of a previous fix for wheel-lock starvation.
- astral-sh/uv#22134 changes Windows lock filenames, and astral-sh/uv#22387 protects the cache lock during cleanup. Neither changes contended-wait scheduling.
- astral-sh/uv#16937 concerns Nexus metadata parsing, astral-sh/uv#16779 concerns expensive conflict-marker processing, and astral-sh/uv#19674 reports filesystem clone errors on a cold cache. None matches the observed wheel-lock waiters.

## Search scope and limitations

The prior related-issue investigation used authenticated gh for open/closed issues and open/closed/merged PRs. Searched command and cold-cache symptoms separately from timeout and proposed-cause terms: all-packages with hang/hangs/deadlock/concurrent, cold cache, UV_NO_CACHE, Waiting to acquire exclusive lock, UV_LOCK_TIMEOUT, lock_file, lock_wheel, spawn_blocking, blocking pool/threads, thread pool, starvation, 512, and lock contention. Added cache-label and concurrency terminology, then followed comments, timelines, referenced fixes, and release notes. REST search was rate-limited; PR keyword searches returned empty results even for known matching PRs. Supplemented GraphQL issue searches with a local search of titles/bodies from the 1,500 latest PRs in all states, dating back to 2026-07-23, and direct inspection of older referenced PRs. Ruled out astral-sh/uv#19321 and astral-sh/uv#20116 (cache-pruning lock lifetime), astral-sh/uv#10255 and astral-sh/uv#10258 (build cycles), astral-sh/uv#6691 and astral-sh/uv#6790 (synchronous runtime blocking), astral-sh/uv#11444 (cargo-zigbuild race), and astral-sh/uv#22051 (ext4 cache-revalidation stalls). No same-problem tracker or confirmed previous fix was found; PR search coverage has the stated limitations.

The report was decomposed before searching into: the all-workspace cold-cache build hang; missing visible lock-timeout failure; shared wheel-lock contention and suspected blocking-pool starvation; and proposed asynchronous waiting or bounded setup. Literal identifiers and symptoms were searched separately from possible causes. Broader searches removed Hatch-specific package names, the version, and Linux from the query. Repository vocabulary included cache locking, concurrency safety, build dependency cycles, thread-pool initialization, and file-descriptor limits.

## Next steps

Use the generated delayed-index fixture to design a regression test that bounds completion and exercises wheel-lock contention before extraction starts. Investigate asynchronous nonblocking lock waits, in-process waiter deduplication, or bounded/shared build-environment setup. Check both streaming and seekable extraction paths. Reducing `UV_LOCK_TIMEOUT` alone did not restore progress in the reproduction. A version comparison can establish the first affected release; no last known-good release has been identified.

Only temporary reproduction files and this issue-context README were written. The repository checkout and GitHub were not modified. Pre-existing checkout changes were left untouched.
