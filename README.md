# Export only the package dependencies

Issue: astral-sh/uv#22353

Classification: duplicate

## Summary

The reporter wants to export package dependencies for a vulnerability tracker, excluding development dependency groups. They currently use `uv export --no-dev`, observe that other groups are not excluded, and propose `--only-package` or `--no-dependency-groups`, possibly replacing `--no-dev`. The report contains no example, project configuration, uv version, platform, or error output.

Duplicate of astral-sh/uv#12366. astral-sh/uv#10592 and merged astral-sh/uv#10618 establish the existing --no-default-groups solution; astral-sh/uv#16512 and astral-sh/uv#18474 track documentation clarity.

The existing command is `uv export --no-default-groups`, without explicit group selections. Add `--no-emit-project` when the output must also omit the project itself. Group-only dependencies are excluded; dependencies required by the project remain included even if also declared in a group.

## Draft response

You can use `uv export --no-default-groups` to export your project's dependencies without including dependency groups, provided you don't also select groups with `--group` or `--all-groups`. `--no-dev` is an alias for `--no-group dev`, so it only disables the group named `dev`. Add `--no-emit-project` if your tracker should receive the dependencies without the project itself.

The request for an explicit “only main” selection is already tracked in astral-sh/uv#12366. Please continue the discussion of a dedicated flag there.

## Classification

astral-sh/uv#12366 already tracks selecting only project dependencies, including export for dependency analysis. The narrower request to disable every default group was implemented by astral-sh/uv#10618. Current source confirms that --no-dev excludes only dev and --no-default-groups disables default group inclusion. The report establishes no incorrect behavior or regression; discussion of another package-only flag can be centralized in the existing open issue.

The closest open discussion explicitly covers export for a dependency-analysis tool, so the vulnerability-tracker context does not require a separate feature discussion. The historical implementation predates this report, opened on 2026-10-08, and this report does not say that `--no-default-groups` fails. There is no evidence that previously working export behavior regressed.

The reporter's exact group configuration is unknown. Configured default groups explain how the documented behavior can occur, but are not confirmed as the reporter's configuration.

## Related

- astral-sh/uv#12366 — Does `uv` support the concept of `--only main`? (issue, open). Canonical discussion of selecting only project.dependencies, explicitly including export for a dependency-analysis tool. A maintainer recommends --no-default-groups --no-sources and identifies broader production/development selection as needing design. The vulnerability-tracker use case requests the same selection capability.
- astral-sh/uv#10592 — Option to disable all dependency groups in `uv sync` (issue, closed). Reports the same distinction: --no-dev leaves other configured default groups enabled, and requests a single exclusion flag or broader --no-dev semantics. Its example uses sync rather than export; the resulting implementation added --no-default-groups to both.
- astral-sh/uv#10618 — feat: add `--no-default-groups` flag (pull request, merged). Merged on 2025-01-21, closing astral-sh/uv#10592 and adding --no-default-groups to ExportArgs. Maintainer comments explicitly confirm that it excludes the implicit dev default and configured default groups while permitting explicitly selected groups.
- astral-sh/uv#16512 — (Documentation) Not clear which groups `uv export` exports (issue, open). Adjacent documentation issue about discovering export's default group selection. A maintainer confirms export uses default groups like run and sync. It concerns explaining existing behavior, whereas this report proposes a dedicated package-only flag.
- astral-sh/uv#18474 — docs: clarify uv export default dependency selection (pull request, open). Documentation proposal addressing astral-sh/uv#16512 by explaining default dependency groups and opt-in extras. It could improve discovery of the existing behavior; it does not implement a package-only flag and has not merged.

## Supporting evidence

- `crates/uv-cli/src/lib.rs:4535` gives export the shared project dependency-group arguments. At line 6754, `--no-dev` is documented as an alias of `--no-group dev`; at line 6801, `--no-default-groups` disables default group selection while allowing explicit groups.
- `crates/uv-configuration/src/dependency_groups.rs:33` implements these semantics: excluding development dependencies adds only the `dev` group to exclusions, and line 57 disables defaults when `no_default_groups` is set. Explicit groups remain selectable. The `prod` method at line 164 retains project dependencies unless an only-group selection was requested.
- `crates/uv-project-commands/src/export.rs:451` applies project default groups to export's selection. This agrees with the maintainer explanation in astral-sh/uv#16512.
- `crates/uv-cli/src/lib.rs:4601` documents that `--no-emit-project` omits the current project while retaining its dependencies.
- Existing snapshots in `crates/uv/tests/project/export.rs:10698` (`export_batch_selections`) show a package's default lint group omitted while its project dependency remains when `no-default-groups = true`. The test at line 12947 (`frozen_lockfile_root_default_groups`) also verifies that the CLI flag omits a recorded non-dev default group.
- In astral-sh/uv#10618, a maintainer explicitly distinguishes disabling default groups, including implicit dev, from disabling explicitly requested groups. Its merged diff adds the flag directly to `ExportArgs`.
- The maintainer recommendation in astral-sh/uv#12366 also includes `--no-sources`. That option concerns source overrides; it is not necessary merely to disable default dependency groups.

These findings come from source, existing test snapshots, and maintainer discussion. No reproduction or test execution was performed.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues for export with --no-dev, --only-package, --no-dependency-groups, --no-default-groups, default groups, only main, production/runtime dependencies, and excluding groups. Separately searched PRs across open, closed, and merged states for export, dependency groups, default groups, and exclusion flags. PR keyword searches returned no matches, so issue timelines were used to find and inspect linked open, closed, and merged PRs, including the historical implementation and documentation proposals. Read candidate bodies, maintainer comments, review comments, and the implementation diff. Ruled out astral-sh/uv#10755, resolved as pre-commit output-file misconfiguration, and astral-sh/uv#22089 with merged astral-sh/uv#22090, which concern uv audit ignoring --no-default-groups rather than export's --no-dev semantics. Checked current CLI definitions, group-selection source, and existing export snapshots. No version or error was supplied; no export regression was established.

The documentation chain also includes open astral-sh/uv#20520 and closed, unmerged astral-sh/uv#18755; both address the same documentation issue and add no distinct package-only behavior. Historical astral-sh/uv#8594 and merged astral-sh/uv#8892 concern selecting all groups, with comments leading to the later exclusion-flag design. The related list prioritizes the direct selection discussion, implemented solution, and closest documentation work.

## Maintainer next step

Recommend the existing export flags and centralize discussion of a dedicated package-only selector in astral-sh/uv#12366. The draft is for review only; no GitHub comment or issue update has been made.
