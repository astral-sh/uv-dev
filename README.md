# Cold-cache Git initialization failure after a preceding uv process exits

Issue: astral-sh/uv#22975

Classification: bug (reported failure; not independently reproduced)

## Summary

The reporter describes intermittent failure of `uv sync --frozen` on uv 0.12.24 when three dependencies come from different subdirectories of one Git repository, with the same tag and locked commit. An empty workspace-local `UV_CACHE_DIR` reportedly produces `failed to remove directory <cache>/git-v1/db/<id>: Directory not empty (os error 39)`. The reported environment is GitHub-hosted Ubuntu 26.04 amd64, Git 2.56.0, and Python 3.14.9, with `maintenance.auto=false` and `gc.auto=0`.

The original report records two failures in ten cold-cache jobs and none in ten warm-cache jobs. The reporter identifies the wrapper as uv-toolbox, which they maintain. It installs tool environments from lockfiles using `uv sync --frozen --offline` first to support operation without a network, then `uv sync --frozen` on failure, using the same cache. The offline uv reportedly exits on an uncached wheel while its `git init` child remains alive. The online uv acquires the released repository lock, rejects the unfinished database, and attempts to remove it while the child is still writing. This corrects the initial interpretation of competing fetch tasks within one live uv process; the reporter says the repository lock itself works.

The follow-up supplies a process-attributed timing summary on Ubuntu 26.04/ext4 and a reduced forced-termination reproducer reported to fail on macOS 26 with both uv 0.12.24 and 0.13.6 (5/5 fresh-cache attempts each, OS error 66). The CI failure follows an ordinary error exit; the reduced example deliberately kills only the parent uv to force the overlap. These are reporter-supplied observations, not independently verified results. Multiple subdirectory packages are not required by the reduced example.

Earlier targeted reproduction succeeded in all 20 sync invocations, ten each on uv 0.12.24 and 0.13.5, on Ubuntu 24.04.5 with matching Git/Python versions. Those tests included cold caches, local-file and HTTP transports, slower initialization, and simultaneous uv processes, but did not exit or terminate the first uv during initialization and immediately retry. They therefore do not test the newly identified trigger. The contributor's 25 successful macOS runs on `main` likewise did not describe that trigger.

No confirmed duplicate or fix was identified. astral-sh/uv#14894 and astral-sh/uv#5645 describe adjacent intermittent Git failures. astral-sh/uv#22829 and astral-sh/uv#22868 concern maintenance racing with checkout cloning, a different reported trigger and stage.

## Classification

Retain the bug classification. The reporter now supplies a concrete child-lifetime explanation and an executable reduced example, including reported failures on uv 0.13.6. Independent verification of the revised trigger and normal-error shutdown path remains outstanding. There is no established upgrade regression, duplicate, or confirmed fix. The retained `not_reproducible` result applies only to the earlier test configurations; it is not a verdict on the new reproducer.

## Reproduction

**Earlier independent outcome: `not_reproducible` for the original test matrix.** Every attempted sync installed all three packages successfully. No run produced `Directory not empty`, a database-removal failure, or a `rev-parse` process starting during `git init`.

### Environment and fixture

- Ubuntu 24.04.5 LTS, x86_64, Linux 7.0.0-1017-azure; temporary storage reported the ext filesystem family (`stat -f`: `ext2/ext3`). Ubuntu 26.04 was not available.
- Git 2.56.0 and CPython 3.14.9, matching the report. Python was installed under the reproduction directory.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`. The comparison executable was uv 0.12.24, installed from PyPI into a temporary target using the installed uv. No checkout binary was built.
- All projects, environments, caches, Git repositories, configuration, scripts, and logs are under `REPRO_ROOT` (the concrete path is recorded below). No existing cache or user Git configuration was used.
- A locally authored repository contains `pkg-a`, `pkg-b`, and `pkg-c`. Each has static version `0.1.0`, `requires-python = ">=3.14"`, no runtime dependencies, the Hatchling build backend (`hatchling==1.27.0`), and a corresponding `src/pkg_a`, `src/pkg_b`, or `src/pkg_c` module. All packages share tag `v0.1.0` and commit `526ac4375c1911b31292276a582a7dc935ddbb14`.
- The consumer is an unpackaged root project with no workspace members, dependency groups, or default `dev` group. No such additional configuration was specified by the report. Its lockfile was generated once with uv 0.12.24 using a separate seed cache, then copied into each trial.

Consumer configuration (`REPO_URL` is the local repository's `file://` URL or the temporary HTTP server's URL):

```toml
[project]
name = "git-concurrency-reproduction"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = ["pkg-a", "pkg-b", "pkg-c"]

[tool.uv.sources]
pkg-a = { git = "REPO_URL", tag = "v0.1.0", subdirectory = "pkg-a" }
pkg-b = { git = "REPO_URL", tag = "v0.1.0", subdirectory = "pkg-b" }
pkg-c = { git = "REPO_URL", tag = "v0.1.0", subdirectory = "pkg-c" }
```

A temporary Git configuration supplied:

```ini
[maintenance]
    auto = false
[gc]
    auto = 0
```

### Commands and observations

The essential sequence was to generate the lockfile with a separate cache, copy the project and lockfile to a fresh directory, then run:

```sh
uv lock --python "$REPRO_PYTHON" --cache-dir "$REPRO_ROOT/seed-cache"
# In a fresh copy of the project and lockfile, with no cache or .venv:
GIT_CONFIG_NOSYSTEM=1 \
GIT_CONFIG_GLOBAL="$REPRO_ROOT/config/gitconfig" \
GIT_TERMINAL_PROMPT=0 \
UV_NO_CONFIG=1 UV_PYTHON_DOWNLOADS=never \
UV_CACHE_DIR="$PWD/cache" \
RUST_LOG=uv_git=debug,uv_fs=trace \
GIT_TRACE2_EVENT="$REPRO_ROOT/logs/trial.trace.jsonl" \
uv sync --frozen --python "$REPRO_PYTHON"
```

The setup also used the temporary Git configuration. Each command ran with a restricted environment rather than inherited Git settings. `REPRO_PYTHON` was the absolute path to the temporary Python 3.14.9 executable. The comparison substituted the temporary uv 0.12.24 executable for `uv`; the installed PATH executable provided uv 0.13.5. The actual scripts and complete logs are retained for exact command details.

| Scenario | uv 0.12.24 | uv 0.13.5 |
| --- | --- | --- |
| Three packages via `file://`, separate cold cache and environment per invocation | 5/5 succeeded | 5/5 succeeded |
| Same packages via local HTTP bare Git repository, separate cold caches | 2/2 succeeded | 2/2 succeeded |
| HTTP with 10,000 inert Git-template files to lengthen initialization | 1/1 succeeded | 1/1 succeeded |
| Two simultaneous uv processes, distinct projects/environments, one initially empty shared cache, slower initialization | 2/2 succeeded | 2/2 succeeded |

There were 20 sync invocations against 18 initially empty caches. The two-process scenarios share a cache within each pair. No cache cleanup ran concurrently. HTTP was served on loopback using Python's standard HTTP server and a bare repository prepared with `git update-server-info`; this was not the reporter's GitHub HTTPS repository.

Each single-process trial logged three `Fetching source distribution from Git` messages and one `Updating Git source`. The other fetches waited for the same exclusive repository lock and then logged `Using existing Git source`. Each shared-cache pair performed one update in total. All commands exited 0 and reported `Installed 3 packages`.

Git Trace2 showed one initialization per cache. Normal initialization took about 2–3 ms; the template variants increased it to approximately 438–457 ms, exceeding the reported 240 ms interval before abortion. No `rev-parse` started during initialization, including when the two processes' traces were combined. Some initial commit-existence probes returned 128 after initialization and before fetching; these were unknown-revision probes, not an overlapping “not a repository” failure, and the syncs succeeded.

### Existing test coverage

The setup and assertions of these tests were read; the Rust integration suite was not built or run:

- `crates/uv/tests/pip_compile/pip_compile.rs::compile_git_concurrent_access` uses a fresh `TestContext` cache and resolves two named packages from subdirectories of the same GitHub HTTPS repository at one pinned commit. Its snapshot requires exit 0 and both resolved requirements.
- `crates/uv/tests/pip_compile/pip_compile.rs::compile_git_unnamed_concurrent_access` exercises the corresponding unnamed Git requirements and likewise snapshots successful resolution of both packages.
- `crates/uv/tests/sync/sync.rs::sync_git_repeated_member_backwards_path` declares two packages from different subdirectories of the same Git repository, asserts the shared commit in the lockfile, and snapshots successful installation of both. It runs `lock` before ordinary `sync` using the same cache, so it does not cover a cold-cache frozen sync.

These tests are gated by `#[cfg(feature = "test-git")]`. `TestContext::new_with_versions_and_bin` creates a fresh cache directory. Searches also covered `crates/uv-client/tests/it/`; no test for this precise database-initialization/removal race was identified. Existing coverage is related functionality, not proof against the reported race.

### Additional reported reproduction

Contributor `nooscraft` reports in the discussion of astral-sh/uv#22975 that 25 runs on macOS using `main` all succeeded with a local repository, three subdirectory packages pinned to the same tag, and an empty cache for every run. The comment does not provide the tested commit, exact macOS/Git/Python versions, fixture, commands, or trace artifacts. Treat this as a contributor-reported result, separate from the 20 retained Ubuntu sync invocations above. It broadens the reported successful configurations but does not reproduce the Ubuntu 26.04 failure or establish a fix on `main`.

The contributor describes subsequent fetches waiting for the repository lock and then using the existing Git source, consistent with the source inspection and retained traces. The reporter now attributes the competing Git processes to successive uv invocations, with a child surviving its parent; the successful macOS runs did not describe testing that shutdown condition.

### Reporter-supplied failure sequence

In the follow-up to astral-sh/uv#22975 (comment ID 6588691516), `jamestrousdale` reports uv 0.12.24, Git 2.56, GitHub-hosted Ubuntu 26.04, and a workspace cache on ext4. A Git wrapper reportedly recorded PID, parent command line, and working directory before executing real Git; Git Trace2 supplied exit statuses and `RUST_LOG=uv_git=debug,uv_fs::locked_file=trace` supplied lock events. The supplied evidence is a summarized timeline, not the raw logs:

| Time from first initialization | Reported event |
| --- | --- |
| 0 ms | Git process A, parent `uv sync --frozen --offline`, starts `git init` in `<cache>/git-v1/db/<id>`. |
| Before 16 ms | Offline uv fails on an uncached wheel and exits; the wrapper starts the online sync. |
| 19 ms | Online uv acquires the exclusive repository lock; its other tasks wait for that lock. |
| 38 ms | Git process B, parent `uv sync --frozen`, runs `git rev-parse` in the same database and exits 128: not a Git repository. |
| 59 ms | Online uv logs `Updating Git source`. |
| 232 ms | Process A exits from SIGPIPE, which the reporter attributes to its parent having exited. |
| 245 ms | Online uv releases the repository lock. |
| 265 ms | Online uv reports directory removal failure, OS error 39. |

The reporter says no other processes used the cache and the two uv invocations ran sequentially. The reported failure rate is 1/50 in this case and approximately 2/10 in another repository, depending on whether offline sync starts initialization before its first error. This supports investigation of a Git child outliving the lock-owning uv process rather than failure of mutual exclusion between live lock holders. The exact shutdown behavior and causal interpretation remain to be verified from source and full traces.

### Reduced reproducer supplied by the reporter

Reported result: macOS 26, uv 0.12.24 and 0.13.6, five failures in five fresh-cache attempts for each version, with `Directory not empty (os error 66)`. Git/Python versions for these macOS attempts are not specified. The example uses one Git dependency and one registry dependency, without subdirectories:

```toml
[project]
name = "repro"
version = "0"
requires-python = ">=3.12"
dependencies = ["packaging", "idna"]

[tool.uv]
package = false

[tool.uv.sources]
packaging = { git = "https://github.com/pypa/packaging", tag = "24.2" }
```

The supplied setup and orchestration are:

1. Place this configuration in `project/pyproject.toml` and generate its lockfile using `uv lock --directory project`. The shown failing package resolves to commit `d8e3b31b734926ebbcaff654279f6855a73e052f`.
2. Create a Git template containing 20 subdirectories under `template/info`, each with 1,000 small files, to lengthen initialization. Set `GIT_TEMPLATE_DIR` to its absolute path.
3. Prepend an executable `bin/git` wrapper to `PATH`. When its first argument is `init` and `REPRO_INIT_MARK` is set, it creates that marker file, then executes `/usr/bin/git` with the original arguments.
4. Use a fresh absolute cache path `CACHE`, with `UV_CACHE_DIR=CACHE`, `UV_PROJECT_ENVIRONMENT=CACHE.venv`, and `REPRO_INIT_MARK=CACHE.init-started`. Start `uv sync --frozen` in the project in the background, redirecting its output to `CACHE.first.log`.
5. Once the initialization marker exists, send SIGKILL to only that uv PID and wait for it to exit. Leave its Git child running. Immediately run another `uv sync --frozen` against the same cache and environment.

The comment contains the complete wrapper and orchestration scripts. They were reviewed as reproduction evidence and were not executed during this update. Forced parent termination makes the lifetime overlap deterministic in the reported tests; it does not independently demonstrate the ordinary offline-error shutdown behavior. The original CI case uses an error exit rather than SIGKILL, so both paths need separate verification. The reporter's suggestions to terminate/wait for children or treat unfinished databases as busy are proposed remedies, not validated fixes or maintainer decisions.

### Downstream trigger and mitigation tracking

The reporter identifies [uv-toolbox](https://github.com/hotdog-werx/uv-toolbox) as the caller and opened hotdog-werx/uv-toolbox#27 to address its offline-first installation strategy. The linked issue is open with no comments at review time. It identifies `sync_environment` in `uv_toolbox/project.py` and shows an offline sync followed by an online retry after `ExternalCommandError`; this is an issue-provided code excerpt, not independently inspected implementation.

The downstream issue supplies additional reported context for an environment with seven subdirectory packages from one Git repository on Ubuntu 26.04: single online sync, offline-then-online sync, and direct `uv-toolbox install` each had 0 failures in 50 attempts across uv 0.12.24 and an unspecified 0.13 version. `uv-toolbox install` invoked through mise's postinstall hook (`mise install --locked`) had 1 failure in 50 attempts on uv 0.12.24. These appear to elaborate the previously reported CI failure, rather than establish an independent reproduction, and do not establish mise itself as the cause.

The linked issue also reports that terminating the first uv's whole process group made the forced-termination reproduction pass in 5/5 attempts, compared with 5/5 failures when only uv was killed. This is useful evidence for investigating surviving children, but remains reporter-supplied and does not establish a portable remedy. The preferred downstream proposal is to drop the offline attempt and run one frozen online sync, with skipping the offline attempt for missing environments as an alternative. A small uv 0.13.6 warm-cache test reportedly succeeded with proxies pointing at a closed port; Python downloads and slow-network behavior still need checking. No implemented or released mitigation is established by the linked issue. Removing the wrapper's trigger would not establish that uv's reported child-lifetime behavior is fixed, and failures in other retrying tools remain a hypothesis.

### Limits and next investigation

The retained independent tests and `reproduction.json` predate this reduced example. Preserve their results, but do not use them to dismiss the revised report or infer that uv 0.13.x is unaffected. The new comment supplies the previously missing two-command sequence, process roles, database working directory, filesystem type, and a minimal project configuration. The original project/lockfile, raw failing CI traces, and complete workflow remain unavailable.

The next investigation should independently exercise the reduced example, then the normal `--offline` failure followed by online retry, and inspect shutdown/cancellation ownership of the Git child and repository lock. The uv-toolbox/mise invocation path and the reported whole-process-group comparison provide concrete additional cases to investigate. Full sanitized wrapper, Trace2, and lock logs with PIDs and timestamps would help verify the parent exit, child's continued writes, and lock transfer. A retained lockfile for the reduced fixture would make the generated dependency selection repeatable. No revised reproducer or additional source verification was performed in this context update.

## Implementation and release context

In the uv 0.12.24 resolver, the initial fetch message precedes acquisition of an exclusive `LockedFile` at `git-v1/locks/<repository digest>`. The lock guard remains in scope while awaiting `GitSource::fetch`. `GitSource::fetch` checks the database with `git rev-parse`; unsuccessful checks can lead to deletion and reinitialization. The database and lock use the same repository digest. The checkout retains this structure. This source inspection describes the normal awaited path; it does not establish child cleanup on error exit or forced termination. The follow-up directs investigation to the lifetime of the Git child relative to the parent-held lock.

The 0.13.0–0.13.6 release notes identify no fix for this specific initialization failure. Two nearby merged changes were reviewed:

- astral-sh/uv#22387, shipped in 0.13.1, keeps the shared cache directory and its lock in place during `uv cache clean`. No concurrent cleanup is described in this report or used in the reproduction.
- astral-sh/uv#22464, shipped in 0.13.1, avoids unnecessary resolution for already-installed dependencies requested with full commit hashes. The fixture starts with an empty environment and uses tag sources pinned by a lockfile, so that optimization does not explain the observed successful cold-cache runs.

The older astral-sh/uv#6790 addresses synchronous lock acquisition blocking the async runtime, not a demonstrated recurrence of this initialization failure. Three initial fetch messages followed by one update are also present in the successful traces and do not alone demonstrate simultaneous database access.

## Related

- hotdog-werx/uv-toolbox#27 — `install` on a cold uv cache can fail with "Directory not empty" because its offline sync leaves a Git child running (issue, open). Tracks mitigation in the identified caller of uv: avoiding or cleaning up the offline-first attempt before retrying online. It links this uv issue and adds mise-hook and process-group observations. It is downstream mitigation tracking, not a duplicate or confirmed fix for uv's reported child-lifetime problem.
- astral-sh/uv#14894 — Problems with git installs in CI via Github Actions (issue, open). Also reports intermittent Ubuntu CI failures installing commit-pinned subdirectory packages from one repository. Its error is an unknown revision from git rev-parse, however, with no observed initialization/removal overlap or confirmed cold-cache dependency; maintainers requested a smaller reproduction. A related symptom, not an established duplicate.
- astral-sh/uv#5645 — Intermitent installation errors when install from a git ref (issue, open). Reports intermittent missing Git checkout directories and index.lock files during installation. This is adjacent cache-integrity evidence, but concerns git-v0/checkouts on uv 0.2.x, without a confirmed mechanism or the new report's git-v1/db initialization race.
- astral-sh/uv#22829 — Git dependency checkout can race with detached `git maintenance` started by uv's own fetch (Git 2.54+) (issue, open). The reporter's cited comparison concerns detached repacking overlapping a local clone, producing missing packfiles or multi-pack-index.lock. Here maintenance is reportedly disabled and the failure occurs while initializing/removing the database. These are distinct reported triggers and failure stages.
- astral-sh/uv#22868 — Keep Git maintenance in the foreground during dependency fetches (pull request, open). Proposes fixing astral-sh/uv#22829 by keeping automatic maintenance in the foreground before cloning. Maintainer discussion confirms the repository lock is acquired before the cache check. The PR remains unmerged and does not establish a fix for concurrent database initialization with maintenance disabled.

## Search scope and exclusions

The following records the earlier uv investigation; uv related-item states and searches were not refreshed for this comment update. The newly linked hotdog-werx/uv-toolbox#27 was read directly, including its empty discussion, to establish the downstream context above.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Split searches into removal errors ("failed to remove directory", "Directory not empty", "git-v1/db", legacy "git-v0/db", "not a repository"), same-repository/subdirectory installs, cold-cache and frozen-sync failures, and possible concurrency/locking causes (race, concurrent, initialization, GitResolver, flock, area:cache). Removed package, platform, and version details for broader matches. Read candidate bodies/comments and followed timeline references. PR search/list queries returned no matches even for the independently verified maintenance PR, so coverage was supplemented with linked PRs and release notes. Inspected merged astral-sh/uv#6790 (runtime deadlock fix) and astral-sh/uv#22387 (cache-clean lock deletion fix); neither establishes a fix or regression of this failure. Ruled out astral-sh/uv#11949 (repeated update messages), astral-sh/uv#12036 and astral-sh/uv#18073 (NFS-specific behavior), and astral-sh/uv#13054 (unresolved Git-hook failure). Reviewed 0.12.24 source and 0.13.0–0.13.6 release notes; no confirmed matching fix found.

Additional comparisons reinforce the distinctions. In astral-sh/uv#11949, a maintainer explains that packages share a clone and each checks its requested commit; the later failure there is a network timeout. In astral-sh/uv#13054, the command runs in a Git post-receive hook and the issue was closed for lack of follow-up, not a confirmed fix. Its comments direct the separate unknown-revision report to astral-sh/uv#14894. astral-sh/uv#6484 includes DNS/refspec failures and a restored-cache checkout failure, without this cold-cache initialization sequence. astral-sh/uv#3176 was attributed to Git/network TLS failures. astral-sh/uv#3512 concerns concurrent setuptools wheel builds, and astral-sh/uv#19674 concerns filesystem reflink failures. None establishes the same underlying problem.

The broad PR search/list results were unexpectedly empty even for a PR found through an issue timeline. Direct PR views worked; the handoff therefore does not claim exhaustive PR-search coverage.

## Local artifacts

- Reproduction root: `/tmp/uv-22975-rkt6m40v`.
- Fixture repository: `repository/`; consumer and lockfile: `template/`; HTTP consumer and lockfile: `http-template/`.
- `trial.py` runs a fresh local-file case, for example `python3 trial.py fresh-old old` or `python3 trial.py fresh-current current` from the reproduction directory. Use a new case name each time.
- `variants.py` records the HTTP, slow-template, and simultaneous-process scenarios. Its server is stopped; its setup uses fresh names, so rerunning the whole script requires a fresh reproduction root.
- `paths.json` records executable locations. `logs/*.log`, `logs/*.trace.jsonl`, and `logs/*.result.json` hold command output, Git traces, and results. `logs/trace-summary.json` records initialization durations and overlap checks.
- The temporary uv 0.12.24 installation, Python 3.14.9 installation, source fixture, and all per-case caches are retained under this root.

Only temporary reproduction files and this handoff were written. No checkout files were edited by the reproduction commands, no uv build was performed, and GitHub access was read-only.

This context update changed only the README. The structured triage and reproduction files retain their earlier findings; no new reproduction commands were run and no public response was drafted or posted.
