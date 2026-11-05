# Concurrent uvx runs on a cold cache each build their own environment, orphaning all but one

Issue: astral-sh/uv#23115

Classification: bug

## Summary

The reporter starts eight identical `uvx pycowsay --help` processes concurrently against a fresh cache and observes eight environment archives; subsequent sequential runs reuse one of them. Sequential cold-cache control runs reportedly create one environment. The issue reports the same behavior on uv 0.13.7 and 0.12.13, using macOS arm64, and with httpie and mcp-proxy-for-aws as additional tools.

The reporter separately describes cache growth from new dependency resolutions: 157 reportedly identical environments and 1,166 environments with differing resolutions, among 1,323 cached tool environments. The reported 235 GB total and approximately 202 GB environment footprint are reporter measurements, not independently verified physical-space accounting.

No exact duplicate found. astral-sh/uv#17399 covers a different script-creation race; merged astral-sh/uv#5509 explains the archive/link design. Open astral-sh/uv#20116 and the cache-management issues cover the separate retention and automatic-cleanup request.

## Report decomposition

- **Primary symptom:** concurrent successful invocations of the same tool create redundant cached environments; later invocations do not reuse the displaced archives.
- **Trigger and subsystem:** a cold shared cache, concurrent processes, `uvx` / `uv tool run`, cached environment materialization. MCP clients launching the same server per session are a motivating workload.
- **Expected capability:** coordinate creation for an identical resolved environment key, or safely reclaim redundant builds.
- **Separate retention request:** reclaim superseded tool/dependency resolutions automatically and make cleanup less dependent on users remembering a manual command.
- **Exact identifiers:** `archive-v0`, `environments-v2`, `pyvenv.cfg`, `CachedEnvironment`; reported uv versions 0.13.7 and 0.12.13. The report contains no failing-command error message.

## Draft response

The source supports the race you describe: concurrent cache misses can each build an environment, save it under a fresh archive ID, and replace the same cache link. There is no per-key creation lock in that path. This is a bug distinct from the inline-script failure in astral-sh/uv#17399.

The next step is to test concurrent creation for one resolved environment key and coordinate those builds so they reuse one environment. For now, run `uv cache prune` after stopping long-running `uvx` processes; pruning otherwise waits for them.

Cleanup of superseded resolutions is a separate retention concern tracked in astral-sh/uv#9790 and astral-sh/uv#5731. astral-sh/uv#20116 proposes automatic pruning, including unused cached tool environments, but remains unmerged.

## Classification

The checkout source permits concurrent misses for the same resolved environment key to build independently, persist under fresh archive IDs, and replace the same cache link without reclaiming the displaced archive. This establishes the reported redundant-build/orphan behavior without requiring a new reproduction. No inspected issue or PR tracks that specific uvx failure closely enough to centralize it. The script race in astral-sh/uv#17399 is distinct, and astral-sh/uv#5509 fixed in-use mutation rather than duplicate creation. The secondary retention request overlaps existing enhancements but does not change the primary bug classification.

## Related

- astral-sh/uv#17399 — How to avoid environment creation race conditions? (issue, open). Related creation race: parallel inline-script runs observe an incomplete environment, and a maintainer recommends locking creation and mutation. It concerns a shared script directory failing validation, whereas this report concerns successful uvx builds leaving redundant archives.
- astral-sh/uv#5509 — Add relocatable installs to support concurrency-safe cached environments (pull request, merged). Introduced building cached environments separately in archive-v0 and publishing them through a content-addressed symlink. Merged in July 2024 to fix in-use environment mutation tracked by astral-sh/uv#5503; it did not promise to deduplicate concurrent builds, so this report does not establish a regression of that fix.
- astral-sh/uv#20116 — Add automatic pruning of unreferenced cache archives (pull request, open). Directly addresses secondary accumulation: proposes preview-gated automatic cleanup of unreferenced archives and cached tool environments unused for a week, with protection for running commands. It remains unmerged and does not serialize same-key environment creation.
- astral-sh/uv#9790 — uv cache management strategies for always-growing caches? (issue, open). Tracks cache growth as dependency versions change and requests retention or eviction policies. A comment reports roughly 270 GB from AWS MCP servers, closely matching the secondary impact, but does not establish concurrent duplicate environment creation.
- astral-sh/uv#5731 — uv cache size limit (issue, open). Canonical size-limit and garbage-collection discussion, with MCP-related disk-growth reports and references to the open automatic-pruning proposal. Broader than preventing redundant builds or evicting superseded tool environments.
- astral-sh/uv#16761 — Support proactive cache pruning (issue, open). Requests configurable periodic pruning during normal commands because users forget manual cleanup. Matches the report's cleanup-discoverability concern, but neither identifies nor fixes the concurrent uvx build race.
- astral-sh/uv#13431 — Ephemeral environments are not ephemeral (issue, open). Reports persistent cached script environments and disk growth. Maintainers explain that ephemeral does not mean deleted after execution and point to astral-sh/uv#5731. Its environments are keyed by different script paths, unlike redundant uvx environments for one resolution.

## Supporting source evidence

- `crates/uv-tool-commands/src/run.rs:1189` calls `CachedEnvironment::from_spec` for cached tool execution.
- `crates/uv-environment-operations/src/environment.rs:238` constructs the cache entry from the interpreter hash and resolution hash. It checks the link, returns an existing environment if valid, otherwise creates and syncs a temporary environment. Through line 282 there is no per-key lock spanning lookup, construction, and publication, and no second lookup that discards a redundant build.
- `crates/uv-environment-operations/src/environment.rs:279` calls `Cache::persist` and returns the newly created archive root to the caller. Consequently, a process can execute from an archive even after another writer replaces the cache link; absence of a link alone is not proof the archive is unused by a running command.
- `crates/uv-cache/src/lib.rs:411` allocates a fresh `ArchiveId`, moves the completed directory into the archive bucket, and creates the cache link. The Unix link implementation at line 951 replaces the symlink. It does not remove the previous target. These operations support the reported same-key race directly; atomic publication prevents partially built environments from becoming visible, but does not deduplicate complete builds.
- `crates/uv-cache/src/lib.rs:712` removes cached environment entries during explicit pruning; line 807 then finds references and removes unreferenced archives. `crates/uv/src/commands/cache_prune.rs:29` acquires the exclusive cache lock or waits, consistent with pruning blocking behind long-running uv processes. This is cleanup, not coordination between concurrent cache writers.
- The reproduction's top-level `ls environments-v2` count measures interpreter-hash directories for this cached-tool layout, not the number of resolved environments. The source still supports one nested resolution link being repeatedly replaced. A regression test should inspect that nested link and its archive target instead of relying on the top-level count.
- `docs/concepts/cache.md:128` documents concurrent cache readers and writers; its clearing-cache section documents periodic explicit pruning. Source inspection establishes the creation gap; no commands from the issue body were executed, no affected release was reproduced, and no disk-footprint measurements were independently validated.

## Historical fixes and proposal status

astral-sh/uv#5503 was closed by astral-sh/uv#5509 on July 29, 2024. The problem then was mutation of an environment another process was already using. The merged change introduced independent archived environments with atomic link publication, the architecture relevant here. Neither its issue nor its description establishes a prior fix for duplicate concurrent builds; the new report should not be called a regression of that fix.

astral-sh/uv#17211 was closed without merging in July 2026 in favor of astral-sh/uv#20116. The replacement remains open, with no submitted reviews shown. Its current proposal includes preview-gated automatic pruning of unused cached tool environments after a week and protection for running commands. It is a proposal, not a capability available in the reported releases. The merged shared-cache locking work in astral-sh/uv#15888 protects running operations against cache removal; it does not make concurrent writers to one environment key mutually exclusive.

## Search coverage

Separated concurrent cold-cache duplication from superseded-version retention and automatic cleanup. Searched open/closed issues and open/closed/merged PRs using uvx/tool run, concurrent/parallel, cold cache, orphan/duplicate environments, environments-v2, archive-v0, CachedEnvironment, superseded/old versions, cache growth, locking, and auto/proactive pruning; also checked closed uvx/0.13.7/cache reports. PR keyword searches returned no entries, so supplemented them with all open PRs, the 400 most recently updated closed PRs including merges, and directly inspected historical references, comments, and reviews. Followed astral-sh/uv#5503 to astral-sh/uv#5509 and astral-sh/uv#5731 through closed astral-sh/uv#17211 to open astral-sh/uv#20116. Ruled out astral-sh/uv#7184 as an unreproduced duplicate-binary report, astral-sh/uv#21374 because its manual wheel-aging proposal explicitly retains cached environments, and the historical in-use-mutation fix as evidence of a deduplication regression.

### Inspected candidates and important differences

- astral-sh/uv#17399 remains a relevant adjacent race, but concerns a stable inline-script directory observed before it becomes a valid virtual environment. The reported uvx path builds separately and publishes complete environments, producing a different failure.
- astral-sh/uv#13883 is a broad locking audit, not an existing focused discussion of redundant uvx archives. astral-sh/uv#19321 focuses on lock granularity and contention, including cleanup blocked by running commands.
- astral-sh/uv#7184 reported duplicate binaries with uv 0.4.5, then closed after the reporter could no longer reproduce them. No same-key concurrent environment mechanism or fixing PR was established.
- astral-sh/uv#12525 reports slow concurrent project environment installation under Ray, not orphaned cached tool environments.
- astral-sh/uv#21374 is an open manual age-based wheel-pruning prototype whose body explicitly leaves cached environments cached. It does not implement either environment creation coordination or superseded-tool cleanup.
- astral-sh/uv#18962 requests broader wheel cleanup and global environment management; it does not establish this tool-cache race.

## Maintainer next step and validation

Keep the concurrent cached-tool creation bug focused on lookup/build/publication for one interpreter-and-resolution key. A follow-up fix should coordinate that operation and verify that concurrent cold-cache invocations share one completed environment, that a later invocation reuses it, and that commands already running remain usable. Inspect the nested environment link and actual environment archives. The supplied reproduction and source findings are enough to start that work; no additional reporter information is necessary for classification.

Discuss automatic retention separately with the existing cache-management issues and astral-sh/uv#20116. Manual `uv cache prune` after stopping long-running uvx commands is the current concrete workaround. No implementation, builds, or tests were performed during this triage.

## Handoff boundaries

Only this README and a temporary structured result were written under the runner temporary directory. The checkout's pre-existing changes were left intact. GitHub access was read-only; no issue, pull request, label, or comment was changed.
