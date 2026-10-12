# Bug: `uv tool upgrade --all` reports `Nothing to upgrade` despite outdated tools

Issue: astral-sh/uv#22497

Classification: bug report; reproduced diagnostic gap fixed, original installation cause unconfirmed

## Summary

The fix adds an explanation when a tool upgrade makes no changes under `lowest` or `lowest-direct` resolution. It names the effective strategy and suggests `uv tool upgrade <tool> --resolution highest`. The updated parent regression verifies the hint and the persisted highest-resolution override. Package selection and outdated-list semantics are unchanged; the original reporter's saved settings remain unknown.

The outdated-list/upgrade-no-op symptom is reproducible on uv 0.13.0 with the real tombi 1.8.1 and 1.8.4 wheels when the installed tool has a saved `resolution = "lowest-direct"` setting. No version constraint or exclude-newer setting is needed. The setting can come from configuration at installation time and persists in the tool receipt. In this variant, retaining the older version follows the requested resolution strategy; the upgrade also emits a lowest-resolution warning. This is not evidence that the reporter has that setting or that their installation has the same cause.

With default resolution settings, an otherwise equivalent reproduction on uv 0.13.0 and Python 3.15.0 successfully upgrades tombi from 1.8.1 to 1.8.4. The runner is Linux x86_64, so the reported Darwin arm64 environment was not exercised. Installing with `@latest` saves no version pin in either experiment.

The reporter has now supplied their global configuration: it enables all preview features, uses managed Python only, explicitly disables exclude-newer, and contains no resolution override. The earlier isolated reproduction did not exercise this configuration. The saved tool receipt and verbose upgrade output are still missing, so neither lowest resolution nor preview behavior is established as the cause.

The historical fix in astral-sh/uv#18586 is included in 0.13.0. Neither that fix nor the closure of astral-sh/uv#19704 establishes a cause for this report. The saved receipt and verbose upgrade output remain the most useful next evidence.

## Classification

The report alleges an upgrade correctness failure and insufficient explanation. The confirmed diagnostic gap is addressed by the focused fix below. The reproduced configuration-dependent version selection is consistent with the documented rule that upgrades retain installation settings and with the existing `tool_upgrade_settings` integration test. It does not establish a regression or a defect in default-resolution upgrades. Keep the original report under investigation until its saved settings and resolver trace are available; do not close it as a confirmed duplicate of a historical fix or attribute it to lowest resolution without that evidence.

## Report details

The reporter uses uv 0.13.0 (commit 4ecd0eff2a81e3ffdcd7cfee10d08936e4747c17, October 9, 2026), Python 3.15.0, and Darwin 27.0.0 arm64. They installed tombi with `uv tool install tombi@latest`. After a later release, `uv tool list --outdated` reports `tombi v1.8.1 [latest: 1.8.4]`, but `uv tool upgrade --all` prints `Nothing to upgrade` and leaves 1.8.1 installed.

The report explicitly denies version bounds and an active exclude-newer cutoff. The supplied global configuration explicitly sets `exclude-newer = false` and contains no resolution or index override. The saved receipt, effective resolution strategy and index settings, verbose upgrade output, and last-known-good uv version remain unavailable. The report requests an upgrade to an allowed newer version and an explanation when configuration or constraints prevent an upgrade.

### Reported global configuration

In the reporter's October 12, 2026 reply to the maintainer's configuration question (comment ID 6129571723 on astral-sh/uv#22497), they supplied these settings from `~/.config/uv/uv.toml`:

| Setting | Reported value |
| --- | --- |
| `required-version` | `">=0.13.0"` |
| `python-preference` | `"only-managed"` |
| `preview-features` | `true` |
| `exclude-newer` | `false` |
| `[audit] malware-check` | `true` |
| `[pip] strict` | `true` |

These are reporter-supplied current settings, not a captured effective configuration or an installation-time receipt. No `resolution` or index setting appears in the supplied file. This does not establish what was persisted when tombi was installed, and does not confirm the lowest-direct explanation reproduced below.

Preview mode is an additional reproduction condition. In the inspected checkout, `PreviewOption::resolve` in `crates/uv-settings/src/settings.rs` maps `preview-features = true` to `Preview::all()`. `PreviewFeature::ToolInstallLocks` is defined in `crates/uv-preview/src/lib.rs`, and both tool installation and upgrade branch on that feature in `crates/uv-tool-commands/src/install.rs` and `upgrade.rs`. This establishes a relevant code path to investigate, not a preview-related root cause. The reporter's actual tool lockfile and resolver trace have not been supplied.

## Reproduction

**Outcome: reproducible, conditionally.** The reported mismatch was observed with a saved lowest-direct resolution strategy, which is additional configuration absent from the report. Default settings did not produce the failure. The reporter's underlying cause remains unconfirmed.

### Environment and fixture

- Executable from PATH: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`, reporting `uv 0.13.0 (x86_64-unknown-linux-gnu)`.
- Host: Linux 6.17.0-1022-azure x86_64. Downloaded CPython 3.15.0 into the temporary reproduction directory and used it for both tool environments.
- All fixture files, tool environments, executable directories, Python downloads, configuration, credentials directory, and caches were isolated under `/tmp/uv-22497-0o7mxaqb`. Commands ran from its `work` directory with an explicitly constructed environment, isolated XDG directories, and keyring disabled. Normal commands disabled configuration discovery; the variant installation explicitly read only the fixture's config file.
- Downloaded the actual PyPI wheels `tombi-1.8.1-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl` and `tombi-1.8.4-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl`, and verified their SHA-256 hashes against public PyPI metadata. Both declare `Requires-Python: >=3.10` and no dependencies. PyPI also lists non-yanked macOS arm64 wheels for both versions; no missing-wheel explanation was established.
- A local HTTP simple index at `http://127.0.0.1:18797` served the real wheels. Each scenario started with only 1.8.1 advertised. After installation, 1.8.4 was added to the same index URL. This models a new release without installing an exact pin, saving an exclude-newer cutoff, or editing a receipt. Separate `/default/simple` and `/lowest/simple` indexes and tool directories kept the two scenarios independent.

### Default settings: upgrade succeeds

With isolated `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, and `UV_NO_CONFIG=1`:

```sh
# The index initially advertises only tombi 1.8.1.
uv tool install tombi@latest --python 3.15.0 \
  --default-index http://127.0.0.1:18797/default/simple
# Add the 1.8.4 wheel link to the same index, retaining 1.8.1.
uv tool list --outdated --show-version-specifiers --show-python
uv tool upgrade --all
uv tool list --outdated
uv tool list --show-python
```

The saved requirement was `requirements = [{ name = "tombi" }]`, with no constraint, resolution override, or exclude-newer option. The outdated listing showed:

```text
tombi v1.8.1 [CPython 3.15.0] [latest: 1.8.4]
- tombi
```

Upgrade succeeded, exiting zero and printing:

```text
Updated tombi v1.8.1 -> v1.8.4
 - tombi==1.8.1
 + tombi==1.8.4
Installed 1 executable: tombi
```

The subsequent outdated listing was empty, and the ordinary listing confirmed tombi 1.8.4 on CPython 3.15.0.

### Saved lowest-direct resolution: symptom reproduced

The fixture `lowest-config.toml` contained only:

```toml
resolution = "lowest-direct"
```

Using a fresh tool directory and an index initially advertising only 1.8.1, installation explicitly loaded this configuration:

```sh
uv --config-file /tmp/uv-22497-0o7mxaqb/lowest-config.toml \
  tool install tombi@latest --python 3.15.0 \
  --default-index http://127.0.0.1:18797/lowest/simple
```

The resulting receipt contained an unconstrained requirement and saved resolution strategy:

```toml
[tool]
requirements = [{ name = "tombi" }]
python = "3.15.0"
# Entrypoint omitted here.

[tool.options]
index = [{ url = "http://127.0.0.1:18797/lowest/simple", explicit = false, default = true, format = "simple", authenticate = "auto" }]
keyring-provider = "disabled"
resolution = "lowest-direct"
```

After adding 1.8.4 to that same index, configuration discovery was disabled (`UV_NO_CONFIG=1`) so the saved receipt alone supplied the resolution strategy:

```sh
uv tool list --outdated --show-version-specifiers --show-python
uv tool upgrade --all
uv tool list --show-version-specifiers --show-python
```

The outdated listing again reported 1.8.4. Upgrade exited zero and produced:

```text
warning: The direct dependency `tombi` is unpinned. Consider setting a lower bound when using `--resolution lowest` or `--resolution lowest-direct` to avoid using outdated versions.
Nothing to upgrade
```

The ordinary listing confirmed that 1.8.1 remained installed. Unlike the report's excerpt, this reproduction includes a warning; it establishes the mismatch and no-op summary, not identical full output.

Overriding the saved strategy confirmed its effect in this fixture:

```sh
uv tool upgrade --all --resolution highest
uv tool list --outdated
uv tool list --show-python
```

This upgraded 1.8.1 to 1.8.4, left the outdated listing empty, and confirmed the new installed version. This control establishes the reason for the fixture's no-op, not the reason for the reporter's no-op.

### Existing test coverage and limits

Read the setup and assertions of the following integration tests:

- `crates/uv/tests/tool/tool_upgrade.rs::tool_upgrade_settings`: installs `black>=23` with `--resolution=lowest-direct` and expects 23.1.0. The parent regression from astral-sh/uv-dev#2605 adds an outdated listing showing 24.3.0 and changes the no-op upgrade to `--all`. The fix retains that sequence, adds the resolution hint to the expected no-op output, and verifies that overriding with `--resolution=highest` upgrades to 24.3.0 and persists the strategy for the next upgrade. It does not use `@latest` or tombi.
- `crates/uv/tests/tool/tool_upgrade.rs::tool_upgrade_all`: installs older synthetic babel and python-dotenv releases from one index, then supplies a newer index to `upgrade --all` and expects both upgrades. It also verifies that an invalid tool directory does not prevent valid upgrades.
- `crates/uv/tests/tool/tool_install.rs::tool_install_at_latest`: installs `black@latest` and snapshots an unconstrained saved requirement, establishing that the syntax does not persist an exact pin.
- `crates/uv/tests/tool/tool_list.rs::tool_list_outdated_respects_exclude_newer`: installs black with a persisted cutoff and expects an empty outdated listing. It covers the historical saved-cutoff fix, not the reported no-cutoff case.

These modules are gated by `test-python` and `test-pypi` in `crates/uv/tests/tool/main.rs`. The test context also applies a fixed exclude-newer timestamp, which the reproduction deliberately did not use. No direct test of the full tombi sequence was found, and no matching tool-upgrade test was found under `crates/uv-client/tests/it/`. During the initial reproduction, existing Rust tests were inspected, not executed; behavioral checks used the installed uv binary. The subsequent fix investigation built and ran the focused debug-profile tests listed below.

No last-known-good uv release is specified, so no before/after version comparison was made. The same reported uv and Python versions were tested, but native Darwin arm64 behavior and the newly supplied global configuration remain untested. The recorded isolated commands do not enable preview features; the successful default-settings control therefore does not rule out a failure specific to the reporter's preview-enabled setup.

## Related

- astral-sh/uv-dev#2605 (parent regression pull request) — Extends `tool_upgrade_settings` with an outdated listing followed by a no-op `upgrade --all`. The unchanged parent test passed locally before the expected diagnostic was updated.

- astral-sh/uv#18522 (issue, closed) — `uv tool upgrade --all` does not update anything. Closest historical symptom match: on uv 0.10.10, outdated tools, including tombi, remained unchanged with 'Nothing to upgrade', while reinstalling with @latest worked. Closed by astral-sh/uv#18586, which fixed outdated checks ignoring saved settings. The new report occurs after that fix and explicitly excludes a configured cooldown.

- astral-sh/uv#18586 (pull request, merged) — Respect installed settings in `uv tool list --outdated`. Merged March 21, 2026, closing astral-sh/uv#18522. Its diff makes outdated checks use saved index, prerelease and exclude-newer settings and consider the tool interpreter; its regression test covers a saved exclude-newer cutoff. Commit ancestry confirms this fix is in 0.13.0, but the same underlying failure has not been established here.

- astral-sh/uv#19704 (issue, closed) — Issue with tool versioning. Same outdated-list/upgrade-no-op symptom on uv 0.11.19. A maintainer requested verbose upgrade output to check for pins; the reporter supplied verbose list output and closed after installing markitdown@latest. No fixing PR or confirmed cause appears in its timeline, so closure does not establish a fix.

- astral-sh/uv#22468 (issue, open) — Bug: `uv tool upgrade` doesn't respect changes to the global `exclude-newer` setting. Adjacent upgrade blocker on the same uv, Python and platform versions: an inherited cooldown persists in the tool receipt after global configuration changes. This makes saved receipt options relevant evidence. The reporter now shows `exclude-newer = false` in the current global config, but no receipt or prior configuration history; retained cooldown settings are neither established nor ruled out. It is not a confirmed duplicate.

- astral-sh/uv#16081 (pull request, merged) — Surface pinned-version hint when `uv tool upgrade` can’t move the tool. Implemented the exact-pin explanation requested in astral-sh/uv#15665; current source and integration tests contain that hint. Relevant to the secondary request for an explanation, but it only diagnoses exact version pins, and @latest does not persist such a pin.

- astral-sh/uv#13968 (issue, open) — Warn when `--upgrade-package <name>` does not result in an upgrade. Tracks the broader request to explain why requested upgrades cannot proceed, with maintainer discussion of resolver incompatibilities and Python requirements. It concerns --upgrade-package diagnostics rather than this tool-upgrade failure, so it overlaps only with the report's secondary request.

## Supporting evidence

- The previous issue-context investigation confirmed that astral-sh/uv#18586 closed astral-sh/uv#18522 and is an ancestor of 0.13.0. The checkout's `changelogs/0.11.x.md` records the saved-settings fix. `changelogs/0.12.x.md` also records configured-index support from astral-sh/uv#20770. The 0.13.0 release notes describe Python 3.15 becoming the default and HTTP cache changes; neither establishes the cause of this report.
- `crates/uv-tool-commands/src/install.rs` constructs an unconstrained registry requirement for `@latest`. Both observed receipts confirm this behavior.
- `crates/uv-tool-commands/src/list.rs` obtains the latest version using saved index, prerelease, and exclude-newer settings, without full dependency resolution or a wheel-tag filter. A listed newer version alone does not prove that upgrade will select it under every saved resolution strategy.
- `crates/uv-tool-commands/src/upgrade.rs` combines CLI settings, receipt settings, and user configuration in that precedence order, then resolves the saved requirements and constraints. The saved lowest-direct strategy and CLI highest override were verified behaviorally above.
- `docs/concepts/tools.md` documents retention of installation settings during upgrades. `docs/concepts/resolution.md` documents lowest-direct's preference for the lowest direct-dependency versions. The reproduced no-op is consistent with those semantics.
- The prior inspection of astral-sh/uv#19704 found closure after reinstalling with `@latest`, without a confirmed root cause or fixing commit. Its closure is not evidence of a regression here.

## Search coverage and exclusions

The following summarizes the existing issue-context investigation; no new GitHub changes were made during reproduction.

Searched astral-sh/uv open and closed issues and open, closed and merged PRs through authenticated gh. Separated upgrade failure from missing explanations. Literal searches covered 'Nothing to upgrade', tool upgrade, tool list --outdated, @latest, tombi and 0.13.0; conceptual searches covered upgrade refusal, pins, constraints, warnings and area:uv-tool. Separately searched saved exclude-newer settings as a possible cause. PR searches returned no hits; followed issue timelines to inspect merged fixes, comments, diffs and release ancestry. Ruled out astral-sh/uv#19325 (dependency-only upgrades missing from the list), astral-sh/uv#8276 (explicit pins), astral-sh/uv#17548 (Windows executable-copy failure), and astral-sh/uv#21058 / astral-sh/uv#21368 (invalid tool directories, already fixed). Also checked astral-sh/uv#18819 / astral-sh/uv#18861 (exclude-newer support), astral-sh/uv#12052 (unreproduced transient failure) and astral-sh/uv#13733 (project dependency upgrade conflicts). No confirmed open duplicate of the reported failure was found.

The strongest title match among open issues, astral-sh/uv#19325, is the opposite mismatch: upgrades modify dependency-only environments that the outdated list omits. It does not track an outdated primary tool that refuses to upgrade. astral-sh/uv#8276 concerns intentionally pinned versions, while astral-sh/uv#17548 follows a failed executable replacement on Windows; neither trigger is reported here.

astral-sh/uv#21058 was an enumeration error caused by invalid directory names, addressed by astral-sh/uv#21368 on August 31, 2026. The old reproduction also made tool listing fail, unlike the successful listing in this report. The current upgrade code propagates enumeration errors. This historical cause is not established as a regression here.

astral-sh/uv#18819 and astral-sh/uv#18861 concern explicit exclude-newer support for outdated listings. The latter merged April 7, 2026. They do not establish a cause for a report that denies this setting. astral-sh/uv#12052 became unreproducible without a confirmed fix. astral-sh/uv#13733 concerned project resolution and was resolved by upgrading another dependency alongside the target, rather than tool management.

PR keyword searches returned no results, including searches across all states and explicit open, closed, and merged filters. The related merged PRs were found through issue timeline references and inspected directly. The search result should not be read as proof that no other PR exists.

## Fix

**Outcome: fixed for the confirmed missing explanation.** The original report's alleged default-resolution upgrade failure remains unconfirmed; this change does not force tools with a saved lowest-version strategy to upgrade.

### Confirmed cause and scope

`ToolOptions` persists the install-time resolution strategy, and `upgrade_tool` combines CLI settings, receipt settings, and user configuration in that order. The resolver intentionally chooses the lowest compatible direct-tool version under `lowest-direct`. The outdated listing separately asks for the newest available version. Before the fix, a no-op upgrade collected an explanatory hint only for an exact version pin, leaving the parent fixture's saved lowest-resolution strategy unexplained.

Inspected both receipt conversions in `crates/uv-settings/src/settings.rs`, installation receipt creation, upgrade receipt reads and writes, resolver candidate selection, and the outdated-list lookup. Neighboring `tool_install_settings` tests intentionally preserve or replace the strategy in receipts, and `tool_list_outdated` intentionally lists newer releases even for an exactly pinned tool. These are compatible contracts, not additional failing cases. No distinct demonstrated failure required a new test or changes in another integration-test module. Both lowest-version enum variants share the same diagnostic path; no redundant option matrix was added.

### Implementation and regression

`crates/uv-tool-commands/src/upgrade.rs` adds a lowest-resolution diagnostic to the existing upgrade hint mechanism. It applies only to no-op outcomes with an effective `lowest` or `lowest-direct` strategy, gives existing exact-pin hints precedence, and suggests the highest-resolution override without promising that all other constraints permit an upgrade. Successful upgrades, dependency-only changes, errors, default highest-resolution no-ops, and outdated listings retain their behavior.

The original parent file `crates/uv/tests/tool/tool_upgrade.rs` remains the only modified integration-test file. Its `tool_upgrade_settings` test keeps the outdated listing and no-op version selection, expects the new hint, executes the suggested override, and performs another upgrade to verify that the saved highest strategy no longer triggers a hint. The existing `test-python` and `test-pypi` module gates apply.

### Focused validation

1. The unchanged parent `tool_upgrade::tool_upgrade_settings` passed using `cargo test --locked -p uv --test tool tool_upgrade::tool_upgrade_settings -- --exact`.
2. After updating only its expected output, the same test failed with `INSTA_UPDATE=no`: the observed no-op output lacked the newly expected hint. Production code was changed only after this failure was confirmed.
3. After the fix, all six selected integration tests passed in the debug/test profile:

   ```sh
   cargo test --locked -p uv --test tool -- \
     tool_upgrade::tool_upgrade_settings \
     tool_upgrade::tool_upgrade_pinned_hint \
     tool_upgrade::tool_upgrade_pinned_hint_with_mixed_constraint \
     tool_upgrade::tool_upgrade_all \
     tool_install::tool_install_settings \
     tool_list::tool_list_outdated --exact
   ```

   These cover the parent regression and install/upgrade/receipt round trip, existing exact-pin diagnostics, ordinary upgrades, installation-setting persistence, and the established outdated-list contract.
4. `RUSTUP_TOOLCHAIN=stable cargo fmt --all` and `git diff --check` passed. The pinned 1.99.0 toolchain lacks rustfmt and clippy; its read-only installation prevented adding rustfmt, so formatting used the installed stable toolchain. Clippy was not run.

No release build, full test suite, dependency changes, commits, pushes, or GitHub writes were performed. Native Darwin arm64 was not tested. The original reporter's sanitized receipt and verbose resolver output are still needed to determine whether this diagnostic addresses their installation.

## Follow-up and remaining evidence

The default-settings reproduction upgrades successfully. A saved `resolution = "lowest-direct"` setting can produce the same outdated-list/no-op mismatch without version bounds or exclude-newer, but it also produced a warning in the test. Check the reporter's saved settings before concluding that this explains their installation.

The global configuration question has been answered; the supplied file should be used as an input to the next isolated reproduction. Compare preview-enabled and preview-disabled behavior using separate tool environments and the same release-transition fixture, including the reported managed-Python preference and disabled cutoff. This comparison has not been performed, and disabling preview is not a confirmed workaround.

Before reinstalling tombi changes the evidence, the remaining diagnostic material is `uv tool upgrade tombi -v`, `uv tool list --outdated --show-version-specifiers --show-python`, and the non-secret requirements, constraints, overrides, resolution, index identities, and exclude-newer settings from tombi's `uv-receipt.toml`. Any tool `uv.lock` should also be considered when investigating the preview path, using only diagnostic metadata with credentials removed. Effective environment or CLI overrides and installation-time configuration remain unknown. If lowest resolution is present, `uv tool upgrade tombi --resolution highest` is a useful targeted comparison. If it is absent, the verbose resolver trace, preview comparison, and native macOS reproduction remain necessary to narrow the cause.

The initial reproduction wrote only the issue-context README and temporary files. The fix changes only `crates/uv-tool-commands/src/upgrade.rs` and `crates/uv/tests/tool/tool_upgrade.rs`, plus this README outside the checkout. No GitHub state was changed. Credentials were not inspected or exposed.

Pull request: astral-sh/uv-dev#2606
