# Unhelpful warning when using `--no-project` with `UV_FROZEN` set

Issue: astral-sh/uv#22755

Classification: bug

## Summary

The reporter sets `UV_FROZEN` globally for CI and runs a minor script with `uv run --no-project test.py`. On uv 0.13.3 on Linux x86_64, this emits:

```text
warning: `--frozen` has no effect when used alongside `--no-project`
```

The reporter asks whether these warnings should ignore environment-derived settings, also mentioning `UV_LOCKED` and `UV_NO_SYNC`. No execution failure is reported. The source confirms that the warning does not use the recorded source of frozen mode.

No duplicate found. astral-sh/uv#5856 and astral-sh/uv#5977 explain the warning's origin; astral-sh/uv#17259 and astral-sh/uv#17318 provide related precedent for environment-aware diagnostics.

## Draft response

The current `--no-project` warning checks whether frozen mode is enabled and always names `--frozen`, even when it came from `UV_FROZEN`. That explains the misleading output you're seeing. The source already records where frozen mode came from, so this warning can distinguish inherited settings from explicit flags. A targeted next step is to suppress the warning for environment-only settings and review `UV_NO_SYNC` and `UV_LOCKED` consistently. For now, `uv run --no-project --no-frozen test.py` overrides `UV_FROZEN` for that invocation.

## Classification

The 0.13.3 source confirms misleading diagnostics: inherited UV_FROZEN triggers a warning naming --frozen even though no such flag was supplied. FrozenSource retains the environment/CLI distinction, but this warning ignores it. UV_NO_SYNC has similar flag-only wording; UV_LOCKED already identifies its environment source but still warns. No existing item tracks this specific problem, and the historical missing-lockfile fix addressed a different diagnostic.

The established correctness issue is the misleading attribution of an inherited setting to a command-line flag. Suppressing environment-only warnings is the requested remedy; the broader policy for other settings still needs a maintainer decision. The wording of the report as a question does not make this a support-only issue.

## Related

- astral-sh/uv#5977 (pull request, merged) — Warn when project-specific settings are passed to non-project `uv run` commands. Introduced the exact --frozen/--no-project warning in August 2024. Its purpose was to warn about ignored project options; it did not address suppressing warnings for inherited environment settings.
- astral-sh/uv#5856 (issue, closed) — Warn when project options are used with `uv run` outside a project. The original request behind astral-sh/uv#5977 explains why these warnings exist. It requested warnings for ignored options, whereas this report concerns misleading warnings from environment-only defaults.
- astral-sh/uv#17259 (issue, closed) — Unfriendly error message when `uv lock` or `uv sync` with `UV_FROZEN=1` env variable. Related precedent for confusing diagnostics caused by inherited UV_FROZEN. It concerned missing-lockfile errors during lock/sync, not no-project warnings, and was fixed by astral-sh/uv#17318.
- astral-sh/uv#17318 (pull request, merged) — Use a dedicated error message when lockfile can't be found. Merged January 2026; made missing-lockfile errors identify UV_FROZEN or UV_LOCKED as their source. The diff leaves the no-project warning unchanged, so this report is not a regression of that fix.

## Supporting evidence

The checkout is at commit `502f93c`, the uv 0.13.3 version bump, matching the reported release. The issue currently has no comments.

- `crates/uv-cli/src/settings.rs:653`: `resolve_frozen` records `FrozenSource::Cli` for an explicit flag and `FrozenSource::Env` for an enabled environment setting. An explicit `--no-frozen` prevents the environment fallback.
- `crates/uv-cli/src/settings.rs:783`: `RunSettings::resolve` resolves locked, frozen, and no-sync values from CLI arguments and environment settings. At line 863, no-sync is reduced to a boolean, discarding its source before command execution.
- `crates/uv-project-commands/src/run.rs:601`: the ordinary-script/non-PEP-723 path emits no-project warnings. At line 612, `frozen.is_some()` triggers hard-coded `--frozen` wording for both CLI and environment sources. At line 615, the no-sync boolean triggers hard-coded `--no-sync` wording.
- `crates/uv-project-commands/src/run.rs:609` and `crates/uv-settings/src/resolved.rs:50`: the locked warning formats `LockedSource`; an environment-derived value is already rendered as `UV_LOCKED=1`. Thus the reporter's broader request also concerns warning suppression, but the attribution problem is not identical for every setting.
- `crates/uv-cli/src/lib.rs:3407`: `--no-frozen` is explicitly documented in source as overriding `UV_FROZEN`, supporting the per-invocation workaround in the draft response.
- `crates/uv/tests/project/run.rs:3512`: `run_no_project` already snapshots the warning for an explicit `--locked`. Existing no-sync environment tests exercise project syncing, but no existing test combining environment-derived frozen mode with this no-project warning was found.

The diff and review of astral-sh/uv#5977 establish that the warning was intentionally introduced for ignored project-specific options. A maintainer explained that runtime warnings kept behavior consistent with cases where project discovery cannot be known during argument parsing. That rationale does not establish a policy for globally inherited settings.

The January 2026 fix in astral-sh/uv#17318 predates this report, but only changes missing-lockfile diagnostics. It does not suppress or correct this no-project warning. There is no evidence here of a previously fixed version of the reported behavior returning.

These findings come from source and existing tests; no runtime reproduction, build, or test execution was performed.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using the exact warning, --no-project with UV_FROZEN, UV_LOCKED, and UV_NO_SYNC separately, and broader terms including no-op, ignored settings, spurious warnings, environment variables, and area:error-messages. Removed version/platform constraints and searched historical fixes; inspected candidate comments, timelines, PR reviews, and diffs. Ruled out astral-sh/uv#10743 because its similar warning concerns dependency-group/project discovery, and astral-sh/uv#9772 because it requests warnings for resolution options ignored by frozen mode. No duplicate found.

The report was decomposed before searching into the exact frozen warning, the requested distinction between explicit flags and inherited settings, and the separate locked/no-sync extensions. Searches covered `uv run`, project discovery, ignored project options, and diagnostic wording without restricting the search to Linux or uv 0.13.3. Historical PRs were located through issue timelines and inspected directly.

astral-sh/uv#10743 was especially plausible because it quotes the same warning template. Its comments establish that dependency-group discovery and a missing `[project]` table explain the behavior, rather than inherited environment defaults. astral-sh/uv#9772 concerns the opposite diagnostic gap: frozen mode silently ignoring resolution options. The astral-sh/uv#5977 review also references astral-sh/uv#5855; that issue concerns ignored `[tool.uv]` settings in inline script metadata and is not the same request.

## Suggested next step

Make the no-project warning use the setting's source, with a maintainer decision on suppressing environment-only no-effect warnings. Review frozen, locked, and no-sync separately because their current source handling differs. A focused regression test should cover `UV_FROZEN=1` with `--no-project` and retain meaningful coverage for an explicitly supplied flag; extend the existing project-run test module and check its feature-gating conventions.

Only this temporary handoff was updated. No checkout files or GitHub state were changed.
