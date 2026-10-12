# Bug: `uv tool upgrade` doesn't respect changes to the global `exclude-newer` setting

Issue: astral-sh/uv#22468

Classification: bug

## Summary

The core reported behavior is **reproducible** with the installed uv 0.12.13 on Linux x86_64 and Python 3.12.3. A tool installed while the user-level configuration contains `exclude-newer = "3 days"` retains that policy after the configuration changes to `false`. Both `uv tool upgrade --all` and `uv tool upgrade --all --reinstall` keep the older tool version. With `UV_PREVIEW_FEATURES=tool-install-locks`, changing the setting to `false` or removing it retains `exclude-newer-span = "P3D"` in both the receipt and lockfile. A fresh installation with the global setting disabled selects the newer release.

The reporter uses uv 0.13.0 (4ecd0eff2, October 9, 2026), Python 3.15.0, and Darwin 27.0.0 arm64, and names Black as the affected tool. That exact version/platform combination was not run. The reproduction uses a dependency-free synthetic tool and a local index to ensure a release exists inside the three-day exclusion window.

The unchanged-timestamp detail is only partly reproduced: without preview locks, the receipt timestamp remains unchanged during the no-op upgrade and reinstall; with preview locks, the receipt timestamp advances. The preview lock retains a sentinel timestamp and the relative span. The CLI-workaround recommendation needs qualification: explicit CLI disabling upgrades successfully without preview locks, but only clears policy metadata in the tested preview-lock path.

## Classification

Retain the bug classification for the reported failure of an inherited global policy to follow configuration changes. Reproduction establishes observable persistence and blocked upgrades; it does not by itself decide whether the intended configuration contract requires different precedence.

The installed `uv help tool upgrade` explicitly documents retaining settings supplied during installation. Source at checkout commit `5cc7f15` implements CLI > receipt > user configuration precedence, and receipts do not record whether `exclude-newer` came from the command line or global configuration. Retaining explicit per-tool settings is intentional; distinguishing those from inherited settings is the design question raised by this report. Changing precedence globally could discard intentional per-tool settings, and existing receipts cannot recover the original setting source.

No regression from a previously working version was established. The behavior already occurs in the available uv 0.12.13. Related authentication and list-merging issues discuss the same precedence model but do not directly track clearing an inherited scalar cutoff.

## Reproduction

Outcome: **reproducible**, with the platform/version and timestamp qualifications below.

### Environment and isolation

- Executable on PATH: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`, reporting `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Platform: Linux `6.17.0-1022-azure`, x86_64; Python 3.12.3.
- Validation ran on October 12, 2026, using the real clock, without internal test-clock overrides.
- All fixtures, configuration, tools, executable directories, Python installation directories, caches, and logs are under `/tmp/uv-issue-22468-GkZv6Z/validated`. Each scenario has separate state. The uv subprocess environment is constructed explicitly, without inheriting credentials or ambient uv settings. Configuration is discovered through `XDG_CONFIG_HOME`, not supplied as a CLI cutoff override. `XDG_CONFIG_DIRS`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `UV_CACHE_DIR`, `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, `UV_PYTHON_INSTALL_DIR`, and `TMPDIR` point into the temporary scenario directory. Python downloads are disabled; the existing Python interpreter is selected explicitly.
- The reproduction server binds only to `127.0.0.1`. No external package downloads, source builds, or GitHub mutations are needed.

### Fixture and commands

A local JSON Simple API index exposes two valid pure-Python wheels for `cooldown-demo`, with a console entrypoint and no dependencies. Both have `Requires-Python: >=3.8`. Release 1.0.0 has an advertised upload time ten days before the run; release 2.0.0 is one day old. In the validated run these times were `2026-10-02T00:22:09Z` and `2026-10-11T00:22:09Z`. This makes 2.0.0 a compatible upgrade that the inherited three-day policy excludes. Installed versions are read from distribution metadata using the tool environment's Python.

The initial user-level file at `$XDG_CONFIG_HOME/uv/uv.toml` contains:

```toml
index-url = "http://127.0.0.1:39953/simple"
exclude-newer = "3 days"
```

The localhost port is allocated dynamically. After installing the tool, replace only the cutoff with `exclude-newer = false`, retaining the same index URL:

```console
uv tool install cooldown-demo
# Change the user-level exclude-newer setting to false.
uv tool upgrade --all
uv tool upgrade --all --reinstall
uv tool upgrade --all --exclude-newer false
```

A separate scenario starts from a fresh installation with the original three-day setting and `UV_PREVIEW_FEATURES=tool-install-locks`, then runs those same commands after global disabling. Another fresh preview-lock scenario removes the cutoff line entirely before the two upgrade commands. A separate fresh install with global `exclude-newer = false` provides a control. Removing the line was tested; commenting it out was not separately tested.

### Observed results

All commands exited successfully.

| Scenario | Initial install | After global change and upgrade | After upgrade with reinstall | Saved policy after those upgrades |
| --- | --- | --- | --- | --- |
| Global `false`, no preview | 1.0.0 | 1.0.0; `Nothing to upgrade` | Reinstalls 1.0.0 | Receipt retains `P3D`; no lockfile exists |
| Global `false`, preview locks | 1.0.0 | 1.0.0; `Nothing to upgrade` | Reinstalls 1.0.0 | Receipt and lock retain `P3D` |
| Global setting removed, preview locks | 1.0.0 | 1.0.0; `Nothing to upgrade` | Reinstalls 1.0.0 | Receipt and lock retain `P3D` |
| Fresh install with global `false`, preview locks | 2.0.0 | Not applicable | Not applicable | Receipt records `false`; lock has no cutoff options |

The default-path CLI control, `uv tool upgrade --all --exclude-newer false`, reports `Updated cooldown-demo v1.0.0 -> v2.0.0`, and installed distribution metadata confirms 2.0.0. The receipt changes to `exclude-newer = false` and drops the span. This establishes that the newer release is compatible and that the retained cutoff prevents the default-path upgrade.

With preview locks enabled, the same CLI control removes the span, stores `exclude-newer = false` in the receipt, and removes the lock's cutoff options, but reports `Nothing to upgrade` and retains 1.0.0. Adding explicit `--upgrade` also retains 1.0.0 and warns that the option is already enabled by default. The cause of this additional preview-path behavior was not established. Therefore, do not promise that the CLI override alone upgrades tools with preview locks, or attribute every preview-path failure to the retained cutoff.

### Timestamp qualification

Without preview locks, the receipt's `exclude-newer = "2026-10-09T00:22:09.70244094Z"` remains unchanged through the no-op upgrade and reinstall. This is consistent with the reporter's receipt observation.

With preview locks, the receipt timestamp advances from `2026-10-09T00:22:10.202115138Z` at installation to `2026-10-09T00:22:10.32774613Z` on upgrade and `2026-10-09T00:22:10.457151167Z` on reinstall. Its `P3D` policy still persists. The lock contains the unchanged `exclude-newer = "0001-01-01T00:00:00Z"` plus `exclude-newer-span = "P3D"`. This sentinel is not the original installation cutoff; unchanged lock text does not establish that an absolute cutoff was frozen. The report's claim that both files retain their original effective timestamps is not fully reproduced.

### Existing test coverage

Read the setup and assertions of these integration tests:

- `crates/uv/tests/tool/tool_install.rs`, `tool_install_relative_exclude_newer_receipt_preserves_span`: installs Black with an explicit CLI three-week cutoff and snapshots both the computed timestamp and `P3W` receipt span.
- `crates/uv/tests/tool/tool_upgrade.rs`, `tool_upgrade_recomputes_relative_exclude_newer`: installs Black with an explicit CLI three-week cutoff at one simulated date, upgrades at a later date, checks the version upgrade, and snapshots the recomputed receipt timestamp. It does not change global configuration.
- `crates/uv/tests/tool/tool_upgrade.rs`, `tool_upgrade_settings`: installs with explicit `--resolution=lowest-direct`, confirms a subsequent upgrade is a no-op, and confirms `--resolution=highest` overrides the saved setting.
- `crates/uv/tests/tool/tool_install.rs`, `tool_install_locks_are_preview`: checks that installation omits `uv.lock` by default and creates it when `tool-install-locks` is enabled.
- `crates/uv/tests/tool/tool_upgrade.rs`, `tool_upgrade_writes_preview_lock`: starts with an installation without a lock and checks that upgrading with the preview enabled writes a lock.

None of these tests covers changing an inherited global cutoff to `false` or removing it after installation. Searches in the tool tests and `crates/uv-client/tests/it/` found no test for that transition. The tool test modules are gated by both `test-python` and `test-pypi` in `crates/uv/tests/tool/main.rs`. No repository tests were added or executed; validation used the installed executable and temporary fixtures.

### Artifacts and limits

The final driver is `/tmp/uv-issue-22468-GkZv6Z/validate.py`; its output is `/tmp/uv-issue-22468-GkZv6Z/validated.log`. The `validated` directory contains the index JSON, wheels, aggregate `results.json`, and per-step command logs, receipts, and locks. The server was stopped after validation. For another run, copy the driver to a fresh temporary directory so its adjacent `validated` directory starts empty.

The exact uv 0.13.0/macOS/Python 3.15 environment, actual Black releases, and origin-preservation behavior for separately configured per-tool settings were not tested. The reporter does not mention the `tool-install-locks` preview feature; it was required to create the lockfile half of the reproduction. No checkout files or existing user state were changed, and no GitHub state was changed.

## Related

- astral-sh/uv#8523 (open issue) — uv tool upgrade - problem with private indexes due to short living credentials. Upgrades reuse receipt values after the user updates global configuration. Maintainer comments discuss saving only explicit CLI overrides and distinguishing configuration origins. Its concrete failure concerns expiring index authentication, whereas this report requires clearing an inherited cutoff.
- astral-sh/uv#11929 (open issue) — Upgrading tools adds an identical entry to the `index` list in uv-receipt.toml. Maintainers identify merging saved receipt settings with current filesystem settings as the cause and discuss replacement semantics. This concerns accumulating list entries; the new report concerns a saved scalar cutoff overriding changed or removed configuration.
- astral-sh/uv#5937 (merged pull request) — Retain and respect settings in tool upgrades. Introduced persisted resolver/installer options and CLI > receipt > filesystem precedence, which remains in the upgrade implementation. This establishes the origin of the behavior, rather than an existing fix for inherited-setting removal.
- astral-sh/uv#19934 (merged pull request) — Allow disabling `exclude-newer`. Merged June 22, 2026, closing astral-sh/uv#19929. Added false overrides in CLI, environment, configuration, and receipts. It supports explicit CLI disabling, which upgraded the tool in the default-path reproduction, but did not change receipt precedence or track inherited values. This report is not evidence that its fix regressed.
- astral-sh/uv#18901 (merged pull request) — Store relative timestamps in tool receipts. Merged April 7, 2026, to retain relative spans and recompute cutoffs during upgrades instead of freezing the installation timestamp. Existing tests cover recomputation. Retaining a removed global policy is distinct. Runtime validation found unchanged receipt timestamps without preview locks and advancing receipt timestamps with preview locks.

- astral-sh/uv#18937 (merged pull request) — Added preview tool lockfiles on June 26, 2026. The reproduction required `tool-install-locks` to create `uv.lock`; the report does not state whether this feature is enabled.

## Supporting evidence

Source inspection of checkout commit `5cc7f15` is consistent with the observed inherited-policy persistence:

- `crates/uv-cli/src/settings.rs`, `ToolInstallSettings::resolve`: merges CLI/environment options with filesystem settings before installation.
- `crates/uv-tool-commands/src/install.rs`: converts these merged settings to `ToolOptions` for the receipt.
- `crates/uv-settings/src/settings.rs`, `ToolOptions`: stores the cutoff without recording whether it was inherited or explicitly supplied.
- `crates/uv-cli/src/settings.rs`, `ToolUpgradeSettings::resolve`: keeps current CLI/environment and filesystem settings separate.
- `crates/uv-tool-commands/src/upgrade.rs:350`: explicitly combines settings as CLI > receipt > user configuration. Reinstall does not bypass this merge.
- `crates/uv-settings/src/combine.rs`: the optional cutoff uses higher-priority `Option::or` semantics, so a saved enabled value wins over current global `false` or absence.
- `crates/uv-settings/src/settings.rs`, conversions between `ToolOptionsWire` and `ToolOptions`: restores relative spans and stores explicit disabling without a span.
- `crates/uv-tool-commands/src/upgrade.rs`: gates tool locks on `ToolInstallLocks` and writes receipt/lock metadata even for a no-op on that path, consistent with the observed timestamp difference.

The release notes confirm that saved settings, relative receipt spans, boolean disabling, and preview tool locks predate uv 0.13.0. The 0.13.0 notes do not identify a change to inherited cutoff precedence. These source and release-note findings support interpretation of the runtime results; they are not substitutes for reproduction or proof of the additional preview-path upgrade cause.

## Prior related-issue search

The following search record is retained from the earlier issue-context review; no new GitHub search was performed during reproduction.

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

## Follow-up

A focused regression test should install with an inherited global cooldown, change that configuration to `false` or remove it, and assert both selected versions and saved policy. Explicit per-tool values need a separate preservation scenario if inheritance semantics change. Reuse the existing relative-time test for recomputation coverage and account for the tool test feature gates.

A maintainer decision is needed on inherited-versus-explicit policy semantics and compatibility with receipts that do not record origins. The preview-path failure to advance versions even after an explicit CLI override deserves separate investigation before presenting that override as a universal workaround. Exact uv 0.13.0/platform validation would refine these qualifications but is not needed to establish the reproduced persistence behavior.
