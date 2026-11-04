# Add ability for `uv run` to only install a specific package from project

Issue: astral-sh/uv#23112

Classification: duplicate

## Summary

The reporter wants CI jobs to install and run one selected project dependency, including the dependencies it needs, at the project-locked version without installing unrelated packages from the same dependency group. They propose `uv run --only-package check-jsonschema check-jsonschema --version`, or alternatively `uv tool run --from-project check-jsonschema --version`.

The reported workaround is:

```console
uv run --only-group test --no-group test --with check-jsonschema check-jsonschema --version
```

The `test` group is defined in `pyproject.toml`. The reporter questions whether combining inclusion and exclusion flags is intended. No uv version, platform-specific failure, exact error, or regression is reported.

astral-sh/uv#10082 is the closest existing discussion. astral-sh/uv#11453 covers the equivalent sync request, and astral-sh/uv#7186 covers project-aware tool execution. Merged astral-sh/uv#19907 and astral-sh/uv#19884 provide supporting machinery and a ty-specific implementation, without a general package selector.

## Draft response

This is already tracked in astral-sh/uv#10082: installing a single tool at its project-locked version in CI without installing the rest of the environment. Please add your use case there so we can keep the API discussion together. Your suggested `uv tool run --from-project` interface also overlaps with astral-sh/uv#7186.

The group flags in your workaround follow the documented rule that exclusions take precedence over inclusions. However, `--with` performs a separate resolution using locked versions as preferences; it does not guarantee installation of the exact locked dependency graph.

## Classification

astral-sh/uv#10082 already tracks the same underlying request: run or install one project-locked tool for CI without installing unrelated project dependencies or creating a group per tool. The requested syntax is an API variant of that capability. Group exclusion precedence is documented, and --with uses lockfile version preferences, so the reported workaround does not establish a correctness bug. The merged related PRs implement internal primitives and ty-specific behavior, not a previously shipped general feature; there is no established regression.

The same CI motivation, single-tool selection, project lockfile version, and desire to avoid a dedicated dependency group make astral-sh/uv#10082 the appropriate discussion to centralize on. The current issue's alternative command spelling does not change that underlying request.

## Related

- astral-sh/uv#10082 — A command to install/get version of the package from `uv.lock` (open, issue). Canonical match: requests installing a single tool at its uv.lock version in CI without constructing the full project environment or defining a group per tool. Maintainer comments discuss selecting any locked package as the dependency-tree root.
- astral-sh/uv#11453 — uv sync: allow installing only a given package and its dependencies (closed, issue). Requests the same locked-package-and-dependencies selection for uv sync. Maintainers distinguish this from --only-install-package, which excludes unlisted dependencies. Closed because the reporter accepted an extra per product as a workaround, not because a general selector shipped.
- astral-sh/uv#7186 — Should `uvx` use a project dependency if available? (open, issue). Overlaps with the reporter's alternative uv tool run --from-project syntax: using a project's selected tool version rather than an independently selected version. Broader discussion includes reusing the project environment and default behavior, whereas this report specifically wants to avoid installing unrelated dependencies.
- astral-sh/uv#19907 — Add locked subgraph materialization primitives (merged, pull request). Referenced from astral-sh/uv#10082 and merged June 23, 2026. Adds materialization of exact locked package subgraphs and cached installation without resolving again. Explicitly a refactor with no user-visible behavior change; it does not add the requested general CLI selector.
- astral-sh/uv#19884 — Use locked ty versions in uv check (merged, pull request). Follow-up to astral-sh/uv#19907, merged June 24, 2026. Installs only the exact locked ty subgraph into a cached environment when uv check cannot use the project environment. This is a concrete implementation for ty, not arbitrary packages through uv run or uv tool run.

## Supporting evidence

- In astral-sh/uv#10082, the original report uses Ruff in CI and explicitly questions needing a group for one development dependency. A March 1, 2025 comment describes avoiding the full virtual environment because it can be slow and require compilation. Maintainer comments on January 6, 2025 discuss selecting any package as the root of the dependency tree and the existing workspace meaning of `--package`.
- In astral-sh/uv#11453, the reporter explicitly clarifies that selecting a package must include its dependencies. The September 23, 2026 closure accepts an extra-per-product workaround; it does not announce a general CLI implementation.
- `docs/concepts/projects/sync.md:141` documents that `--only-group` omits the project and default groups. At line 146, it states that group exclusions always take precedence over inclusions. This explains why excluding the same selected group can leave no project dependencies to install.
- `crates/uv-project-commands/src/run.rs:908` constructs the separate `--with` environment specification with `PreferenceLocation::Lock` when a base lockfile exists, then calls `CachedEnvironment::from_spec`. This confirms version preferences and a separate resolution, rather than exact locked-subgraph installation. It does not establish that the reporter's actual environment used any particular version.
- `crates/uv-cli/src/lib.rs:3443` defines `uv run --package` as selecting a workspace member. The hidden `--only-install-package` option belongs to `SyncArgs` and `AddArgs`, not `RunArgs`. `crates/uv-configuration/src/install_options.rs:69` filters out every package not explicitly named, including unlisted transitive dependencies, so it is not an equivalent single-tool installation interface.
- astral-sh/uv#19907 explicitly describes its implementation as a refactor without behavior changes. Its follow-up astral-sh/uv#19884 is scoped to `uv check` and `ty`. Current `crates/uv-project-commands/src/toolchain.rs:60` materializes an exact selected tool subgraph, with the `uv check` call site in `crates/uv-project-commands/src/check.rs:633`. These establish relevant implementation support, not availability of the requested arbitrary-package command.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests separately for single/specific-package installation, only-package, only-install-package, project-locked tool versions, from-project, lockfile/tool usage, and only-group/no-group/with interactions; also searched the exact check-jsonschema identifier. Conceptual searches covered arbitrary dependency-tree roots, isolated project tools, selective synchronization, task runners, and locked subgraph materialization. Followed issue comments and timeline references from astral-sh/uv#10082 to astral-sh/uv#19907 and astral-sh/uv#19884, and from astral-sh/uv#7186 through project-tool discussions. Inspected but ruled out astral-sh/uv#5815 (the installed tool's own lockfile, rather than the caller's project lockfile), astral-sh/uv#22353 (exporting runtime dependencies, resolved with --no-default-groups), and astral-sh/uv#8292 (whole-group selection, already supported). Broader astral-sh/uv#5903, astral-sh/uv#3560, astral-sh/uv#12533, and astral-sh/uv#13268 concern task definitions or declarative tool management rather than this focused selector. Checked current CLI, group-selection documentation, --with resolution code, and installation filters; no general implementing PR was identified.

astral-sh/uv#4028 was also inspected through astral-sh/uv#11453: it concerns omitting the project while retaining its dependencies, rather than selecting one dependency and its subgraph. astral-sh/uv#9026 requests access to the project dependencies from a tool environment, which differs from avoiding those dependencies here. astral-sh/uv#12663 requests per-project tool lockfiles and mutually incompatible tool environments, a broader tool-management feature.

## Verification and limits

Evidence comes from the issue report, authenticated read-only GitHub searches and issue/PR inspection, and the checked-out source and documentation. No package installation, reproduction, build, or test was needed for this classification. No source files or GitHub state were changed. The workaround's exact runtime result was not reproduced.
