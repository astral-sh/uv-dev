# `uvx --from <local-path>` is never rechecked while every git ref type is (inconsistent with astral-sh/uv#16196's closure)

Issue: astral-sh/uv#22587

Classification: duplicate

## Summary

astral-sh/uv#12136 is the canonical open discussion. Closed astral-sh/uv#16196 and astral-sh/uv#16974 document the same local-source symptom; astral-sh/uv#13431 covers the ephemeral terminology concern.

The reporter sees unchanged output after editing a local Python package and restarting it with `uvx --from . <cmd>`. They contrast this with new commits being picked up through `git+file://` sources using the default branch, an explicit branch, or a moved tag. They request automatic local-source rechecking or a notice about caching, and clearer README wording about “ephemeral” environments.

The same local-source behavior and proposed default changes are already under discussion. Current source and documentation do not support the broader claims that local paths receive no checks or that Git/PyPI sources are uncached.

## Draft response

The local-source behavior you describe is already tracked in astral-sh/uv#12136, which remains open with needs-decision and considers editable installs by default or avoiding local-path caching. Please keep the proposed default change and Git comparison in that discussion. For pure-Python development, you can use `uvx --from <path> --with-editable <path> <cmd>` so source edits are visible without rebuilding.

A clarification: current local-path cache keys track project metadata and configured inputs, rather than every Python source file. Git builds are also cached by resolved commit; rechecking a ref does not mean rebuilding an unchanged commit. Tool environments are cached and disposable across source types, including PyPI. The concern about describing these as ephemeral is also discussed in astral-sh/uv#13431.

## Classification

Open astral-sh/uv#12136 already tracks the same command, local-source edits being missed, and the requested change to default caching/editable behavior. The Git comparison and notice/documentation proposals add context to that existing discussion. The ephemeral terminology concern also has an existing discussion in astral-sh/uv#13431. There is no evidence of a previously shipped fix regressing; duplicate therefore takes precedence.

astral-sh/uv#12136 was opened on March 12, 2025 and remains open. Its description explicitly says that the environment is cached, suggests `--with-editable <path>`, and asks whether local paths should be editable by default or uncached. This is a closer canonical discussion than the reporter-cited closed question.

astral-sh/uv#16196 was opened on October 9, 2025 and closed on November 1, 2025 after guidance about existing controls. astral-sh/uv#16974 was opened on December 3, 2025 and closed on December 16, 2025; the maintainer comments include both a concern about `--refresh` and advice to use editable installs. Neither inspected thread establishes a released fix for the default local-source behavior. The new report supplies no uv version or last-known-good version, so a regression cannot be inferred from those closures.

The classification does not dismiss the source-freshness or documentation concerns. It centralizes the same underlying request in the existing open issue.

## Related

- astral-sh/uv#12136 — `uvx --from <path>` does not respect changes in subsequent invocations (open). Direct match: local-path tool executions reuse an environment after source edits. The issue proposes editable installs by default or avoiding local-path caching, and remains open with needs-decision.
- astral-sh/uv#16196 — uvx does not invalidate cache when source code changes with --from flag (closed). Same local-source symptom, including requests for change detection, clearer documentation, or a caching warning. Maintainer replies recommend editable installs, reinstall-package, cache-keys, or no-cache. Its closure did not establish that a fix shipped.
- astral-sh/uv#16974 — Can't convince uvx to update program from a local project (closed). Reports the same stale local tool after source edits, additionally with --refresh on uv 0.9.14. One maintainer considered the refresh behavior a bug; later replies recommend editable installs and cache keys. This is corroborating history, not evidence of a completed fix.
- astral-sh/uv#13431 — Ephemeral environments are not ephemeral (open). Matches the separate terminology complaint: ephemeral was interpreted as deletion after execution. A maintainer explains that cached environments are disposable and may be recreated. Its reproduction concerns uv run script environments and disk usage, rather than local uvx source freshness.

## Supporting evidence

### Local sources have selective cache checks

In this checkout, `crates/uv-tool-commands/src/run.rs:1189` routes tool execution through `CachedEnvironment::from_spec`. In `crates/uv-environment-operations/src/environment.rs:60`, that method resolves requirements before looking up a cached environment. The environment hash includes per-distribution cache information at line 209. The `cache_info` method in the same file calls `CacheInfo::from_path` for local wheels, archives, and directories.

`crates/uv-cache-info/src/cache_info.rs:64` reads directory cache keys from `pyproject.toml`. Defaults include `pyproject.toml`, `setup.py`, `setup.cfg`, and a `src` directory key. Arbitrary edits to Python files are not recursively included by default. User-defined file keys and globs can add those inputs.

`docs/concepts/cache.md:15` and its “Dynamic metadata” section at line 41 describe these selective checks and configurable cache keys. This supports the narrower distinction between source edits not covered by the defaults and an absence of all freshness checks.

### Git ref resolution and build reuse are separate

`docs/concepts/cache.md:12` documents Git caching by fully resolved commit hash. In `crates/uv-distribution/src/source/mod.rs:1880`, `git_archive` obtains the precise commit, selects a commit-specific cache shard, and returns a compatible cached wheel before building when one exists.

A moved branch or tag can identify a new commit without implying that unchanged Git sources rebuild on every invocation. The reporter's observations concern changed commits; the supplied example does not establish unconditional rebuilding of an unchanged commit. The branch/tag behavior was not independently reproduced during this review.

### “Ephemeral” describes disposable cached environments

`README.md:151` contains the reported wording. `docs/concepts/tools.md:31` explicitly says tool environments live in the cache, are disposable, and are cached to reduce repeated-invocation overhead. The “Tool versions” section also describes cached registry tool versions.

In astral-sh/uv#13431, a maintainer explains that “ephemeral” means callers cannot rely on the environment remaining present or at the same location, and that it can be recreated after cache deletion. That discussion is relevant to wording, while its concrete reproduction involves script environments and cache growth.

### Existing guidance and validation limits

Maintainer replies in astral-sh/uv#16196 recommend editable installs, `--reinstall-package`, `tool.uv.cache-keys`, and `--no-cache`. The concrete editable invocation in the draft includes both `--from` and `--with-editable`, consistent with the local-tool usage discussed in astral-sh/uv#16974.

This handoff is based on repository source, documentation, and GitHub discussions. No runtime reproduction or build was performed. The report omits its uv version and a complete package fixture; its Git example contains an abbreviated `pyproject.toml`. Those limits do not prevent identifying the existing discussion, but they prevent treating every reported ref variant or any regression as independently confirmed.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using authenticated gh. Separately searched local --from source edits/stale caches, Git refresh/recheck/branch/tag/git+file behavior, ephemeral/disposable environments, and cache warnings/documentation. Broadened to uv tool, editable installs, cache invalidation, cache-keys, and CachedEnvironment; searched historical fixes and references to astral-sh/uv#12136 and astral-sh/uv#16196. Inspected candidate comments and followed discussion links. Ruled out astral-sh/uv#6804 (registry-version and installed-tool selection), astral-sh/uv#13414 and its canonical astral-sh/uv#5815 (dependency lockfile support), and astral-sh/uv#5731 (cache size limits). Also inspected astral-sh/uv#10088 and merged astral-sh/uv#10068: these concern HTTP archive subdirectories and lockfile handling, not local-path freshness. No matching fixing pull request or evidence of a fixed-then-regressed bug was found.

Literal searches covered `uvx`, `--from`, local paths, source changes, stale builds, Git refs, `git+file`, `--refresh`, and “ephemeral.” Conceptual searches covered cache invalidation, editable installs, disposable environments, notices/warnings, and the repository's tool/cache vocabulary. Mechanism and fix searches separately covered cache keys, `CachedEnvironment`, cached environments, local caching, and historical issue references. Pull-request searches included all states and explicit open, closed, and merged queries.

The astral-sh/uv#13431 link to astral-sh/uv#5731 leads to a cache-capacity request, which does not track source freshness. The astral-sh/uv#13414 closure points to astral-sh/uv#5815 for consuming a dependency's lockfile, a different request despite the shared Git tool command. The initially plausible repeated-recheck report in astral-sh/uv#10088 concerns HTTP ZIP URLs and subdirectories; its discussion of astral-sh/uv#10068 does not establish a local `uvx` caching fix.

## Maintainer next step

Centralize the local-path default-behavior discussion in astral-sh/uv#12136, carrying over the Git comparison and proposed notice as additional context. Keep the terminology clarification connected to astral-sh/uv#13431. No implementation or delivery commitment is implied.
