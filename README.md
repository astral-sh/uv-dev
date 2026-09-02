# Periodic crash (SIGSEV) during `uv pip sync` operations

Issue: astral-sh/uv#21428

Classification: bug

## Summary

The reporter sees periodic native crashes during `uv pip sync` on EL8 Linux (kernel 4.18, glibc 2.28, x86-64). The submitted core from uv 0.12.6 faults in glibc's `__pthread_detach` at `pthread_detach.c:49`; GDB cannot access the thread-control-block address passed to that function. The process has 74 threads: the original thread is waiting to join uv's `main2` thread, while approximately 72 worker threads remain alive in syscalls.

The reporter has no reliable reproducer, but has a core with the same backtrace from uv 0.11.15 in May 2026. This establishes that the problem predates uv 0.12.6 and rules out the Linux x86-64 profile-guided release build introduced in that version as the regression point.

A maintainer identified the root cause as an interaction between the runtime-shutdown behavior introduced by astral-sh/uv#4793 and glibc bug 19951. astral-sh/uv#4793 changed uv to call Tokio's `shutdown_background`, allowing runtime worker handles to be detached rather than waiting for all pending tasks. When `main2` drops those handles while workers are exiting, affected glibc versions can race in `pthread_detach`, free or unmap the thread control block, and then access it. That mechanism matches both the crash location and GDB's inaccessible thread address.

The upstream glibc fix is commit `5da15b15adab661c80e373b6af89be0b5fa5b3ad`, **nptl: Do not use pthread set_tid_address as state synchronization (BZ #19951)**. Its commit message confirms a use-after-free caused by using separate `joinid` and `cancelhandling` fields to synchronize `pthread_join`, `pthread_detach`, and thread exit. It replaces those fields with a single atomic thread-state machine and explicitly states that this avoids the `pthread_detach` race. The maintainer reports that the fix is included in glibc 2.43 and later.

## Impact and workarounds

- Confirmed affected combinations include uv 0.11.15 and uv 0.12.6 on the reporter's glibc 2.28 EL8 environment.
- The failure is intermittent because it depends on worker shutdown racing with handle detachment.
- The maintainer recommends uv's statically linked musl build on RHEL as a short-term workaround; it does not use the affected glibc pthread implementation.
- Upgrading to glibc 2.43 or later incorporates the upstream fix, though that is generally not practical on EL8.
- Whether uv should add its own mitigation remains an open maintainer decision.

## Classification

This remains a bug: a supported `uv pip sync` invocation can terminate uv with SIGSEGV. It is not a duplicate of another uv issue. The fault is now source-backed as an interaction between uv's background Tokio shutdown and an affected glibc thread-lifecycle implementation, rather than an unlocalized uv memory error.

## Related

- astral-sh/uv#4793 — **Avoid hangs before exiting CLI** (merged pull request). This introduced explicit `runtime.shutdown_background()` calls so uv would not wait for unnecessary pending HTTP tasks during shutdown. The resulting background worker detachment exposes the glibc `pthread_detach`/thread-exit race on affected systems.
- astral-sh/uv#14462 — **Threadpool initialization fails during basic builds on HPC cluster unless low `concurrent-builds` limit set** (open issue). This remains adjacent because it concerns many-thread installation failures on Rocky Linux 8, but its explicit allocation failure and `ThreadPoolBuildError`/EAGAIN mechanism is different.
- astral-sh/uv#3150 — **Segfault after Resource temporarily unavailable** (closed issue). This also involved installation on Rocky Linux 8, but the segfault followed failure to create the Rayon pool and was mitigated with `RAYON_NUM_THREADS`; it is not the `pthread_detach` race diagnosed here.

## Superseded leads

astral-sh/uv#21401 is no longer the leading explanation. Although it fixed a separate potential use-after-free in remote-wheel range reading, the same crash on uv 0.11.15 plus the exact glibc race matching the core make that range-reader issue unrelated to this failure. astral-sh/uv#21001 is likewise ruled out as the regression source because the crash predates its Linux x86-64 PGO release changes.

## Supporting sources

- The maintainer diagnosis is recorded in astral-sh/uv#21428's discussion.
- astral-sh/uv#4793's patch shows the change from implicitly dropping the Tokio runtime to explicitly calling `shutdown_background()` in both runtime paths.
- glibc bug 19951: https://sourceware.org/bugzilla/show_bug.cgi?id=19951
- Upstream glibc fix: https://github.com/bminor/glibc/commit/5da15b15adab661c80e373b6af89be0b5fa5b3ad
