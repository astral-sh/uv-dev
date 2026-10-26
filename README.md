# Allow customizing the type checker used in `uv check`

Issue: astral-sh/uv#22941

Classification: enhancement

## Summary

astral-sh/uv#19768 is the originating discussion and explicitly delegates checker selection to this issue. astral-sh/uv#19791 concerns configuring ty rather than replacing it. No duplicate or implementing pull request was found.

The reporter requests a way for `uv check` to use a type checker other than ty. The report contains no reproduction, error, platform, or version constraint; this is a request for a new capability. Source inspection confirms that checker selection is currently specific to ty.

## Draft response

The customization you're requesting isn't supported yet: `uv check` currently invokes ty, and `--ty-version` only selects its version. This issue tracks checker selection separately from the additional checks discussed in astral-sh/uv#19768. The next step is to define how projects select a checker and how uv passes the environment and supported options. For now, you can add your preferred checker as a development dependency and invoke it through `uv run`, for example `uv run mypy .`.

## Classification

The request adds checker selection to a command whose current implementation directly invokes ty and exposes ty-specific version selection. No incorrect behavior or regression is reported. Although alternative checkers were discussed in astral-sh/uv#19768, a maintainer explicitly created this issue to track that capability separately.

The alternative-checker comment in astral-sh/uv#19768 is the closest match, but its maintainer explicitly separated that capability into astral-sh/uv#22941. Centralizing this issue back into the older discussion would contradict that stated scope. No historical implementation of configurable checker selection or regression was established.

## Related

- astral-sh/uv#19768 — Include additional checks, such as `ruff check`, in `uv check` (open). Origin of the request: a comment proposes selecting Pyrefly, pyright, or mypy. Maintainer zsol explicitly opened astral-sh/uv#22941 to track checker selection separately because astral-sh/uv#19768 concerns additional checks. Related, but not a duplicate.
- astral-sh/uv#19791 — Allow users to specify a custom ty configuration file when running `uv check` (open). Adjacent customization request for the same command, but it selects a ty configuration file while retaining ty. It does not request switching type checkers.

## Supporting evidence

- In astral-sh/uv#19768, kinto0's October 23, 2026 comment asks to configure other type checkers, naming Pyrefly, pyright, and mypy. The comment offers a Pyrefly prototype contingent on the checker already being in `uv.lock`; this is a contributor proposal, not an accepted design or merged implementation.
- In the same discussion, maintainer zsol's October 26, 2026 comment explicitly says astral-sh/uv#22941 was opened to track this separately because the older issue is about additional checks. The new issue's cross-reference timeline points back to that discussion.
- `crates/uv-project-commands/src/check.rs:749` directly calls `ty::run`, passing the selected ty version/path and the project environment. `crates/uv-project-commands/src/check/ty.rs` resolves or installs `Binary::Ty` and constructs a ty `check` invocation with ty-specific arguments.
- `crates/uv-cli/src/lib.rs:4975` exposes `ty_version`; its help describes choosing a version of ty, not choosing a checker. The complete `CheckArgs` structure has no checker-selection option.
- astral-sh/uv#19791 originates from astral-sh/ty-pre-commit#7, where users need separate ty configuration files for fast local checks and slower manual/CI checks. Maintainer comments confirm the distinction between invoking ty directly and forwarding options through `uv check`.
- facebook/pyrefly#3176 links back to the alternative-checker proposal from its PEP 723 script-support discussion. That narrower motivation does not replace this issue as the uv discussion for checker selection.

## Search scope and exclusions

The report was decomposed before searching:

- Current behavior and subsystem: `uv check` invokes ty.
- Requested capability: configure a different type checker.
- Trigger: invoking `uv check` for a project that uses another checker.
- Exact identifiers: `uv check` and `ty`. There is no reported error, platform restriction, or version-specific failure.

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using literal terms "uv check", "type checker", and "ty-version"; conceptual combinations involving mypy, pyright, Pyrefly, custom/configurable checkers, alternative checkers, and backends; and area:uv-check/check-command searches for implementations or fixes. Inspected candidate bodies, comments, and cross-reference timelines, including linked astral-sh/ty-pre-commit#7 and facebook/pyrefly#3176. No matching implementation PR surfaced. Ruled out astral-sh/uv#19768 as a duplicate because its maintainer explicitly split this request into the new issue. Also distinguished astral-sh/uv#19791 (ty configuration), astral-sh/uv#16314 (linting), astral-sh/uv#5903 (general task running), and closed astral-sh/uv#21551 (workspace exclusions).

The conceptual searches broadened beyond ty to alternative tools and general configuration. Closed-issue and merged-PR searches checked for prior implementations or fixes. The broad command search also returns unrelated dependency-validation and lockfile checks; these do not match the requested capability.

Additional inspected candidates were excluded from the Related list because their scopes are more distant:

- astral-sh/uv#16314 requests Ruff linting integration.
- astral-sh/uv#5903 discusses a general task runner and custom tasks, rather than selecting the checker behind `uv check`.
- astral-sh/uv#21551 reports ty exclusions being overridden by automatically selected workspace members. Its closed status does not establish that alternative-checker support ever existed.
- astral-sh/ty-pre-commit#7 and facebook/pyrefly#3176 provide context for the closer discussions listed above.

## Maintainer next step

Use astral-sh/uv#22941 for design discussion about checker selection, environment handoff, and supported options. The existing prototype offer in astral-sh/uv#19768 may inform that discussion, but no accepted interface or delivery date was established. Users can invoke a checker installed as a project development dependency through `uv run` meanwhile.

## Validation

The conclusion is based on repository source and GitHub issue evidence; no build or behavioral test was needed for this capability request. GitHub access was read-only. Only this temporary handoff README was authored; checkout files were not modified. The checkout already contained a modified `agents/codex/config.toml` and an untracked `.issue-triage-event.json`.
