# Allow customizing the type checker used in `uv check`

Issue: astral-sh/uv#22941

Classification: enhancement

## Summary

astral-sh/uv#19768 is the originating discussion and explicitly delegates checker selection to this issue. astral-sh/uv#19791 concerns configuring ty rather than replacing it. No duplicate was found. A contributor has linked a uv prototype and the open companion PR meta/pyrefly#5435; neither establishes an accepted or released uv implementation.

The reporter requests a way for `uv check` to use a type checker other than ty. The report contains no reproduction, error, platform, or version constraint; this is a request for a new capability. Source inspection confirms that checker selection is currently specific to ty.

The maintainer has outlined the integration contract that alternative checkers would need: workspace target selection, configuration discovery, import environments, Python-version selection, script handling, and fixes. These remain design questions; no interface or implementation has been approved in the discussion.

## Draft response

The customization you're requesting isn't supported yet: `uv check` currently invokes ty, and `--ty-version` only selects its version. This issue tracks checker selection separately from the additional checks discussed in astral-sh/uv#19768. The next step is to define how projects select a checker and how uv passes the environment and supported options. For now, you can add your preferred checker as a development dependency and invoke it through `uv run`, for example `uv run mypy .`.

## Classification

The request adds checker selection to a command whose current implementation directly invokes ty and exposes ty-specific version selection. The primary request is for new functionality. A follow-up raises an unverified concern about script metadata synchronization ignoring the original no-sync choice; it does not yet establish a bug or regression. Although alternative checkers were discussed in astral-sh/uv#19768, a maintainer explicitly created this issue to track that capability separately.

The alternative-checker comment in astral-sh/uv#19768 is the closest match, but its maintainer explicitly separated that capability into astral-sh/uv#22941. Centralizing this issue back into the older discussion would contradict that stated scope. No historical implementation of configurable checker selection or regression was established.

## Related

- astral-sh/uv#19768 — Include additional checks, such as `ruff check`, in `uv check` (open). Origin of the request: a comment proposes selecting Pyrefly, pyright, or mypy. Maintainer zsol explicitly opened astral-sh/uv#22941 to track checker selection separately because astral-sh/uv#19768 concerns additional checks. Related, but not a duplicate.
- astral-sh/uv#19791 — Allow users to specify a custom ty configuration file when running `uv check` (open). Adjacent customization request for the same command, but it selects a ty configuration file while retaining ty. It does not request switching type checkers.

- meta/pyrefly#5435 — Support `uv check` (open, not merged). The contributor identifies this as the Pyrefly branch used by the uv prototype. Its PR description covers `--exclude-scripts`: skip discovered PEP 723 scripts while retaining explicitly supplied files, allowing project checks to leave script checks to `uv check --script`. This is companion checker work, not a duplicate or a merged uv implementation. The supplied facebook/pyrefly PR link resolves to this canonical repository.

## Supporting evidence

- In astral-sh/uv#19768, kinto0's October 23, 2026 comment asks to configure other type checkers, naming Pyrefly, pyright, and mypy. The comment offers a Pyrefly prototype contingent on the checker already being in `uv.lock`; this is a contributor proposal, not an accepted design or merged implementation.
- In the same discussion, maintainer zsol's October 26, 2026 comment explicitly says astral-sh/uv#22941 was opened to track this separately because the older issue is about additional checks. The new issue's cross-reference timeline points back to that discussion.
- `crates/uv-project-commands/src/check.rs:749` directly calls `ty::run`, passing the selected ty version/path and the project environment. `crates/uv-project-commands/src/check/ty.rs` resolves or installs `Binary::Ty` and constructs a ty `check` invocation with ty-specific arguments.
- `crates/uv-cli/src/lib.rs:4975` exposes `ty_version`; its help describes choosing a version of ty, not choosing a checker. The complete `CheckArgs` structure has no checker-selection option.
- astral-sh/uv#19791 originates from astral-sh/ty-pre-commit#7, where users need separate ty configuration files for fast local checks and slower manual/CI checks. Maintainer comments confirm the distinction between invoking ty directly and forwarding options through `uv check`.
- facebook/pyrefly#3176 links back to the alternative-checker proposal from its PEP 723 script-support discussion. That narrower motivation does not replace this issue as the uv discussion for checker selection.

## Checker integration contract

Maintainer zsol's October 27, 2026 comment in astral-sh/uv#22941 identifies the following open design questions. It establishes the scope to investigate, not an accepted API or a commitment to an adapter implementation.

- **Checker selection:** How should users specify the Python package providing the checker?
- **Workspace targets:** How should `--package foo` and `--all-packages` map to checker arguments, and is an adapter script needed? Unselected nested members may still supply imports without becoming diagnostic targets. Explicit selection and automatic discovery must be distinguished, including their interaction with the checker's include/exclude settings.
- **Python version and environment:** How should the selected Python version, including a `--python` override, reach the checker? The import-resolution environment and the Python version being checked may legitimately differ. The behavior of `--no-sync`, especially with `--python`, also needs definition.
- **Scripts and multiple environments:** How should a checker learn that `--script` was requested? Future support for checking multiple scripts could involve conflicting environments or different pinned Python versions; this is a future design consideration, not a reported current failure.
- **Fixes:** How should `--fix` be supported by alternative checkers?

### Metadata and diagnostic consistency

The maintainer describes ty's current integration as calling `uv workspace metadata` for two inputs:

- `workspace_root` locates workspace-wide configuration. This allows a selected member outside the workspace directory to inherit workspace rules without separate per-member configuration.
- `environment.root`, when present, identifies the environment to search for installed packages and stubs. For projects, ty requests metadata with `--active`; for PEP 723 scripts, it requests `--sync --script <file>` and uses the script's environment for imports.

The stated rationale is to give `uv check`, direct `ty check`, and the IDE the same settings and diagnostics. How other checkers can provide that consistency remains unresolved. These details are attributed to the maintainer's explanation; ty's metadata calls and cross-entry-point diagnostic equivalence were not independently verified in this investigation.

### Pyrefly prototype and remaining integration questions

Pyrefly contributor kinto0 links a uv prototype in astral-sh/uv#22941, using the Pyrefly branch from meta/pyrefly#5435. The uv comparison is https://github.com/astral-sh/uv/compare/main...kinto0:uv:poc/uv-check-pyrefly. The companion PR is open and unmerged. Its description adds `--exclude-scripts` to skip discovered PEP 723 scripts while retaining explicit file targets. The prototype itself has not been executed or independently reviewed here; the following observations are the contributor's reports.

- **Checker availability:** The prototype runs Pyrefly from `uv.lock`. The contributor reports that scripts and packages each need Pyrefly individually, making `--script` and member selection inconvenient. A download fallback is suggested in their edited proposal, but is not an accepted requirement or implemented behavior established here.
- **Selection semantics:** They report that `pyrefly check .` (path-based checking) ignores configured excludes, whereas `pyrefly check` (project-based checking) honors them. They also report that the path-based mode consults `.gitignore` while project-based checking does not. This makes invocation mode consequential for target selection and exclusion compatibility; these observations have not been independently reproduced.
- **Script metadata:** The prototype reportedly queries `uv workspace metadata` only after identifying a script; for other targets it relies on uv passing CLI arguments. The contributor proposes a configurable script-interpreter command, set by uv to `uv workspace metadata --script <file>`, potentially replacing the `use_uv` setting. This remains a design suggestion and does not establish configuration or diagnostic consistency across direct CLI and IDE use.
- **Synchronization:** Pyrefly reportedly does not pass `--sync`. The contributor asks whether it should and suspects that a metadata query with explicit `--sync` can ignore `--no-sync` from the original `uv check` invocation in ty. Honoring `UV_NO_SYNC` in metadata is proposed as a possible solution. Neither the suspected behavior nor that solution is confirmed; metadata flag precedence and environment inheritance need investigation before treating this as a correctness finding.
- **Fixes:** The prototype has no `--fix` support. The contributor reports that Pyrefly autofixes are limited to suppressing errors and removing unused ignores, and suggests excluding fixes from an initial integration. No maintainer decision accepts that scope.

The current proposal leaves checker selection open between `uv check --type-checker pyrefly` and a `tool.uv` setting. The contributor reports that Pyrefly's build-system interface supports metadata queries but requires checked files upfront as a source database, so uv-driven source selection needs adaptation. Their edited proposal sketches project exclusions and a root target, with possible force-ignore and script-exclusion support. These sketches do not establish supported public interfaces beyond the companion PR's stated scope.

Whether `UV_NO_SYNC` can replace explicit active/sync choices remains an open question. Target Python version versus import environment, isolated-script metadata, workspace selection versus import visibility, and CLI/IDE consistency also remain unresolved parts of the maintainer's contract.

## Search scope and exclusions

The report was decomposed before searching:

- Current behavior and subsystem: `uv check` invokes ty.
- Requested capability: configure a different type checker.
- Trigger: invoking `uv check` for a project that uses another checker.
- Exact identifiers: `uv check` and `ty`. There is no reported error, platform restriction, or version-specific failure.

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using literal terms "uv check", "type checker", and "ty-version"; conceptual combinations involving mypy, pyright, Pyrefly, custom/configurable checkers, alternative checkers, and backends; and area:uv-check/check-command searches for implementations or fixes. Inspected candidate bodies, comments, and cross-reference timelines, including linked astral-sh/ty-pre-commit#7 and facebook/pyrefly#3176. No matching uv implementation PR surfaced in the initial searches; the subsequent prototype comment supplies the companion checker PR meta/pyrefly#5435 listed above. Ruled out astral-sh/uv#19768 as a duplicate because its maintainer explicitly split this request into the new issue. Also distinguished astral-sh/uv#19791 (ty configuration), astral-sh/uv#16314 (linting), astral-sh/uv#5903 (general task running), and closed astral-sh/uv#21551 (workspace exclusions).

The conceptual searches broadened beyond ty to alternative tools and general configuration. Closed-issue and merged-PR searches checked for prior implementations or fixes. The broad command search also returns unrelated dependency-validation and lockfile checks; these do not match the requested capability.

Additional inspected candidates were excluded from the Related list because their scopes are more distant:

- astral-sh/uv#16314 requests Ruff linting integration.
- astral-sh/uv#5903 discusses a general task runner and custom tasks, rather than selecting the checker behind `uv check`.
- astral-sh/uv#21551 reports ty exclusions being overridden by automatically selected workspace members. Its closed status does not establish that alternative-checker support ever existed.
- astral-sh/ty-pre-commit#7 and facebook/pyrefly#3176 provide context for the closer discussions listed above.

## Maintainer next step

Evaluate the linked uv prototype and meta/pyrefly#5435 against the checker integration contract. Prioritize checker availability for scripts and selected members, path-based versus project-based exclusion behavior, and whether script metadata requests preserve the requested synchronization policy. Verify the reported no-sync concern separately, including explicit `--sync` precedence and `UV_NO_SYNC` inheritance, before deciding whether it warrants a bug report or a change in metadata behavior. Also validate distinct target Python versions and import environments, isolated scripts, and configuration consistency across CLI and IDE entry points. The proposed script-interpreter command and omission of fixes have not been accepted. No interface or delivery date was established. Users can invoke a checker installed as a development dependency through `uv run` meanwhile; that alone does not establish parity with the proposed integration contract.

## Validation

The conclusion is based on repository source and GitHub issue evidence, with the companion PR status and description checked read-only. Prototype behavior and the synchronization concern remain contributor reports, not independently validated findings; no build or behavioral test was needed for this capability request. GitHub access was read-only. Only this temporary handoff README was authored; checkout files were not modified. The checkout already contained a modified `agents/codex/config.toml` and an untracked `.issue-triage-event.json`.
