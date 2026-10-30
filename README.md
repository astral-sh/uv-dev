# Allow customizing the type checker used in `uv check`

Issue: astral-sh/uv#22941

Classification: enhancement

## Summary

astral-sh/uv#19768 is the originating discussion and explicitly delegates checker selection to this issue. astral-sh/uv#19791 concerns configuring ty rather than replacing it. No duplicate was found. A contributor has linked a uv prototype and the open companion PR meta/pyrefly#5435; neither establishes an accepted or released uv implementation.

The reporter requests a way for `uv check` to use a type checker other than ty. The report contains no reproduction, error, platform, or version constraint; this is a request for a new capability. Source inspection confirms that checker selection is currently specific to ty.

The maintainer has outlined the integration contract that alternative checkers would need: workspace target selection, configuration discovery, import environments, Python-version selection, script handling, and fixes. These remain design questions; no interface or implementation has been approved in the discussion.

The maintainer points to draft astral-sh/uv#22960 for workspace-locked checker reuse in script checks and merged astral-sh/uv#21555 for selection/exclusion semantics. The former currently targets ty, not configurable checkers. The maintainer explicitly says output parity between `uv check` and `ty check` is not yet complete; honoring clear user intent is the guiding selection policy.

## Draft response

The customization you're requesting isn't supported yet: `uv check` currently invokes ty, and `--ty-version` only selects its version. This issue tracks checker selection separately from the additional checks discussed in astral-sh/uv#19768. The next step is to define how projects select a checker and how uv passes the environment and supported options. For now, you can add your preferred checker as a development dependency and invoke it through `uv run`, for example `uv run mypy .`.

## Classification

The request adds checker selection to a command whose current implementation directly invokes ty and exposes ty-specific version selection. The primary request is for new functionality. A follow-up raises an unverified concern about script metadata synchronization ignoring the original no-sync choice; it does not yet establish a bug or regression. Although alternative checkers were discussed in astral-sh/uv#19768, a maintainer explicitly created this issue to track that capability separately.

The alternative-checker comment in astral-sh/uv#19768 is the closest match, but its maintainer explicitly separated that capability into astral-sh/uv#22941. Centralizing this issue back into the older discussion would contradict that stated scope. No historical implementation of configurable checker selection or regression was established.

## Related

- astral-sh/uv#19768 — Include additional checks, such as `ruff check`, in `uv check` (open). Origin of the request: a comment proposes selecting Pyrefly, pyright, or mypy. Maintainer zsol explicitly opened astral-sh/uv#22941 to track checker selection separately because astral-sh/uv#19768 concerns additional checks. Related, but not a duplicate.
- astral-sh/uv#19791 — Allow users to specify a custom ty configuration file when running `uv check` (open). Adjacent customization request for the same command, but it selects a ty configuration file while retaining ty. It does not request switching type checkers.
- meta/pyrefly#5435 — Support `uv check` (open, not merged). The contributor identifies this as the Pyrefly branch used by the uv prototype. Its PR description covers `--exclude-scripts`: skip discovered PEP 723 scripts while retaining explicitly supplied files, allowing project checks to leave script checks to `uv check --script`. This is companion checker work, not a duplicate or a merged uv implementation. The supplied facebook/pyrefly PR link resolves to this canonical repository.
- astral-sh/uv#22960 — Fall back to the workspace's locked `ty` for script checks (open draft, not merged). The maintainer links this work in response to the prototype requiring checker dependencies separately in scripts and packages. Its current scope is PEP 723 scripts without an applicable direct ty dependency: reuse the containing workspace's locked ty in a separate cached environment while checking with the script's own interpreter and dependencies. It does not implement arbitrary checker selection or establish a solution for every package-selection case.
- astral-sh/uv#21555 — Respect ty exclusions in `uv check` (merged September 10, 2026). Fixes astral-sh/uv#21551 by passing `--force-exclude` for automatically selected virtual-workspace members. The maintainer cites it as the precedent for distinguishing explicit user intent from automatic discovery. It informs alternative-checker integration rather than implementing checker selection.

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

The stated rationale is to give `uv check`, direct `ty check`, and the IDE the same settings and diagnostics. The maintainer explicitly notes that even `uv check` and `ty check` do not yet fully produce the same output; parity is a design goal, not an established guarantee. How other checkers can provide that consistency remains unresolved. The metadata details are attributed to the maintainer's explanation and were not independently verified in this investigation.

### Selection policy and checker reuse references

The maintainer cites merged astral-sh/uv#21555 as the concrete selection-policy precedent. Its PR description specifies:

- At a virtual workspace root, automatic member selection honors configured exclusions, including excluded ancestor directories.
- Selecting a project through the current directory (including a project-bearing workspace root), `--package`, or `--all-packages` checks the selected projects even if their directories are excluded. An explicitly selected `--script` is likewise checked despite exclusions.
- Exclusions still apply to files and subdirectories discovered within selected project directories. Unselected nested workspace members remain excluded as diagnostic targets.
- With `--no-project`, or outside a project, uv supplies no target paths and leaves discovery and exclusions to ty.

These rules express the maintainer's stated priority of honoring clear user intent. They provide concrete cases for evaluating Pyrefly's path-based versus project-based behavior; simply passing explicit paths must not silently change how automatically selected members respect exclusions.

Draft astral-sh/uv#22960 is the maintainer's reference for avoiding separate checker declarations in scripts. Its description proposes consulting the containing workspace's existing lockfile when a script has no applicable direct ty dependency, preferring the root's dev group, then production dependencies, then exactly one other group declaring ty for the workspace interpreter. Explicit `TY` or `--ty-version` takes precedence; `--no-project` skips workspace lookup. The checker and its locked dependencies use a separate cached environment, while the named script retains its own interpreter and import dependencies. These are draft-PR design details, not released behavior or an accepted generic download fallback. The PR author reports that all 22 repository script checks passed with ty 0.0.69; that result was not independently reproduced here.

### Pyrefly prototype and remaining integration questions

Pyrefly contributor kinto0 links a uv prototype in astral-sh/uv#22941, using the Pyrefly branch from meta/pyrefly#5435. The uv comparison is https://github.com/astral-sh/uv/compare/main...kinto0:uv:poc/uv-check-pyrefly. The companion PR is open and unmerged. Its description adds `--exclude-scripts` to skip discovered PEP 723 scripts while retaining explicit file targets. The prototype itself has not been executed or independently reviewed here; the following observations are the contributor's reports.

- **Checker availability:** The prototype runs Pyrefly from `uv.lock`. The contributor reports that scripts and packages each need Pyrefly individually, making `--script` and member selection inconvenient. The maintainer acknowledges the inconvenience and points to draft astral-sh/uv#22960. That PR proposes workspace-locked ty reuse for scripts; it does not yet establish the contributor's suggested generic download fallback or equivalent Pyrefly behavior.
- **Selection semantics:** They report that `pyrefly check .` (path-based checking) ignores configured excludes, whereas `pyrefly check` (project-based checking) honors them. They also report different `.gitignore` handling between the modes; the current comment does not specify which mode honors it. The contributor says file-based checks intentionally ignore configuration include/exclude rules and proposes a new `--project-dir path/to/dir` argument so package checks can use project-based semantics and honor package configuration. They also propose `--extend-project-excludes`: existing exclusion arguments reportedly replace configured exclusions, whereas uv needs to add exclusions for package selection without discarding the configured ones. These are proposed Pyrefly interfaces, not verified available flags or an accepted uv contract; the reported behavior has not been independently reproduced.
- **Configuration discovery:** In astral-sh/uv#22941, Pyrefly contributor kinto0 reports that path-based checks search upward independently from each file for its configuration. They expect a member `a`'s Pyrefly configuration to be found both when selecting `a` from the workspace root and when invoking the checker within `a`. In contrast, they hypothesize that ty uses a single configuration based on the command's working directory, potentially making `uv check --package a` from the workspace root differ from `uv check` inside `a`. The Pyrefly description is contributor-reported; the ty comparison is explicitly tentative. Neither has been independently verified here, and no configuration files, diagnostics, or versions accompany the example. This identifies a configuration-discovery comparison to investigate, not a confirmed ty bug or a complete reproduction.
- **Script metadata:** The prototype reportedly queries `uv workspace metadata` only after identifying a script; for other targets it relies on uv passing CLI arguments. The contributor proposes a `script-command` setting for script handling, using `uv workspace metadata --script <file>` as the canonical command, potentially replacing the `use_uv` setting. They include `--exclude-scripts` in their proposed basic integration and ask why ordinary `uv check` excludes scripts; that question does not establish a decision to change the project/script separation described above. These remain design suggestions and do not establish configuration or diagnostic consistency across direct CLI and IDE use.
- **Project environment discovery:** The contributor reports that Pyrefly already reads `VIRTUAL_ENV` and expects it to locate the interpreter selected by `uv check`, making an initial project-level `uv workspace metadata --active` call unnecessary. This is their proposed simplification, not independently verified behavior. Interpreter discovery alone does not establish workspace-wide configuration inheritance via `workspace_root`, handling of a target Python version distinct from the import environment, or parity with direct checker and IDE entry points; those parts of the maintainer contract still need assessment.
- **Synchronization:** Pyrefly reportedly does not pass `--sync`. The contributor asks whether it should and suspects that a metadata query with explicit `--sync` can ignore `--no-sync` from the original `uv check` invocation in ty. Honoring `UV_NO_SYNC` in metadata is proposed as a possible solution. Neither the suspected behavior nor that solution is confirmed; metadata flag precedence and environment inheritance need investigation before treating this as a correctness finding.
- **Fixes:** The prototype has no `--fix` support. The contributor reports that Pyrefly autofixes are limited to suppressing errors and removing unused ignores, and suggests excluding fixes from an initial integration. No maintainer decision accepts that scope.

The current proposal leaves checker selection open between `uv check --type-checker pyrefly` and a `tool.uv` setting. The contributor reports that Pyrefly's build-system interface supports metadata queries but requires checked files upfront as a source database, so uv-driven source selection needs adaptation. The concrete proposal now uses project-based package checks, additive project exclusions, script exclusion, and configurable script handling to address that adaptation. No maintainer acceptance or validation of this set of interfaces is present in the supplied discussion; the companion PR's reviewed description only establishes its stated script-exclusion scope.

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
- astral-sh/uv#21551 reports ty exclusions being overridden by automatically selected workspace members. Its fix, astral-sh/uv#21555, is now explicitly cited by the maintainer as the selection-policy precedent and is listed above. Neither the bug nor its fix establishes historical alternative-checker support.
- astral-sh/ty-pre-commit#7 and facebook/pyrefly#3176 provide context for the closer discussions listed above.

## Maintainer next step

Evaluate the linked uv prototype and meta/pyrefly#5435 against the checker integration contract. Use draft astral-sh/uv#22960 to assess how workspace-locked checker reuse could apply to Pyrefly, without assuming its current ty/script scope covers selected members or arbitrary checkers. Evaluate path-based versus project-based selection against the explicit/automatic rules in merged astral-sh/uv#21555, and check whether script metadata requests preserve the requested synchronization policy. Treat output parity as a goal still being worked toward, not a property already guaranteed by ty integration. Verify the reported no-sync concern separately, including explicit `--sync` precedence and `UV_NO_SYNC` inheritance, before deciding whether it warrants a bug report or a change in metadata behavior. Also validate distinct target Python versions and import environments, isolated scripts, and configuration consistency across CLI and IDE entry points. For configuration discovery, compare `uv check --package a` from a workspace root with `uv check` inside member `a`, using distinguishable workspace and member settings and recording the effective configuration and diagnostics. Assess the same selection paths in the Pyrefly prototype, where per-file upward discovery is reported. Keep this comparison separate from exclusion precedence; identical target files do not by themselves establish identical configuration selection. Assess the proposed `--project-dir` and `--extend-project-excludes` interfaces against package configuration and the selection policy, ensuring uv-added exclusions do not replace user exclusions. Check the proposed reliance on `VIRTUAL_ENV` separately from workspace configuration discovery and target-version selection. The proposed Pyrefly interfaces and omission of fixes have not been accepted. No interface or delivery date was established. Users can invoke a checker installed as a development dependency through `uv run` meanwhile; that alone does not establish parity with the proposed integration contract.

## Validation

The conclusion is based on repository source and GitHub issue evidence, with the companion Pyrefly PR and the two maintainer-linked uv PRs checked read-only for status, descriptions, and available comments. Prototype behavior, per-file Pyrefly configuration discovery, exclusion replacement, `VIRTUAL_ENV` discovery, and the synchronization concern remain contributor reports; the proposed ty working-directory difference is an unverified hypothesis. None of these were independently validated; no build or behavioral test was needed for this capability request. GitHub access was read-only. Only this temporary handoff README was authored; checkout files were not modified. The checkout already contained a modified `agents/codex/config.toml` and an untracked `.issue-triage-event.json`.
