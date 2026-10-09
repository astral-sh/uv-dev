# Does a uv script access the Internet every time it runs?

Issue: astral-sh/uv#22388

Classification: question

## Summary

The reporter uses `#!/usr/bin/env -S uv run --script` and briefly sees `Resolving dependencies` on every invocation. They ask whether dependencies are resolved over the Internet every time and whether commands defined in `[project.scripts]` behave the same way.

The reported environment is Ubuntu 26.04, Linux 7.0.0-38-generic x86_64, with uv 0.12.21 (7af826859, September 29, 2026). The report contains no script, dependency metadata, lockfile status, verbose output, observed network requests, or execution failure. The issue has no comments.

astral-sh/uv#7538 and astral-sh/uv#9688 provide historical context. Merged changes establish persistent script environments, optional script locks, and project lockfile validation. No matching confirmed regression was found.

## Draft response

Not necessarily. `Resolving dependencies` is a progress message, not an indication that a network request occurred. In uv 0.12.21, a local script with inline metadata can reuse its cached environment and skip resolution when its installed dependencies satisfy the requirements. When resolution is needed, uv can use cached metadata; missing or expired metadata can require network requests.

To disable uv's network access, use `uv run --offline --script example.py`. The interpreter and required dependencies must be available locally. For reproducible script dependencies, `uv lock --script example.py` creates `example.py.lock`, which subsequent runs reuse; a lockfile alone does not disable networking.

For `[project.scripts]`, `uv run command` checks the project's lockfile and environment before launching the entry point, but does not necessarily access the network. Once the environment is prepared, `uv run --no-sync command` skips those updates. Running `.venv/bin/command` directly does not invoke uv. These options do not restrict network access by your Python code.

To explain why you see the message every time, please share a minimal script including its inline metadata, whether it has a lockfile, and verbose output from two consecutive `uv run -v --script example.py` invocations, with any secrets removed.

## Classification

The report asks whether a progress message implies Internet access; it establishes neither actual network requests nor incorrect dependency behavior. Source at 0.12.21 shows cached script-environment reuse and an early return when requirements are satisfied. The closest historical reports concern an older execution path or already implemented script locking, so they do not establish a duplicate or a regression. The reason for this particular script's repeated message remains unknown.

The repeated progress message is an observation, while repeated Internet access is the reporter's question. Resolver progress can describe work performed using cached metadata. The report does not establish misleading output, unnecessary network requests, or a broken environment. A bug classification would be appropriate if additional evidence establishes incorrect behavior; that evidence need not depend on a runnable reproduction.

astral-sh/uv#7538 is a particularly close historical symptom match, but it predates persistent local script environments. astral-sh/uv#9688 requested script locking, which was implemented before the reported release. Neither history establishes that an earlier bug has returned. The project-entry-point question is answered by the project lock/sync workflow rather than by assuming identical script behavior.

## Related

- astral-sh/uv#7538 (issue, closed) — "uv run" runs resolve too much. Reports the same shebang-script progress message and slower runs after a pause. Maintainers explained cached resolution and PyPI's metadata expiration. This describes older script behavior; persistent local script environments subsequently changed the execution path.
- astral-sh/uv#11347 (pull request, merged) — Use a stable directory for (local) script virtual environments. Merged February 12, 2025; replaced resolution-keyed environments for local scripts with persistent environments and update_environment. The reported 0.12.21 source includes the resulting shortcut when installed requirements are satisfied, so historical always-resolve explanations cannot be applied unchanged.
- astral-sh/uv#9688 (issue, closed) — Feat: Create a cached lock file for `uv run` with scripts. Follow-up to astral-sh/uv#7538 explicitly requested avoiding repeated resolution and network dependence. Closed by script-lockfile support in astral-sh/uv#10136. The new report asks for clarification and does not establish failure of that implemented capability.
- astral-sh/uv#10135 (pull request, merged) — Add support for locking PEP 723 scripts. Merged January 8, 2025; implemented uv lock --script and closed astral-sh/uv#6318. Establishes the available option for recording script dependency versions; locking alone is not an offline guarantee.
- astral-sh/uv#10136 (pull request, merged) — Respect PEP 723 script lockfiles in uv run. Merged January 8, 2025; made uv run reuse an existing script lockfile and closed astral-sh/uv#9688. Supports the proposed script-locking guidance and predates the reported version.
- astral-sh/uv#6091 (pull request, merged) — Validate lockfile (rather than re-resolve) in `uv lock`. Project-side background: replaced a second resolution with validation of an existing lockfile. Explains why project lock checks need not resolve dependencies afresh. Its original lockfile-instability bug, astral-sh/uv#6063, is not reported here.

## Supporting evidence

- **The progress message is not a network indicator.** In the [0.12.21 reporter implementation](https://github.com/astral-sh/uv/blob/0.12.21/crates/uv/src/commands/reporters.rs#L537), the message is set when the resolver progress reporter is constructed, rather than when an HTTP request is sent. Current checkout counterpart: `crates/uv-resolve-operations/src/reporters.rs:34`.
- **The reported release already supports reusing script environments.** The [0.12.21 script execution path](https://github.com/astral-sh/uv/blob/0.12.21/crates/uv/src/commands/project/run.rs#L380) discovers the script environment and calls `update_environment` for unlocked script requirements. The [satisfied-requirements shortcut](https://github.com/astral-sh/uv/blob/0.12.21/crates/uv/src/commands/project/mod.rs#L2959) returns without resolution when installed requirements are fresh, no reinstall or upgrade is requested, there are no source trees, and synchronization is in sufficient mode. The [run settings](https://github.com/astral-sh/uv/blob/0.12.21/crates/uv/src/settings.rs#L919) default to sufficient mode. This is conditional behavior, not a claim that all scripts and configurations avoid network access.
- **Existing test coverage supports environment reuse.** The [0.12.21 run_pep723_script test](https://github.com/astral-sh/uv/blob/0.12.21/crates/uv/tests/project/run.rs#L287) runs an inline-metadata script twice: the first run installs its dependency, the second reuses the environment, and neither creates a script lockfile. Current checkout counterpart: `crates/uv/tests/project/run.rs:287`. The snapshot is not a network trace; the early-return source provides the stronger evidence.
- **Caching and offline mode have distinct meanings.** `docs/concepts/cache.md:5` documents dependency caching, and `docs/concepts/indexes.md:258` documents HTTP cache-control behavior, including PyPI's ten-minute metadata cache. Expiration can require revalidation if metadata is needed; it does not by itself force a satisfied local script environment to resolve again. `crates/uv-cli/src/lib.rs:240` documents `--offline` as restricting uv to locally cached data and local files. It does not sandbox the executed Python program.
- **Script locking is explicit.** `docs/guides/scripts.md:276` documents `uv lock --script example.py`, creation of `example.py.lock`, and reuse on subsequent runs. astral-sh/uv#10135 and astral-sh/uv#10136 both merged January 8, 2025. astral-sh/uv#11347 merged February 12, 2025. These changes all predate uv 0.12.21.
- **Project entry points use the project workflow.** `docs/concepts/projects/config.md:35` defines `[project.scripts]` entry points and shows `uv run hello`. `docs/concepts/projects/sync.md:9` explains automatic locking/syncing, `--locked`, `--frozen`, and `--no-sync`. Current `crates/uv-project-commands/src/run.rs:719` skips project synchronization under `--no-sync`; otherwise it validates/updates the lock and synchronizes before executing the command. New registry releases do not by themselves invalidate a project lockfile. Direct execution of an installed entry point bypasses uv, though the application can still perform its own network operations.

## Search scope and exclusions

Distinct questions covered: (1) repeated resolver progress for a shebang-invoked local script; (2) whether this implies repeated Internet access and what caching/offline/locking controls apply; and (3) project entry-point execution through uv versus direct invocation. Version and platform were treated as conditions, not assumed causes.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs. Separately searched the script message/shebang and project.scripts questions using Resolving dependencies, script/internet/network, uv run/every time, project.scripts/offline/network, then cache, re-resolve, lockfile, offline fallback, area:cache, area:scripts, and version 0.12.21. Supplemented empty PR search responses with an all-state PR listing (limit 20,000), filtering script environments, caching, resolution, locking, offline, and no-sync titles. Inspected candidate bodies, comments, review comments, and linked issue/PR chains; checked source at 0.12.21. Ruled out astral-sh/uv#12903 (pinned uv tool run), astral-sh/uv#15454 and astral-sh/uv#10380 (requests for cache-first/offline fallback), and astral-sh/uv#15156 (case-sensitive imports on a case-insensitive filesystem). astral-sh/uv#10123 was an unmerged draft superseded by the listed locking PRs.

The linked discussion chain was astral-sh/uv#7538 → astral-sh/uv#9688 and astral-sh/uv#6318 → astral-sh/uv#10135 / astral-sh/uv#10136. The abandoned draft astral-sh/uv#10123 was closed in favor of other PRs. astral-sh/uv#11472 contains a maintainer explanation that script environments use stable cached paths and are reused. astral-sh/uv#6091 led to astral-sh/uv#6063, whose lockfile-instability symptoms are absent here.

The especially plausible tool-network report astral-sh/uv#12903 concerns explicitly pinned `uv tool run` packages and a maintainer-confirmed expectation about avoiding requests. The reporter here does not use that command or supply equivalent evidence. astral-sh/uv#15454 and astral-sh/uv#10380 request additional cache/fallback policies, which this report does not request. astral-sh/uv#15156 concerns module-name casing during installation, not resolver progress or repeated networking.

## Next step and verification limits

Provide the draft explanation and request the script's inline metadata, lockfile status, and verbose output from two consecutive executions with secrets removed. Those details can distinguish normal metadata resolution from an environment that unexpectedly fails the reuse checks.

This handoff is based on repository documentation, existing tests, source at the reported release, and issue/PR discussions. No reproduction, build, or test was run. No GitHub content was changed. The checkout's pre-existing modifications were left untouched.
