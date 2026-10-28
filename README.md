# Concurrent Git fetches for packages from the same repository race on a cold cache: "failed to remove directory …/git-v1/db/<id>: Directory not empty"

Issue: astral-sh/uv#22975

Classification: bug

## Summary

The reporter describes intermittent failure of `uv sync --frozen` on uv 0.12.24 when three packages come from different subdirectories of the same Git repository at the same commit. An empty workspace-local `UV_CACHE_DIR` produces `failed to remove directory <cache>/git-v1/db/<id>: Directory not empty (os error 39)`. The environment is a GitHub-hosted Ubuntu 26.04 amd64 runner, Git 2.56.0, and Python 3.14.9.

The report records two failures in ten cold-cache jobs and none in ten jobs with a restored repository database. Retrying or restoring a warm cache reportedly avoids the failure. Git automatic maintenance was reportedly disabled. The provided Trace2 summary places a failing `git rev-parse` 14 ms after another process starts `git init` in the same database; the initializing process is killed after uv aborts. This timing is reported evidence, not an independently reproduced result. No runnable repository or full trace is attached, and the report does not establish whether uv 0.13.x is affected.

No confirmed duplicate found. astral-sh/uv#14894 and astral-sh/uv#5645 describe adjacent intermittent Git failures; astral-sh/uv#22829 and its open fix astral-sh/uv#22868 concern a different maintenance-versus-clone race.

## Draft response

In uv 0.12.24, the database check (`git rev-parse`) and initialization both run under the per-repository lock. The repeated “Fetching source distribution” messages occur before acquiring that lock, so they alone do not demonstrate concurrent access. The overlap in your trace still needs investigation.

This appears distinct from astral-sh/uv#22829 and astral-sh/uv#22868, which concern background maintenance racing with checkout cloning. Please try the same cold-cache case on uv 0.13.6 and share a minimal runnable repository with its lockfile, plus sanitized failing Git Trace2 and `RUST_LOG=uv_git=trace,uv_fs=trace` output with process IDs retained. Please also confirm whether another uv process or cache cleanup can access that cache during the run.

## Classification

Intermittent failure to sync valid Git dependencies from an empty cache is incorrect behavior. The source confirms the database validation and deletion/reinitialization path, but also places it under a per-repository lock; the reported overlap's cause remains unconfirmed. No existing discussion establishes the same initialization race, and no previously fixed instance of this bug was identified.

The observed sync failure warrants a bug classification even while reproduction and the concurrency mechanism remain unresolved. The evidence does not justify centralizing this report in one of the adjacent issues. The historical deadlock and cache-clean fixes address different failures, so there is no established regression of either fix.

## Related

- astral-sh/uv#14894 — Problems with git installs in CI via Github Actions (issue, open). Also reports intermittent Ubuntu CI failures installing commit-pinned subdirectory packages from one repository. Its error is an unknown revision from git rev-parse, however, with no observed initialization/removal overlap or confirmed cold-cache dependency; maintainers requested a smaller reproduction. A related symptom, not an established duplicate.
- astral-sh/uv#5645 — Intermitent installation errors when install from a git ref (issue, open). Reports intermittent missing Git checkout directories and index.lock files during installation. This is adjacent cache-integrity evidence, but concerns git-v0/checkouts on uv 0.2.x, without a confirmed mechanism or the new report's git-v1/db initialization race.
- astral-sh/uv#22829 — Git dependency checkout can race with detached `git maintenance` started by uv's own fetch (Git 2.54+) (issue, open). The reporter's cited comparison concerns detached repacking overlapping a local clone, producing missing packfiles or multi-pack-index.lock. Here maintenance is reportedly disabled and the failure occurs while initializing/removing the database. These are distinct reported triggers and failure stages.
- astral-sh/uv#22868 — Keep Git maintenance in the foreground during dependency fetches (pull request, open). Proposes fixing astral-sh/uv#22829 by keeping automatic maintenance in the foreground before cloning. Maintainer discussion confirms the repository lock is acquired before the cache check. The PR remains unmerged and does not establish a fix for concurrent database initialization with maintenance disabled.

## Supporting evidence

- In [the uv 0.12.24 resolver](https://github.com/astral-sh/uv/blob/0.12.24/crates/uv-git/src/resolver.rs#L207), `GitResolver::fetch` emits its initial fetch message before taking `LockedFileMode::Exclusive`. The lock uses `cache_digest(repository_url)`, and the guard remains in scope while awaiting `spawn_blocking(move || source.fetch())`. Thus, three initial fetch messages and one update message do not by themselves prove simultaneous database access.
- In [the uv 0.12.24 Git source](https://github.com/astral-sh/uv/blob/0.12.24/crates/uv-git/src/source.rs#L65), the database path uses the same repository digest. `git_remote.db_at(&db_path).ok()` treats an unsuccessful repository check as an unavailable database. This call is inside `GitSource::fetch`, after normal lock acquisition.
- [The uv 0.12.24 Git implementation](https://github.com/astral-sh/uv/blob/0.12.24/crates/uv-git/src/git.rs#L189) identifies the check: `GitRepository::open` invokes `git rev-parse`. `GitRemote::db_at` calls it. In `GitRemote::checkout`, an unusable database reaches `remove_dir_all(into)`, directory creation, and `GitRepository::init`. This explains the validation/removal path but does not establish why two actors could enter it concurrently.
- [The uv 0.12.24 lock implementation](https://github.com/astral-sh/uv/blob/0.12.24/crates/uv-fs/src/locked_file.rs#L273) opens or creates the lock file and acquires the requested lock. Unix creation uses `persist_noclobber` and opens the existing file on collision. The inspected code does not substantiate a claim that ordinary initialization intentionally occurs outside the repository lock.
- The checkout's `crates/uv-git/src/resolver.rs`, `source.rs`, and `git.rs` retain the same lock/check/recreation structure. This was source inspection, not a test of current-version behavior.
- The maintainer discussion on astral-sh/uv#22868 explicitly notes that the repository lock is acquired before the cache check. Its proposed maintenance settings affect Git fetch, while this report describes overlapping repository initialization with maintenance disabled.
- The 0.13.0–0.13.6 release notes contain no identified fix for this initialization failure. astral-sh/uv#22387 merged on 2026-10-12 and shipped in 0.13.1, but fixes deletion of the shared cache lock during `uv cache clean`; the report does not describe concurrent cleanup. astral-sh/uv#6790 merged in 2024 and fixes synchronous lock acquisition blocking the async runtime, as reported in astral-sh/uv#6691. Neither demonstrates that this bug was fixed and returned.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Split searches into removal errors ("failed to remove directory", "Directory not empty", "git-v1/db", legacy "git-v0/db", "not a repository"), same-repository/subdirectory installs, cold-cache and frozen-sync failures, and possible concurrency/locking causes (race, concurrent, initialization, GitResolver, flock, area:cache). Removed package, platform, and version details for broader matches. Read candidate bodies/comments and followed timeline references. PR search/list queries returned no matches even for the independently verified maintenance PR, so coverage was supplemented with linked PRs and release notes. Inspected merged astral-sh/uv#6790 (runtime deadlock fix) and astral-sh/uv#22387 (cache-clean lock deletion fix); neither establishes a fix or regression of this failure. Ruled out astral-sh/uv#11949 (repeated update messages), astral-sh/uv#12036 and astral-sh/uv#18073 (NFS-specific behavior), and astral-sh/uv#13054 (unresolved Git-hook failure). Reviewed 0.12.24 source and 0.13.0–0.13.6 release notes; no confirmed matching fix found.

Additional comparisons reinforce the distinctions. In astral-sh/uv#11949, a maintainer explains that packages share a clone and each checks its requested commit; the later failure there is a network timeout. In astral-sh/uv#13054, the command runs in a Git post-receive hook and the issue was closed for lack of follow-up, not a confirmed fix. Its comments direct the separate unknown-revision report to astral-sh/uv#14894. astral-sh/uv#6484 includes DNS/refspec failures and a restored-cache checkout failure, without this cold-cache initialization sequence. astral-sh/uv#3176 was attributed to Git/network TLS failures. astral-sh/uv#3512 concerns concurrent setuptools wheel builds, and astral-sh/uv#19674 concerns filesystem reflink failures. None establishes the same underlying problem.

The broad PR search/list results were unexpectedly empty even for a PR found through an issue timeline. Direct PR views worked; the handoff therefore does not claim exhaustive PR-search coverage.

## Next evidence needed

Obtain a minimal runnable multi-package repository and lockfile, a comparison on uv 0.13.6 using a fresh cache, and sanitized failing Git Trace2 plus uv Git/filesystem trace output retaining process IDs and lock events. Establish whether another uv process or cache-clean operation touches the same cache. These details are needed to connect the observed overlap to the lock lifecycle; the currently supplied debug lines cannot do that.

No reproduction or build was run. GitHub access was read-only, and only this temporary handoff README was written.
