# tree --outdated should indicate when a newer version exists but is excluded by exclude-newer

Issue: astral-sh/uv#23062

Classification: enhancement

## Summary

The reporter uses `exclude-newer = "7 days"` and wants `uv tree --outdated` to reveal when a newer release is still inside that cooldown. Their example reports Ruff 0.17.2 as latest, while repeating the command with `--exclude-newer false` reports 0.17.3. They want to decide whether to upgrade immediately or wait, without comparing two command outputs.

The requested additions are an indication that newer releases were excluded, the newest excluded version, and when that version clears a relative cooldown. The illustrated timestamp is a proposed display, not an independently verified release date. No uv version is supplied.

No duplicate found among the inspected items. astral-sh/uv#18899 covers relative-cutoff correctness; astral-sh/uv#19864 discusses exclusion warnings and bypasses; astral-sh/uv#18220 and astral-sh/uv#18952 cover dated resolver hints; astral-sh/uv#18819 and astral-sh/uv#18861 establish cooldown-aware tool reporting.

## Draft response

Your example matches the current behavior: `uv tree --outdated` filters releases through `exclude-newer` before choosing the version shown as `latest`, and does not retain excluded-release information for display. The relative-cutoff fix in astral-sh/uv#18899 addresses a different problem.

Showing the newest excluded release and when it clears a relative cooldown would be an enhancement. The warning discussion in astral-sh/uv#19864 is related, but this tree-specific request can remain separate. The next step is to settle the annotation and its behavior for fixed cutoffs, which do not expire automatically. Any date should describe cooldown eligibility rather than guarantee that the full dependency graph can resolve.

## Classification

The request adds visibility into releases intentionally excluded from the latest eligible-version lookup, plus cooldown timing information. Source and existing tests confirm cutoff-aware selection; the report does not establish an incorrect eligible version, stale cutoff, or regression. The related discussions cover different commands, resolver failures, or bypass policies, and none inspected tracks this tree annotation closely enough to centralize the request.

The distinction is between selecting the latest version under the configured policy and adding information about releases outside that policy. The supplied example does not show a wrong selection under the policy. There is no evidence that excluded-release annotations previously existed and were lost.

## Related

- astral-sh/uv#18899 — Recompute relative `exclude-newer` values during `uv tree --outdated` (pull request, merged). Fixed the same command using a stale cutoff from the lockfile by evaluating relative cooldowns against the current time. Merged April 7, 2026; it does not expose releases that remain inside the cooldown, so this report does not establish a regression of that fix.
- astral-sh/uv#19864 — Structured bypasses for exclude-newer (issue, open). A commenter explicitly requests a warning when exclude-newer ignores a newer version. The main discussion concerns bypass policies for explicit upgrades and dependency constraints, rather than displaying excluded versions and eligibility dates in tree output.
- astral-sh/uv#18952 — Improve `--exclude-newer` hints (pull request, merged). Merged April 15, 2026, adding resolver-failure hints with the latest excluded version satisfying a requirement and its publication date. This provides related diagnostic precedent, but does not annotate successful tree --outdated output or calculate cooldown expiry.
- astral-sh/uv#18220 — Include earliest valid date in `exclude-newer` resolver hint (issue, closed). Requested upload-date information to help users adjust a cutoff that prevents resolution; closed by astral-sh/uv#18952. It shares the request for actionable timing information, but addresses unsatisfiable resolution rather than planning upgrades from a dependency tree.
- astral-sh/uv#18819 — Should `uv tool list --outdated` also support the flag `--exclude-newer` (issue, closed). Requested that outdated-tool reporting honor cooldowns, which a maintainer supported and astral-sh/uv#18861 implemented. It concerns filtering unavailable upgrades in a different command, rather than additionally revealing filtered releases.
- astral-sh/uv#18861 — Respect `--exclude-newer` in `uv tool list --outdated` (pull request, merged). Implemented astral-sh/uv#18819 on April 7, 2026. It establishes intentional cutoff-aware outdated reporting for tools, but adds neither excluded-version annotations nor eligibility dates.

## Supporting evidence

- `crates/uv-resolve-operations/src/latest.rs:40`: `LatestClient::consider_candidate` rejects files whose upload timestamp is at or after the effective cutoff. It separately filters prereleases, yanked files, Python incompatibility, and applicable wheel incompatibility. At line 106, `find_latest` returns only one distribution filename, without excluded-release metadata.
- `crates/uv-project-commands/src/tree.rs:326`: the latest-version client receives the lock's exclusion policy. At line 352, the result becomes a version; at line 363, only versions greater than the locked version enter the display map. The current path also cannot signal a hidden newer release when the locked version already equals the latest eligible version.
- `crates/uv-lock/src/lock/tree.rs:615`: the text renderer receives a version map and prints only `(latest: v…)`; it has no excluded version or release-time field.
- `crates/uv-distribution-types/src/exclude_newer.rs:65`: exclusion values distinguish absolute timestamps from relative spans. The `timestamp` method at line 81 computes relative cutoffs against the current time. A fixed cutoff does not become less restrictive as time passes.
- `crates/uv/tests/project/tree.rs:1735` and `crates/uv/tests/project/tree.rs:1790`: existing `outdated` and `outdated_exclude_newer_relative` tests cover latest-version output and advancing relative cutoffs. Both use the `test-universal` feature gate. Neither exercises reporting a release that is still excluded.
- The inspected diff for astral-sh/uv#18899 changes cutoff recomputation and adds the relative-cutoff test. It does not add excluded-release metadata. The body and comments on astral-sh/uv#18952 establish that its dated hints apply to resolver failures.

These findings come from source, tests, and GitHub discussion inspection. The reporter's package/version example was not executed; no builds or tests were run for this read-only triage.

## Search scope and exclusions

Decomposed the report into missing excluded-release visibility and requested version/eligibility-date annotations. Used authenticated gh to search open and closed issues and open, closed, and merged PRs, combining tree/outdated/exclude-newer with cooldown, excluded, warning, ignored, installable, eligible, display, notification, latest, and date/hint; searches omitted incidental package names and versions. Followed reporter links, comments, timelines, and fixing-PR chains, and inspected current selection/rendering code and existing tests. PR keyword searches returned no matches despite known linked PRs, so PR discovery relied on those chains and is not exhaustive; several final expanded searches were rate-limited. Ruled out astral-sh/uv#16485 (debug logging for failed locking) and astral-sh/uv#22944 (audit policy for quarantined security fixes) as duplicates. The stale-cutoff discussion in astral-sh/uv#18708 leads to astral-sh/uv#18899, a different fixed behavior.

Additional inspected context: astral-sh/uv#18921 proposes bypassing cooldowns for direct pinned dependencies and is linked from astral-sh/uv#19864; it does not implement tree reporting. The warning suggestion in astral-sh/uv#19864 is a user's proposal, not a maintainer commitment to add that warning.

## Maintainer next step

Decide whether tree output should show a separate excluded-version annotation and how to label eligibility for relative versus fixed cutoffs. Any implementation should identify releases excluded specifically by the cooldown while respecting the other latest-version filters, and avoid presenting cooldown expiry as a guarantee of dependency resolution. Existing coverage can guide focused regression cases for both an available eligible upgrade and an already-current eligible version with a newer excluded release.
