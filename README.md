# `uv build --all-packages` deadlocks with a cold cache: wheel-lock waiters exhaust the tokio blocking pool

Issue: astral-sh/uv#22348

Classification: bug

## Summary

The reporter describes an intermittent indefinite hang in `uv build --all-packages` on uv 0.12.23, Linux x86_64, with 54 workspace members sharing `hatchling` and `hatch-vcs` build requirements. A fresh `UV_CACHE_DIR` or `UV_NO_CACHE=1` triggers the problem. All source-distribution announcements appear, but no build backend starts. The reporter observed about 530 exclusive wheel-lock wait messages, 514 threads, 541 open lock descriptors, and 11 locks held by the same process. The process remained unchanged beyond six minutes and in CI could survive until the six-hour job timeout.

The report includes a reproduction sketch, not a complete fixture or attached stack trace. Running members in separate processes with bounded parallelism and a shared cache reportedly avoids the hang. That workaround has not been independently tested.

The closest matches are astral-sh/uv#21379, astral-sh/uv#21372, and astral-sh/uv#16342, which introduced the relevant wheel locking, blocking extraction, and timeout behavior. astral-sh/uv#17512 is adjacent concurrency work. No duplicate was established.

## Draft response

This should not hang indefinitely. The source supports your starvation hypothesis: contended lock waits and wheel extraction share Tokio's blocking pool. Also, `uv build` waits for all package futures before printing errors, so the missing timeout message does not establish that the timer never fired.

Please share a minimal generated workspace and the resolved build-dependency versions so we can reproduce the scheduling failure and validate a regression test. Your bounded separate-process workaround is reasonable in the meantime.

## Classification

A valid workspace build hanging indefinitely is incorrect behavior. The checkout confirms unbounded package fan-out, blocking file-lock waits, wheel extraction on the same blocking pool, and delayed error reporting until all package futures complete. These support the reported starvation mechanism without independently reproducing it. Related historical fixes address different deadlocks or resource limits; there is no evidence this is a recurrence of a previously fixed starvation bug or a duplicate of an existing tracker.

The suggestions to poll locks asynchronously, deduplicate waiters, or limit build-environment setup are possible remedies for a correctness defect, not evidence that this is merely an enhancement request. The timeout observation needs careful wording: no printed timeout is not proof that Tokio's timer failed. Existing control flow can retain completed error results while another package remains pending.

The issue has no comments or cross-referenced PRs at the time of inspection. No matching prior starvation fix was established, so this is not classified as a regression of a closed original issue. The version introducing the reporter's exact failure has not been bisected.

## Related

- astral-sh/uv#21379 — Avoid duplicate concurrent wheel downloads (pull request, merged). Added the reported remote-wheel locking path on every platform: acquire the per-wheel lock before checking the HTTP cache and retain it through download, extraction, and publication. Released in uv 0.12.8, before the reported 0.12.23. This is relevant implementation history, not a fix for blocking-pool starvation.
- astral-sh/uv#21372 — Run streaming ZIP extraction in a blocking task (pull request, merged). Moved streaming ZIP extraction into one blocking task per archive, including wheel downloads; released in uv 0.12.9. Establishes that extraction needs the same pool used by contended file-lock waiters. It does not establish that the reporter's particular scheduling failure has been reproduced.
- astral-sh/uv#16342 — Add a 5 min default timeout for deadlocks (pull request, merged). Introduced the five-minute file-lock timeout in uv 0.9.16 to stop indefinite cache clean/prune lock waits. Directly relevant to the missing-timeout symptom, but wrapping a blocking task in a timeout does not cancel its running lock operation or bound wheel extraction.
- astral-sh/uv#17512 — Investigate why uv holds so many file handles open (issue, open). Discusses excessive simultaneous cache-lock handles and limiting work before the build semaphore. This is useful concurrency context, but tracks file-descriptor exhaustion and 'Too many open files', rather than wheel-lock waiters starving Tokio's blocking pool.

## Supporting evidence

Source observations refer to this checkout, whose changelog includes uv 0.13.0, rather than an execution of the reporter's uv 0.12.23 binary.

- `crates/uv-build-commands/src/lib.rs:448` starts package futures with `join_all`, without a bound at that fan-out. At line 670, each `build_package` initializes its own `SharedState`; the preparer's in-process download deduplication is therefore not shared across all package builds.
- `crates/uv-installer/src/preparer.rs:138` onward uses the supplied `InFlight` state to deduplicate preparation. The build command passes each package's separate state through its build dispatch.
- `crates/uv-fs/src/locked_file.rs:210` schedules even the initial nonblocking lock probe on the blocking pool and awaits it outside the timeout. At lines 231–239, the contended blocking lock operation is separately scheduled with `spawn_blocking` and its join handle is wrapped in `tokio::time::timeout`. The default is five minutes. Timing out that await does not terminate an already-running blocking lock operation.
- `crates/uv-distribution/src/distribution_database.rs:183` defines the per-wheel advisory lock. The streaming and downloaded-wheel paths acquire it at lines 787 and 982 and retain it through cache population. Seekable extraction uses `spawn_blocking` at line 1358.
- `crates/uv-extract/src/stream.rs:122` schedules streaming ZIP extraction on the blocking pool too. This means the dependency on that pool applies to streaming as well as seekable extraction.
- `crates/uv-build-commands/src/lib.rs:495` onward awaits the complete `join_all` result before rendering each package's error. Thus timed-out waiters can coexist with a still-pending command and no visible timeout error.
- `crates/uv/src/lib.rs:3115` constructs a current-thread Tokio runtime without overriding its blocking-thread limit. The reported thread count is consistent with the proposed pool saturation, but thread count alone does not prove which tasks hold or await pool capacity.

Together these establish a credible starvation path and explain why the per-lock timeout is not an overall command deadline. They do not independently prove the exact scheduling sequence in the reporter's process. No reproduction, build, or test was run during this read-only investigation.

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

Used authenticated gh for open/closed issues and open/closed/merged PRs. Searched command and cold-cache symptoms separately from timeout and proposed-cause terms: all-packages with hang/hangs/deadlock/concurrent, cold cache, UV_NO_CACHE, Waiting to acquire exclusive lock, UV_LOCK_TIMEOUT, lock_file, lock_wheel, spawn_blocking, blocking pool/threads, thread pool, starvation, 512, and lock contention. Added cache-label and concurrency terminology, then followed comments, timelines, referenced fixes, and release notes. REST search was rate-limited; PR keyword searches returned empty results even for known matching PRs. Supplemented GraphQL issue searches with a local search of titles/bodies from the 1,500 latest PRs in all states, dating back to 2026-07-23, and direct inspection of older referenced PRs. Ruled out astral-sh/uv#19321 and astral-sh/uv#20116 (cache-pruning lock lifetime), astral-sh/uv#10255 and astral-sh/uv#10258 (build cycles), astral-sh/uv#6691 and astral-sh/uv#6790 (synchronous runtime blocking), astral-sh/uv#11444 (cargo-zigbuild race), and astral-sh/uv#22051 (ext4 cache-revalidation stalls). No same-problem tracker or confirmed previous fix was found; PR search coverage has the stated limitations.

The report was decomposed before searching into: the all-workspace cold-cache build hang; missing visible lock-timeout failure; shared wheel-lock contention and suspected blocking-pool starvation; and proposed asynchronous waiting or bounded setup. Literal identifiers and symptoms were searched separately from possible causes. Broader searches removed Hatch-specific package names, the version, and Linux from the query. Repository vocabulary included cache locking, concurrency safety, build dependency cycles, thread-pool initialization, and file-descriptor limits.

## Next steps

Obtain a minimal generated workspace with the resolved build requirements and reproduce with an isolated cold cache. A deterministic regression test should force lock contention with limited blocking-pool capacity and check both forward progress and visible timeout/error behavior. Investigate nonblocking asynchronous lock waits or in-process waiter deduplication, and whether package setup needs a shared bound. Verify any fix against both streaming and seekable wheel extraction; reducing the lock timeout alone cannot cancel a running blocking lock operation.

Only this temporary issue-context README was updated. The checkout and GitHub were not modified.
