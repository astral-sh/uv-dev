# Bug: `uv tool upgrade` doesn't respect changes to the global `exclude-newer` setting

Issue: astral-sh/uv#22468

Classification: bug

## Summary

Related discussions cover receipt/configuration merging; merged PRs explain the precedence, disabling support, and relative-cutoff handling. None directly tracks clearing an inherited tool cooldown.

The reporter uses uv 0.13.0 (4ecd0eff2, October 9, 2026), Python 3.15.0, and Darwin 27.0.0 arm64. They install Black while `~/.config/uv/uv.toml` contains `exclude-newer = "3 days"`, then change it to `exclude-newer = false`, comment it out, or remove it. Both `uv tool upgrade --all` and `uv tool upgrade --all --reinstall` reportedly retain the three-day restriction. The tool receipt and tool lock retain `exclude-newer-span = "P3D"`; the reporter also says the timestamps remain unchanged.

Expected behavior is to apply current global configuration, remove obsolete cutoff metadata, and allow newer versions while preserving settings explicitly supplied for an individual tool. The live issue had no comments when inspected.

## Draft response

Tool installation currently saves the merged settings, and upgrades prioritize those saved values over current global configuration. The receipt does not distinguish an inherited cutoff from an explicit per-tool setting, which explains why your global change is ignored.

For an immediate override, try `uv tool upgrade black --exclude-newer false`. This explicitly disables that tool's cutoff; it does not restore inheritance from global configuration. A fix needs to distinguish inherited values from explicit tool settings. Please also confirm whether the `tool-install-locks` preview feature is enabled for the reported `uv.lock` behavior.

## Classification

Source confirms that installation persists merged settings without recording the origin of exclude-newer, and upgrades prioritize that saved value over current global configuration, including false or an absent value. This prevents an inherited policy from following global changes. Related issues concern authentication or list merging; prior fixes cover disabling support and relative-time recomputation, not this cutoff-reset behavior. No previously fixed regression was established.

The saved-setting precedence is intentional for explicit per-tool options, as documented in the tool upgrade CLI help. The correctness problem is treating a value inherited from global configuration as an enduring per-tool override. Existing receipts lack the origin information needed to satisfy both expectations reliably.

This is not classified as a duplicate of the adjacent authentication or list-merging reports. Those discussions are relevant to the design but do not track resetting an inherited scalar cutoff. Neither the relative-span fix nor the boolean-disabling fix establishes that this global-configuration transition previously worked. The unchanged-timestamp detail is not independently confirmed.

## Related

- astral-sh/uv#8523 (open issue) — uv tool upgrade - problem with private indexes due to short living credentials. Upgrades reuse receipt values after the user updates global configuration. Maintainer comments discuss saving only explicit CLI overrides and distinguishing configuration origins. Its concrete failure concerns expiring index authentication, whereas this report requires clearing an inherited cutoff.
- astral-sh/uv#11929 (open issue) — Upgrading tools adds an identical entry to the `index` list in uv-receipt.toml. Maintainers identify merging saved receipt settings with current filesystem settings as the cause and discuss replacement semantics. This concerns accumulating list entries; the new report concerns a saved scalar cutoff overriding changed or removed configuration.
- astral-sh/uv#5937 (merged pull request) — Retain and respect settings in tool upgrades. Introduced persisted resolver/installer options and CLI > receipt > filesystem precedence, which remains in the upgrade implementation. This establishes the origin of the behavior, rather than an existing fix for inherited-setting removal.
- astral-sh/uv#19934 (merged pull request) — Allow disabling `exclude-newer`. Merged June 22, 2026, closing astral-sh/uv#19929. Added false overrides in CLI, environment, configuration, and receipts. It supports an explicit CLI workaround but did not change receipt precedence or track inherited values, so this report is not evidence that its fix regressed.
- astral-sh/uv#18901 (merged pull request) — Store relative timestamps in tool receipts. Merged April 7, 2026, to retain relative spans and recompute cutoffs during upgrades instead of freezing the installation timestamp. Existing tests cover recomputation. Retaining a removed global policy is distinct; the reporter's unchanged-timestamp claim needs separate verification.

## Supporting evidence

Source inspection of checkout commit 5cc7f15 supports the configuration-persistence mechanism:

- `crates/uv-cli/src/settings.rs:1098`: tool installation combines CLI/environment resolver options with filesystem options before passing the merged options to installation.
- `crates/uv-tool-commands/src/install.rs:467`: installation converts those merged options to `ToolOptions`.
- `crates/uv-settings/src/settings.rs:2393` and `crates/uv-settings/src/settings.rs:2488`: the receipt options retain `exclude_newer`, but do not retain its CLI-versus-filesystem origin.
- `crates/uv-cli/src/settings.rs:1232`: upgrade keeps CLI/environment options and filesystem options separate.
- `crates/uv-tool-commands/src/upgrade.rs:350`: upgrade explicitly combines options with CLI > receipt > user precedence. Thus a saved enabled cutoff wins over either an absent global value or an explicit global `false`. Reinstall does not change this merge.
- `crates/uv-settings/src/combine.rs`: `Option<ExcludeNewerOverride>` combines using `self.or(other)`, retaining the higher-priority saved value.
- `crates/uv-settings/src/settings.rs:2503`: receipt deserialization reconstructs relative values from their stored spans. `crates/uv/tests/tool/tool_upgrade.rs:548`, `tool_upgrade_recomputes_relative_exclude_newer`, already tests a later upgrade with an explicitly supplied relative cutoff. That test does not change global configuration.
- `crates/uv-settings/src/settings.rs:2557`: an explicit disabled receipt value has no relative span. `docs/concepts/resolution.md:840` documents `--exclude-newer false` as an override of lower-priority settings. Together with upgrade precedence, this supports proposing the CLI workaround; it was not executed during this review.
- `crates/uv-tool-commands/src/upgrade.rs:287`: tool locks are gated by `tool-install-locks`. The lock-enabled branch at line 393 resolves using the same merged settings and writes a new lock from that resolution. Lines 630–635 write the tool lock and merged receipt options even on a no-op in this branch.

astral-sh/uv#18937 introduced the preview tool lock on June 26, 2026. The report does not mention enabling the feature, so that detail is needed to reproduce the lockfile half precisely. It does not undermine the receipt-precedence finding.

The historical changes predate the reported October 2026 release: astral-sh/uv#5937 merged in August 2024, astral-sh/uv#18901 in April 2026, and astral-sh/uv#19934 in June 2026. Review of the latter's changes confirms that it added an explicit disabled state without changing tool upgrade precedence.

## Search coverage and exclusions

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Literal searches covered exclude-newer, exclude-newer-span, false, uv tool upgrade, global configuration, receipts, and per-tool settings. Conceptual searches covered cooldowns, inherited settings, persistence, precedence, and configuration merging without package/version/platform restrictions. PR search omitted known matches, so a complete paginated all-state PR listing was also searched by title. Inspected candidate bodies, comments, reviews, and reference chains, including historical fixes predating 0.13.0. Ruled out astral-sh/uv#20652 and its canonical discussion astral-sh/uv#19196 (unused package exemptions in project locks), astral-sh/uv#18775 (lock-check configuration mismatch), and astral-sh/uv#18522 with astral-sh/uv#18586 (outdated-list settings mismatch). No exact duplicate found.

The report was decomposed before searching into:

1. Obsolete inherited cooldown enforcement after global removal or `false`, including upgrade with reinstall.
2. Retained `exclude-newer`/`exclude-newer-span` metadata and the separate unchanged-timestamp observation.
3. Preserving explicit per-tool policy while allowing inherited policy to follow global changes.

Observable-behavior searches were kept separate from potential-mechanism searches about receipts and precedence. Repository vocabulary included `area:uv-tool`, `area:configuration`, cooldowns, resolver settings, and lock freshness. Queries covered all states; closed-issue and merged-fix checks addressed the version-specific report.

Additional inspected distinctions:

- astral-sh/uv#20652 was closed in favor of astral-sh/uv#19196. Their trigger is unused per-package exemptions appearing in shared project locks and breaking checks on another machine, rather than a saved tool-wide cutoff blocking upgrades.
- astral-sh/uv#18775 concerns global cooldown differences causing `uv lock --check` failures across environments.
- astral-sh/uv#18522 was closed by astral-sh/uv#18586, which made `uv tool list --outdated` respect installed settings. That fixes a listing/upgrade mismatch, not removal of inherited policy.
- astral-sh/uv#8276 concerns intentionally pinned tool versions.
- astral-sh/uv#17999 and astral-sh/uv#18792 concern persistence or expiry of package-specific bypasses during project operations.
- The authentication discussion in astral-sh/uv#8523 references astral-sh/uv#18246, which changes authentication failure handling. The later astral-sh/uv#21275 restores configured authentication for the same saved index while retaining index precedence. These are authentication-specific changes, not general cutoff-origin tracking.
- astral-sh/uv#20279 is an internal borrowing/refactoring change, not a change to policy inheritance.

## Follow-up and validation limits

No runtime reproduction, builds, or test runs were performed. Source inspection establishes the stale-policy mechanism, but not the reporter's exact selected versions or unchanged timestamps. No checkout files or GitHub state were changed; only this handoff was authored.

For implementation work, first cover global removal and explicit global disabling after a tool was installed with an inherited cooldown, checking both selected versions and stored policy. Preserve explicit per-tool values in a separate focused scenario. Reuse the existing relative-cutoff test rather than duplicating its coverage, and inspect lock output with the preview enabled. The relevant tool test module is already gated by `test-python` and `test-pypi` in `crates/uv/tests/tool/main.rs`.

The design must account for existing receipts whose setting origins cannot be recovered. Reversing precedence globally would discard intentional per-tool settings. The CLI workaround explicitly sets policy for the selected tool and is not a repair of inheritance.
