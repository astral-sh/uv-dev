# Periodic crash (SIGSEV) during `uv pip sync` operations

Issue: astral-sh/uv#21428

Classification: bug

## Summary

The reporter is running uv 0.12.6 (`x86_64-unknown-linux-gnu`) with Python 3.13 on an EL8 Linux system (kernel 4.18, glibc 2.28). A periodic `uv pip sync` crash produced a core whose active thread faults in glibc's `__pthread_detach` while the main thread waits in `pthread_join`; the other listed threads are waiting in `syscall`. The uv frames are not symbolized, so the trace confirms a native uv crash but does not establish which uv subsystem or dependency caused it.

No existing issue tracks the same failure closely enough to make this a duplicate. The strongest fix-oriented lead is astral-sh/uv#21401: uv 0.12.6 contains `astral_async_http_range_reader` 0.11.0, while uv 0.12.9 updates it to 0.11.1 to fix a potential use-after-free while reading metadata ranges from remote wheels. That mechanism could produce a native crash, but the available trace does not confirm that it applies here.

Two older reports establish a separate adjacent failure mode on Rocky Linux 8 during installation. In astral-sh/uv#3150 and astral-sh/uv#14462, Rayon worker-pool creation fails under resource pressure and uv then aborts or segfaults. Those reports contain explicit allocation, `ThreadPoolBuildError`, and `Resource temporarily unavailable` diagnostics. None appears in astral-sh/uv#21428, so they are not evidence of a shared root cause.

## Draft response

Thanks for the core-dump details. This confirms that the uv process is crashing, but the unsymbolized uv frames do not identify the cause. uv 0.12.9 includes astral-sh/uv#21401, which fixes a potential use-after-free in the HTTP range reader used for remote wheel metadata; we cannot determine from this trace whether that is the failure here.

Could you first retry with uv 0.12.9 or newer and report whether the crash recurs? If it does, please include the install source for the uv binary, the approximate failure frequency, the last output from `uv pip sync -vv` with credentials and private paths removed, whether the requirements use a private index or direct wheel URLs, and GDB output from `info files` and `thread apply all bt full` for the matching core and binary.

## Classification

This is a bug because a supported `uv pip sync` invocation terminates the uv process with SIGSEGV in `__pthread_detach`. A reproduction is not required to establish that the observed behavior is incorrect. It is not classified as a duplicate: no inspected issue or pull request has the same command, trigger, crash frame, and environment with a confirmed matching mechanism. In particular, astral-sh/uv#21401 is a pre-existing merged fix that may cover the failure but is not confirmed by this unsymbolized trace, while the related thread-pool issues fail earlier and report explicit resource exhaustion.

## Related

- astral-sh/uv#21401 — **Bump async_http_range_reader to 0.11.1** (merged pull request). This is the closest fix-oriented candidate. It replaced the range-reader version present in uv 0.12.6 and fixed a potential use-after-free when reading remote wheel metadata. The report's intermittent native crash could be consistent with memory unsafety, but no symbolized frame or reproduction ties this crash to the range reader.
- astral-sh/uv#14462 — **Threadpool initialization fails during basic builds on HPC cluster unless low `concurrent-builds` limit set** (open issue). This is an adjacent installer/threading failure on Rocky Linux 8. It differs because the abort follows explicit memory-allocation failures and a Rayon `ThreadPoolBuildError` with EAGAIN; astral-sh/uv#21428 reports none of those diagnostics.
- astral-sh/uv#3150 — **Segfault after Resource temporarily unavailable** (closed issue). This historical report is close in command, EL8/Rocky platform family, and final segmentation fault. Its identified trigger was failure to create the Rayon pool, and reducing `RAYON_NUM_THREADS` resolved it. The current core instead stops in `__pthread_detach` without a preceding resource error, so the same cause is not established.

## Search evidence

Literal searches covered `SIGSEGV`, `Segmentation fault`, `core dumped`, `__pthread_detach`, `pthread`, `main2`, `uv pip sync`, `uv pip install`, periodic/intermittent/sporadic crashes, uv 0.12.6, glibc 2.28, EL8/RHEL/Rocky 8, and `Resource temporarily unavailable`. Conceptual searches covered worker and thread-pool failures, Rayon, excessive concurrency, installer and cold-cache failures, mmap/range-reader lifetime problems, use-after-free, and wheel metadata. Closed issues, merged pull requests, and release notes from uv 0.12.5 through 0.12.9 were reviewed for version-specific fixes.

astral-sh/uv#21001 was an especially plausible version-specific lead because uv 0.12.6 first enabled profile-guided optimization for Linux x86-64 release binaries, but it was excluded from the related list because the report provides no comparison with a non-PGO release and no evidence connects PGO to the crash. Reports involving QEMU, iSH, riscv64-musl TLS, and subprocess signal reporting were also excluded because their platforms, triggers, or crashing processes differ materially.
