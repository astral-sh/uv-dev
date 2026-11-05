# Concurrent uvx runs on a cold cache each build their own environment, orphaning all but one

Issue: astral-sh/uv#23115

Classification: bug

## Summary

Reproduced the primary behavior from astral-sh/uv#23115 using the installed uv 0.13.6 on Linux x86_64 with system CPython 3.12.3. Eight concurrent `uvx pycowsay --help` invocations against an empty isolated cache all succeeded and created two environment archives containing the same pycowsay 0.0.0.2 installation and identical `pyvenv.cfg` files. Exactly one nested environment link referenced one archive; no cache symlink referenced the other. Three later sequential invocations reused the referenced archive and left both archives present. Three sequential invocations against a separate empty cache created only one environment. Explicit pruning after the processes exited removed both cached environments.

The report observed eight archives from eight processes on uv 0.13.7 and 0.12.13, macOS arm64, and uv-managed Python 3.14.7. The number of duplicate builds is timing-dependent; this reproduction confirms the redundant-build and orphaned-archive behavior on a different uv version, platform, and interpreter. It does not independently verify those exact reported environments or the additional httpie and mcp-proxy-for-aws workloads.

The reporter separately describes cache growth from new dependency resolutions: 157 reportedly identical environments and 1,166 environments with differing resolutions, among 1,323 cached tool environments. The reported 235 GB total and approximately 202 GB environment footprint remain reporter measurements, not independently verified physical-space accounting.

No exact duplicate was identified in the existing related-issue investigation. astral-sh/uv#17399 covers a different script-creation race; merged astral-sh/uv#5509 explains the archive/link design. Open astral-sh/uv#20116 and the cache-management issues cover the separate retention and automatic-cleanup request.

## Report decomposition

- **Primary symptom:** concurrent successful invocations of the same tool create redundant cached environments; later invocations do not reuse the displaced archives.
- **Trigger and subsystem:** a cold shared cache, concurrent processes, `uvx` / `uv tool run`, cached environment materialization. MCP clients launching the same server per session are a motivating workload.
- **Expected capability:** coordinate creation for an identical resolved environment key, or safely reclaim redundant builds.
- **Separate retention request:** reclaim superseded tool/dependency resolutions automatically and make cleanup less dependent on users remembering a manual command.
- **Exact identifiers:** `archive-v0`, `environments-v2`, `pyvenv.cfg`, `CachedEnvironment`; reported uv versions 0.13.7 and 0.12.13. The report contains no failing-command error message.

## Draft response

The concurrent duplicate-environment behavior reproduces with uv 0.13.6 on Linux x86_64 and Python 3.12.3: eight simultaneous pycowsay runs produced two identical environment archives, with only one referenced by the environment cache. Three subsequent sequential runs reused that archive and left the extra one on disk. A separate sequential cold-cache control created one environment. The exact duplicate count depends on timing; the reported macOS and Python versions were not tested.

This is distinct from the inline-script failure in astral-sh/uv#17399. A focused fix should coordinate creation for one resolved environment key while keeping running commands usable. Explicit `uv cache prune` removed both environments after the test processes exited; pruning also removes the currently cached tool environment, so the next invocation will recreate it. Cache pruning requires an exclusive cache lock and can wait for long-running commands.

Cleanup of superseded resolutions is a separate retention concern tracked in astral-sh/uv#9790 and astral-sh/uv#5731. astral-sh/uv#20116 proposes automatic pruning, including unused cached tool environments, but remains unmerged in the existing issue-context investigation.

## Classification

Bug: observed concurrent successful tool runs create redundant cached environments for the same interpreter and package resolution, and leave a completed environment unreferenced after all commands exit. Later runs reuse only the linked archive. The sequential control isolates concurrent creation as the relevant trigger for this fixture. Source inspection is consistent with the observed behavior; behavioral reproduction, rather than source inspection alone, establishes this finding.

No inspected issue or PR tracks that specific uvx behavior closely enough to centralize it. The script race in astral-sh/uv#17399 is distinct, and astral-sh/uv#5509 fixed in-use mutation rather than duplicate creation. The secondary retention request overlaps existing enhancements but does not change the primary bug classification.

## Reproduction

**Outcome: reproducible.** All reproduction files, logs, caches, and tool directories were created under `/home/runner/work/_temp/uv-23115-jarEPm`. No repository files were modified.

### Environment and fixture

- Installed executables: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv` and its `uvx` shortcut, discovered on `PATH`; no checkout binary was used or built.
- `uv --version`: `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Linux 7.0.0-1017-azure x86_64; `/usr/bin/python3`: CPython 3.12.3.
- PyPI resolved the report's small pycowsay fixture to pycowsay 0.0.0.2, with no dependencies. `--no-build` restricted the test to wheels.
- Subprocesses received a fresh allowlisted environment, temporary tool/config/data/cache paths, `--no-config`, an explicit interpreter, disabled Python downloads, and the public PyPI index. Existing user tool installations and cache contents were excluded.

### Commands and procedure

The saved reproducer was executed once:

```sh
python3 /home/runner/work/_temp/uv-23115-jarEPm/reproduce.py
```

It used a thread barrier to launch eight subprocesses concurrently against an initially nonexistent shared cache. Each subprocess ran this command from the temporary reproduction directory, using the installed `uvx` executable:

```sh
uvx --no-config --cache-dir /home/runner/work/_temp/uv-23115-jarEPm/concurrent-cache \
  --python /usr/bin/python3 --no-managed-python --no-python-downloads \
  --default-index https://pypi.org/simple --no-build -v pycowsay --help
```

After waiting for all eight processes to exit, the reproducer counted `archive-v0/*/pyvenv.cfg` files and inspected every `environments-v2/*/*` symlink and its resolved archive target. It then issued three explicit sequential invocations with the same command and cache, followed by three explicit sequential invocations using a separate initially nonexistent `sequential-cache`. Each subprocess's stdout and stderr were saved separately; structured results are in `/home/runner/work/_temp/uv-23115-jarEPm/results.json`.

### Observed results

| Phase | Exit codes | Environment archives | Nested environment links | Unreferenced environment archives |
| --- | --- | --- | --- | --- |
| Eight concurrent cold-cache runs | All 0 | 2 | 1 | 1 |
| Three later sequential runs using that cache | All 0 | 2 | 1 | 1 |
| Three sequential runs using a separate cold cache | All 0 | 1 | 1 | 0 |
| Explicit prune of concurrent cache after commands exited | 0 | 0 | 0 | 0 |

The concurrent cache contained:

- `archive-v0/0J8ggJ2QLnBwF5IU`: completed environment used by concurrent process 7; no cache symlink referenced it after all concurrent processes exited.
- `archive-v0/MqfnKdB7i4FDyOuC`: completed environment used by concurrent process 6 and the other six concurrent commands; referenced by `environments-v2/a18ede1e7dd8e4d9/e86b9f3586855392`.

Both contained only pycowsay 0.0.0.2 as the installed distribution and byte-identical `pyvenv.cfg` files. Two concurrent processes logged installation; the other six found the completed linked environment. All three later sequential runs checked `MqfnKdB7i4FDyOuC`, logged no installation, and left the archive inventory and environment link unchanged. The control installed on its first run and reused its single archive on the next two.

An additional scan of every cache symlink confirmed that the displaced archive had no reference elsewhere in the cache. The saved `check_prune.py` then executed:

```sh
uv --no-config --cache-dir /home/runner/work/_temp/uv-23115-jarEPm/concurrent-cache cache prune -v
```

Pruning logged removal of the cached environment entry and both environment archives, reported `Removed 63 files (152.0KiB)`, and left zero environment archives and zero environment links. The retained sequential-control cache and saved logs provide post-run evidence; the concurrent environment archives themselves were removed by this cleanup check.

The report's top-level `ls environments-v2` count measures interpreter-hash directories, not resolved-environment links. This reproduction inspected the actual nested link. Eight archives are not required to demonstrate the race; two archives for one key already establish the reported duplication. Large real-world disk usage, automatic removal of superseded resolutions, and indefinite retention were not independently measured. Observed retention covers the completed concurrent run and three subsequent sequential runs before explicit pruning.

### Existing test coverage

The checkout organizes the relevant integration tests under `crates/uv/tests/tool/` and `crates/uv/tests/build/`, rather than the older `crates/uv/tests/it/` layout. Relevant setup and assertions were read:

- `crates/uv/tests/tool/tool_run.rs`, `tool_run_cache`: uses tool directories and Python 3.11/3.12, snapshots an initial black installation followed by sequential cache reuse, refresh reuse, distinct interpreter environments, reuse after switching back, and an extra environment with `--with`. It does not launch concurrent commands or count duplicate archives.
- `crates/uv/tests/build/cache_prune.rs`, `prune_cached_env`: runs pytest 8.0.0 through `tool run`, then snapshots verbose pruning of its environment cache entry and dangling archive. It covers explicit environment cleanup, not concurrent creation.
- Both modules are gated by `test-python` and `test-pypi` in their suite entry points. Searches of the tool tests, relevant cache tests, and `crates/uv-client/tests/it/` did not identify a test of this same-key concurrent environment-creation scenario. The client's HTTP cache concurrency tests concern a different cache layer.

No integration tests were added or run, and no build was needed. A follow-up regression test should exercise overlapping cold-cache creation, count actual environment archives and nested links, and verify subsequent reuse. The exact reported macOS/uv/Python combinations remain untested; additional reporter information is not needed to establish the reproduced primary behavior.

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
- `crates/uv-cache/src/lib.rs:411` allocates a fresh `ArchiveId`, moves the completed directory into the archive bucket, and creates the cache link. The Unix link implementation at line 951 replaces the symlink. It does not remove the previous target. These operations are consistent with the reproduced same-key duplication; atomic publication prevents partially built environments from becoming visible, but this path does not deduplicate complete builds.
- `crates/uv-cache/src/lib.rs:712` removes cached environment entries during explicit pruning; line 807 then finds references and removes unreferenced archives. `crates/uv/src/commands/cache_prune.rs:29` acquires the exclusive cache lock or waits, consistent with pruning blocking behind long-running uv processes. This is cleanup, not coordination between concurrent cache writers.
- The reproduction's top-level `ls environments-v2` count measures interpreter-hash directories for this cached-tool layout, not the number of resolved environments. The source still supports one nested resolution link being repeatedly replaced. The reproduction inspected that nested link and its archive target instead of relying on the top-level count.
- `docs/concepts/cache.md:128` documents concurrent cache readers and writers; its clearing-cache section documents periodic explicit pruning. The isolated reproduction above confirms redundant environments on installed uv 0.13.6. The exact reported releases and large disk-footprint measurements were not independently validated. Source inspection provides supporting implementation context rather than substituting for observed behavior.

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

Keep the concurrent cached-tool creation bug focused on lookup/build/publication for one interpreter-and-resolution key. A follow-up fix should coordinate that operation and verify that concurrent cold-cache invocations share one completed environment, that a later invocation reuses it, and that commands already running remain usable. Inspect the nested environment link and actual environment archives. The isolated reproduction confirms the primary behavior without requiring additional reporter configuration.

Discuss automatic retention separately with the existing cache-management issues and astral-sh/uv#20116. Manual `uv cache prune` after tool processes exit was verified to remove the duplicated archives, including the currently linked environment. No implementation changes, builds, or repository test-suite runs were performed.

## Handoff boundaries

Only this README and reproduction artifacts under the runner temporary directory were written. The checkout's pre-existing changes were left intact. Reproduction used the installed uv executable with isolated temporary caches and tool directories. No GitHub action, issue, pull request, label, or comment was changed, and no credentials were inspected or exposed.
