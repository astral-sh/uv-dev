# Support aliases for `uv tool install`

Issue: astral-sh/uv#22686

Classification: duplicate

## Summary

The reporter requests an optional `--alias` argument that lets multiple installations of the same package coexist with separate environments, dependencies, executable names, and management lifecycles. Examples include a published stable version alongside a local editable development version, different releases or prereleases, and installations from different sources. Aliases should work with `uv tool list`, `uv tool upgrade`, and `uv tool uninstall`. Omitting the option should leave normal installation behavior unchanged. Packages with multiple entry points need collision handling.

astral-sh/uv#6365 is the canonical discussion. astral-sh/uv#12451 establishes a closely matching duplicate precedent; astral-sh/uv#20370 and astral-sh/uv#21272 are open implementation proposals, while astral-sh/uv#7095 closed unmerged.

## Draft response

Your goal of independently managing multiple installations of one package is already tracked in astral-sh/uv#6365 through a proposed `--suffix` option. The proposal covers distinct names for each entry point, and astral-sh/uv#20370 and astral-sh/uv#21272 propose corresponding `list`, `upgrade`, and `uninstall` support; both remain unmerged. Please continue in astral-sh/uv#6365 with your editable-versus-published example and preference for arbitrary aliases so those details can inform the shared design.

## Classification

The independently managed, separately named installations requested here are already tracked by open astral-sh/uv#6365. The prior duplicate astral-sh/uv#12451 and pre-existing implementation proposals explicitly cover separate dependency environments and lifecycle commands. Arbitrary aliases and editable-versus-published installations are additional design details, and the reporter allows alternative interfaces. No prior merged implementation or regression is established; duplicate takes precedence over enhancement.

The issue currently carries the enhancement label, but duplicate is the recommended triage classification. The distinction between an arbitrary alias and a suffix remains a design question for the existing discussion; the request explicitly leaves the CLI interface open. The evidence supports centralizing discussion, without asserting that either open implementation already satisfies every editable-install or alternate-source scenario.

## Related

- astral-sh/uv#6365 — Add a `--suffix` option to `uv tool install` (issue, open). Canonical request for multiple installations of the same tool under distinct names. Maintainer comments describe suffixes as part of installation identity and applying them to every entry point. The proposed --alias spelling and editable-versus-published example refine the same capability.
- astral-sh/uv#12451 — allow multiple installations of a tool with different names in the bin folder (issue, closed). Requested separately named installations of the same package with different plugin dependencies. A maintainer closed it as a duplicate of astral-sh/uv#6365, and the reporter agreed, establishing that the canonical request includes independent environments rather than executable renaming alone.
- astral-sh/uv#20370 — Add suffix support to tool installs (pull request, open, draft). Open draft targeting astral-sh/uv#6365. Proposes suffix-based installation identities, renaming every entry point, retaining package identity in receipts, and supporting list, upgrade, and uninstall. Predates this report and remains unmerged; it proposes suffixes rather than arbitrary aliases.
- astral-sh/uv#21272 — Add --suffix option to uv tool install (astral-sh/uv#6365) (pull request, open). Another open, unmerged implementation targeting astral-sh/uv#6365. Its description covers coexisting installations, suffixed entry points, package/suffix receipt fields, and management commands resolving suffixed names. Predates this report and uses suffixes rather than arbitrary aliases.
- astral-sh/uv#7095 — [WIP] Add a `--suffix` option to `uv tool install` (pull request, closed). Earlier implementation for astral-sh/uv#6365, closed without merging. It separated environments by package plus suffix; maintainer review discussed retaining the underlying package name for upgrades and handling executable extensions. Its closure does not establish a shipped feature or regression.

## Supporting evidence

- In astral-sh/uv#6365, maintainer zanieb's September 5, 2024 comments quote the original design: a custom suffix applies to each entry point, suppresses the plain executable name, and allows installations with distinct keys. The follow-up distinguishes this proposed behavior from what was implemented at that time.
- In astral-sh/uv#12451, the example installs `tox` twice, with `tox-uv` added to only one installation and a different target command name. On April 1, 2025, zanieb identifies astral-sh/uv#6365 as the duplicate target, and the reporter agrees. This closely matches independent environments with different dependencies.
- The underlying design in closed astral-sh/uv#3560 discusses installation keys, suffixes, and management commands. A maintainer comment gives `foo@stable` and `foo@beta` as examples. The RFC is historical design evidence, not evidence that suffix support shipped.
- astral-sh/uv#20370 was opened July 14, 2026 and is still an open draft with no merge timestamp. Its description explicitly covers `list`, `upgrade`, and `uninstall`, plus keeping the underlying package and suffix in the receipt.
- astral-sh/uv#21272 was opened August 22, 2026 and is still open with no merge timestamp. Its description also covers receipt identity and resolving suffixed names across management commands. Both open PRs predate astral-sh/uv#22686, opened October 18, 2026. Neither has review or issue comments establishing an accepted final interface.
- astral-sh/uv#7095 closed on January 22, 2025 without merging. Its review comments discuss separating the package name from the installed name for upgrades, retaining package information in receipts, and placing suffixes correctly relative to Windows executable extensions.
- In the current checkout, `crates/uv-tool/src/lib.rs:144` defines `InstalledTools::tool_dir` using a `PackageName` and joins that package name to the tools root. `tools()` and `get_tool_receipt()` also use package names. `crates/uv-cli/src/lib.rs:5475` defines `ToolInstallArgs` with editable-install support but no alias or suffix option. This supports the reported current limitation.
- `docs/concepts/tools.md` distinguishes temporary tool execution from persistent tool installation and documents replacing an installed tool's constraints through another installation. Version-qualified `uvx` execution does not provide the requested independently named persistent installations.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered --alias, uv tool install, editable installations, different sources, tool list/upgrade/uninstall, and entry-point collisions. Conceptual searches covered suffixes, renaming, multiple versions/installations, side-by-side tools, independent environments, and area:uv-tool. Followed astral-sh/uv#6365 comments and timeline to its prior duplicate, implementation PRs, and the design in astral-sh/uv#3560; inspected PR comments and reviews. PR keyword searches returned no matches, so linked PRs were retrieved directly and their states verified; no merged implementation was identified. Ruled out astral-sh/uv#14118, which concerns project command aliases, and astral-sh/uv#15384, which concerns project-specific tool version selection. Checked the current CLI and package-keyed tool storage separately from capability searches.

The report was decomposed before searching into independent installation identity, editable/alternate-source coexistence, executable naming and multiple-entry-point collisions, and alias-aware lifecycle commands. There is no exact error message, platform constraint, or reported regression version to search. Queries used generic capability terms without incidental package names.

astral-sh/uv#14118 was inspected with its comments and excluded because its aliases expand project commands such as `uv run` or `uv sync`; maintainers route that discussion to astral-sh/uv#5903. It does not track multiple persistent installations of one package. astral-sh/uv#15384 was also inspected and excluded because it asks how project configuration should select tool versions, rather than how to name and manage multiple installed copies.

## Maintainer next step

Use astral-sh/uv#6365 as the canonical discussion and retain the editable-versus-published example and arbitrary-alias preference as design input there. Review the draft response before any posting or label change. This handoff does not imply approval, completeness, or a release date for either open PR.

Only this temporary handoff was updated. No checkout files or GitHub state were changed.
