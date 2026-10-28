# Concurrent Git fetches from one repository: cold-cache initialization failure

Issue: astral-sh/uv#22975

Classification: bug (reported failure; not independently reproduced)

## Summary

The reporter describes intermittent failure of `uv sync --frozen` on uv 0.12.24 when three dependencies come from different subdirectories of one Git repository, with the same tag and locked commit. An empty workspace-local `UV_CACHE_DIR` reportedly produces `failed to remove directory <cache>/git-v1/db/<id>: Directory not empty (os error 39)`. The reported environment is GitHub-hosted Ubuntu 26.04 amd64, Git 2.56.0, and Python 3.14.9, with `maintenance.auto=false` and `gc.auto=0`.

The report records two failures in ten cold-cache jobs and none in ten jobs with a restored repository database. Its Trace2 summary places a failing `git rev-parse` 14 ms after another process starts `git init` in the same database. The full trace, repository, and lockfile were not supplied. The report does not establish whether uv 0.13.x is affected.

Targeted reproduction did not reproduce the failure: all 20 sync invocations succeeded, ten on uv 0.12.24 and ten on the installed uv 0.13.5. Git and Python matched the report; the available OS was Ubuntu 24.04.5. Tests included three packages, isolated cold caches, local-file and HTTP transports, deliberately slower initialization, and concurrent uv processes sharing an initially empty cache. Traces showed repository-lock contention without overlapping initialization and validation. These results do not rule out an intermittent failure in the reporter's environment.

No confirmed duplicate or fix was identified. astral-sh/uv#14894 and astral-sh/uv#5645 describe adjacent intermittent Git failures. astral-sh/uv#22829 and astral-sh/uv#22868 concern maintenance racing with checkout cloning, a different reported trigger and stage.

## Classification

Retain the bug classification for the reported failure to sync valid dependencies. The reproduction outcome is `not_reproducible` in the tested configurations, not evidence that the reported error is expected behavior or that the issue is fixed. Neither concurrent initialization nor a root cause has been independently confirmed. There is no established upgrade regression or duplicate.

## Reproduction

**Outcome: `not_reproducible`.** Every attempted sync installed all three packages successfully. No run produced `Directory not empty`, a database-removal failure, or a `rev-parse` process starting during `git init`.

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

### Limits and evidence needed

The report's stated three-package shape was sufficient for a meaningful targeted attempt, including the affected uv version, matching Git/Python versions, and two concurrency stress variants. Nevertheless, Ubuntu 26.04 and the original GitHub repository, project, lockfile, Git configuration, and CI workflow were not available. This result does not establish a fix in 0.13.x or exclude a configuration-dependent failure.

To reproduce the remaining discrepancy, request a minimal runnable repository with its exact `pyproject.toml` and `uv.lock`, a sanitized failing Git Trace2 log plus `RUST_LOG=uv_git=trace,uv_fs=trace` retaining process IDs and lock events, and the relevant CI steps. Confirm whether other uv processes, cache restoration, cleanup, custom Git templates, hooks, or filesystem mounts can touch the cache. A fresh-cache comparison on uv 0.13.6 would help, but the installed 0.13.5 comparison already succeeded here.

## Implementation and release context

In the uv 0.12.24 resolver, the initial fetch message precedes acquisition of an exclusive `LockedFile` at `git-v1/locks/<repository digest>`. The lock guard remains in scope while awaiting `GitSource::fetch`. `GitSource::fetch` checks the database with `git rev-parse`; unsuccessful checks can lead to deletion and reinitialization. The database and lock use the same repository digest. The checkout retains this structure. Source inspection explains the path but does not establish why the reporter observed overlapping access.

The 0.13.0–0.13.6 release notes identify no fix for this specific initialization failure. Two nearby merged changes were reviewed:

- astral-sh/uv#22387, shipped in 0.13.1, keeps the shared cache directory and its lock in place during `uv cache clean`. No concurrent cleanup is described in this report or used in the reproduction.
- astral-sh/uv#22464, shipped in 0.13.1, avoids unnecessary resolution for already-installed dependencies requested with full commit hashes. The fixture starts with an empty environment and uses tag sources pinned by a lockfile, so that optimization does not explain the observed successful cold-cache runs.

The older astral-sh/uv#6790 addresses synchronous lock acquisition blocking the async runtime, not a demonstrated recurrence of this initialization failure. Three initial fetch messages followed by one update are also present in the successful traces and do not alone demonstrate simultaneous database access.

## Related

- astral-sh/uv#14894 — Problems with git installs in CI via Github Actions (issue, open). Also reports intermittent Ubuntu CI failures installing commit-pinned subdirectory packages from one repository. Its error is an unknown revision from git rev-parse, however, with no observed initialization/removal overlap or confirmed cold-cache dependency; maintainers requested a smaller reproduction. A related symptom, not an established duplicate.
- astral-sh/uv#5645 — Intermitent installation errors when install from a git ref (issue, open). Reports intermittent missing Git checkout directories and index.lock files during installation. This is adjacent cache-integrity evidence, but concerns git-v0/checkouts on uv 0.2.x, without a confirmed mechanism or the new report's git-v1/db initialization race.
- astral-sh/uv#22829 — Git dependency checkout can race with detached `git maintenance` started by uv's own fetch (Git 2.54+) (issue, open). The reporter's cited comparison concerns detached repacking overlapping a local clone, producing missing packfiles or multi-pack-index.lock. Here maintenance is reportedly disabled and the failure occurs while initializing/removing the database. These are distinct reported triggers and failure stages.
- astral-sh/uv#22868 — Keep Git maintenance in the foreground during dependency fetches (pull request, open). Proposes fixing astral-sh/uv#22829 by keeping automatic maintenance in the foreground before cloning. Maintainer discussion confirms the repository lock is acquired before the cache check. The PR remains unmerged and does not establish a fix for concurrent database initialization with maintenance disabled.

## Draft response

I could not reproduce this with uv 0.12.24 or 0.13.5, Git 2.56.0, and Python 3.14.9 on Ubuntu 24.04. All 20 syncs succeeded across cold-cache, slower-initialization, and shared-cache concurrency scenarios. The traces showed the other fetches waiting for the repository lock, even with initialization taking over 400 ms.

The three fetch messages and one update also occur in these successful runs. The failing overlap in your trace still needs investigation. Please share a minimal runnable repository and lockfile, sanitized failing Git Trace2 and uv Git/filesystem trace logs retaining process IDs and lock events, and the relevant CI steps. Please confirm whether another process or cache operation can access that cache. These results do not establish that the issue is fixed in 0.13.x.

## Search scope and exclusions

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
