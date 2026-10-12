# `uv tool upgrade --all` reports `Nothing to upgrade` despite outdated tools

Issue: astral-sh/uv#22497

Classification: bug

## Summary

Closest matches are astral-sh/uv#18522 and its merged fix astral-sh/uv#18586, plus the unresolved cause behind closed astral-sh/uv#19704. Saved cooldowns and upgrade explanations have adjacent discussions, but no confirmed open duplicate was found.

## Draft response

Installing with `@latest` does not save an exact version pin, so that syntax alone should not prevent future upgrades. astral-sh/uv#18522 was fixed by astral-sh/uv#18586, which made outdated checks respect saved tool settings; that fix is included in 0.13.0. astral-sh/uv#19704 closed after reinstalling, without a confirmed cause.

Before reinstalling tombi, please share the output of `uv tool upgrade tombi -v` and `uv tool list --outdated --show-version-specifiers --show-python`, plus tombi's saved requirements and options from its `uv-receipt.toml`, with credentials removed. Those details are needed to determine why the upgrade keeps 1.8.1 and whether this is a regression.

## Classification

The report describes a failed upgrade and misleading output for an allegedly unconstrained tool. The similar historical bug in astral-sh/uv#18522 was fixed by astral-sh/uv#18586, and that fix is included in 0.13.0. This recurrence warrants investigation as a possible regression or an uncovered case, not closure as a duplicate of the fixed issue. The underlying mechanism remains unconfirmed; the inspected open issues do not establish the same failure.

## Related

- astral-sh/uv#18522 (issue, closed) — `uv tool upgrade --all` does not update anything. Closest historical symptom match: on uv 0.10.10, outdated tools, including tombi, remained unchanged with 'Nothing to upgrade', while reinstalling with @latest worked. Closed by astral-sh/uv#18586, which fixed outdated checks ignoring saved settings. The new report occurs after that fix and explicitly excludes a configured cooldown.

- astral-sh/uv#18586 (pull request, merged) — Respect installed settings in `uv tool list --outdated`. Merged March 21, 2026, closing astral-sh/uv#18522. Its diff makes outdated checks use saved index, prerelease and exclude-newer settings and consider the tool interpreter; its regression test covers a saved exclude-newer cutoff. Commit ancestry confirms this fix is in 0.13.0, but the same underlying failure has not been established here.

- astral-sh/uv#19704 (issue, closed) — Issue with tool versioning. Same outdated-list/upgrade-no-op symptom on uv 0.11.19. A maintainer requested verbose upgrade output to check for pins; the reporter supplied verbose list output and closed after installing markitdown@latest. No fixing PR or confirmed cause appears in its timeline, so closure does not establish a fix.

- astral-sh/uv#22468 (issue, open) — Bug: `uv tool upgrade` doesn't respect changes to the global `exclude-newer` setting. Adjacent upgrade blocker on the same uv, Python and platform versions: an inherited cooldown persists in the tool receipt after global configuration changes. This makes saved receipt options relevant evidence, but the new report denies exclude-newer configuration and does not establish that mechanism; it is not a confirmed duplicate.

- astral-sh/uv#16081 (pull request, merged) — Surface pinned-version hint when `uv tool upgrade` can’t move the tool. Implemented the exact-pin explanation requested in astral-sh/uv#15665; current source and integration tests contain that hint. Relevant to the secondary request for an explanation, but it only diagnoses exact version pins, and @latest does not persist such a pin.

- astral-sh/uv#13968 (issue, open) — Warn when `--upgrade-package <name>` does not result in an upgrade. Tracks the broader request to explain why requested upgrades cannot proceed, with maintainer discussion of resolver incompatibilities and Python requirements. It concerns --upgrade-package diagnostics rather than this tool-upgrade failure, so it overlaps only with the report's secondary request.

## Report details

The reporter uses uv 0.13.0 (commit 4ecd0eff2a81e3ffdcd7cfee10d08936e4747c17, October 9, 2026), Python 3.15.0, and Darwin 27.0.0 arm64. They installed tombi with `uv tool install tombi@latest`. After a later release, `uv tool list --outdated` reports `tombi v1.8.1 [latest: 1.8.4]`, but `uv tool upgrade --all` prints `Nothing to upgrade` and leaves 1.8.1 installed.

The report explicitly denies version bounds and an exclude-newer setting. It requests both an upgrade to an allowed newer version and an explanation when configuration or constraints prevent an upgrade. These are separate claims: the reported failure is a correctness issue, while general explanations overlap existing diagnostic work.

## Supporting evidence

- The timeline for astral-sh/uv#18522 identifies astral-sh/uv#18586 as its closing fix. That PR merged on March 21, 2026, at commit c1517ec647a72a698b2af4d77927b31d9e3d83b3. GitHub's comparison with 0.13.0 reports the release ahead by 2,180 commits, behind by zero, with that fix as the merge base. This confirms inclusion in the reported release; it does not establish that the new report has the same cause.
- The diff for astral-sh/uv#18586 changes the outdated lookup to use each tool's saved settings. Its added test uses an exclude-newer cutoff and expects no outdated entry. The corresponding test remains at `crates/uv/tests/tool/tool_list.rs:188`.
- `crates/uv-tool-commands/src/install.rs:254` constructs an unconstrained registry requirement for `@latest`. The temporary resolution constraint is separate from the constraints saved to the receipt (`install.rs:441`, `install.rs:681`, and `install.rs:1067`). The existing `tool_install_at_latest` test at `crates/uv/tests/tool/tool_install.rs:4793` snapshots a receipt with `requirements = [{ name = "black" }]`, without a version pin. The install syntax alone therefore does not explain the failure.
- `crates/uv-tool-commands/src/list.rs:131` combines CLI, receipt, and filesystem settings for outdated checks. The lookup at `list.rs:153` is a latest-package lookup, with no full dependency resolution and no wheel-tag filter. Its displayed latest version alone does not prove that the full upgrade can select that version. No package-compatibility cause has been established for tombi here.
- `crates/uv-tool-commands/src/upgrade.rs:350` applies CLI > receipt > user configuration precedence and then resolves saved requirements and constraints. This makes the receipt useful evidence even when current global configuration has no cooldown; it does not prove the stored-cooldown mechanism in astral-sh/uv#22468 applies.
- The upgrade command emits `Nothing to upgrade` when no tools are found or no upgrade outcome is recorded (`upgrade.rs:97` and `upgrade.rs:195`). Exact pins receive a separate hint (`upgrade.rs:243` and `upgrade.rs:653`), covered by `crates/uv/tests/tool/tool_upgrade.rs:665` and the merged astral-sh/uv#16081.
- The timeline and comments for astral-sh/uv#19704 show a request for verbose upgrade diagnostics, verbose list diagnostics instead, and closure after reinstalling with `@latest`. There is no linked fixing commit or confirmed root cause.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed and merged PRs through authenticated gh. Separated upgrade failure from missing explanations. Literal searches covered 'Nothing to upgrade', tool upgrade, tool list --outdated, @latest, tombi and 0.13.0; conceptual searches covered upgrade refusal, pins, constraints, warnings and area:uv-tool. Separately searched saved exclude-newer settings as a possible cause. PR searches returned no hits; followed issue timelines to inspect merged fixes, comments, diffs and release ancestry. Ruled out astral-sh/uv#19325 (dependency-only upgrades missing from the list), astral-sh/uv#8276 (explicit pins), astral-sh/uv#17548 (Windows executable-copy failure), and astral-sh/uv#21058 / astral-sh/uv#21368 (invalid tool directories, already fixed). Also checked astral-sh/uv#18819 / astral-sh/uv#18861 (exclude-newer support), astral-sh/uv#12052 (unreproduced transient failure) and astral-sh/uv#13733 (project dependency upgrade conflicts). No confirmed open duplicate of the reported failure was found.

The strongest title match among open issues, astral-sh/uv#19325, is the opposite mismatch: upgrades modify dependency-only environments that the outdated list omits. It does not track an outdated primary tool that refuses to upgrade. astral-sh/uv#8276 concerns intentionally pinned versions, while astral-sh/uv#17548 follows a failed executable replacement on Windows; neither trigger is reported here.

astral-sh/uv#21058 was an enumeration error caused by invalid directory names, addressed by astral-sh/uv#21368 on August 31, 2026. The old reproduction also made tool listing fail, unlike the successful listing in this report. The current upgrade code propagates enumeration errors. This historical cause is not established as a regression here.

astral-sh/uv#18819 and astral-sh/uv#18861 concern explicit exclude-newer support for outdated listings. The latter merged April 7, 2026. They do not establish a cause for a report that denies this setting. astral-sh/uv#12052 became unreproducible without a confirmed fix. astral-sh/uv#13733 concerned project resolution and was resolved by upgrading another dependency alongside the target, rather than tool management.

PR keyword searches returned no results, including searches across all states and explicit open, closed, and merged filters. The related merged PRs were found through issue timeline references and inspected directly. The search result should not be read as proof that no other PR exists.

## Follow-up and limits

Collect the verbose single-tool upgrade output, the outdated listing with saved version specifiers and interpreter information, and the tool's saved requirements and options before a reinstall changes the evidence. Request only diagnostic material with credentials removed. The report supplies no verbose upgrade trace or receipt, so the cause and whether this is a regression of the historical mechanism remain unconfirmed.

This handoff is based on read-only GitHub inspection and the checkout's source, documentation, and existing tests. No reproduction, build, or test run was performed. No checkout files or GitHub state were changed.
