# Close gap between exclude-newer and audits ignore-until-fixed

Issue: astral-sh/uv#22944

Classification: duplicate

## Summary

astral-sh/uv#18506 contains the exact cooldown-aware ignore request. astral-sh/uv#18995 covers automatic security exemptions, while astral-sh/uv#19864 and astral-sh/uv#18921 track explicit bypasses. astral-sh/uv#18737 establishes the current ignore semantics.

The reporter runs `uv audit` as a failing CI gate and uses `ignore-until-fixed` for selected advisories. When an advisory gains a fix version that is still inside the configured `exclude-newer` cooldown, auditing reports the vulnerability while resolution excludes the fix. The report proposes three alternatives: keep the ignore until the fix becomes installable, condition that behavior on CVSS severity, or automatically exempt security fixes from the cooldown.

The report supplies a proposed workflow rather than a concrete package/advisory reproduction or uv version. No reproduction is needed to establish the relationship: the current filtering code and an earlier discussion describe this interaction directly.

## Draft response

Your CI scenario is already described in the May 6 discussion on astral-sh/uv#18506, including an ignore that lasts until a fix satisfies exclude-newer. Please continue the cooldown-aware ignore proposal there.

Currently, --ignore-until-fixed checks whether the advisory lists a fix version; it does not check whether your resolver settings permit installing it. Automatic security-fix exemptions were discussed in astral-sh/uv#18995, where maintainers favored explicit, reviewed overrides. More precise bypasses are tracked in astral-sh/uv#19864. The roadmap also discusses CVSS filtering and concerns about missing or misleading scores.

If you decide to install a fix before its cooldown expires, --exclude-newer-package <package>=false provides a manual exception for that package. Automatic cooldown-aware ignores are not currently implemented.

## Classification

The primary request and CI trigger are already discussed explicitly in the May 6 comments on astral-sh/uv#18506. The automatic-exemption alternative is also covered by astral-sh/uv#18995, with more precise manual bypasses tracked in astral-sh/uv#19864. Discussion can be centralized in those existing threads. Current code checks for known fix versions without checking installability, consistent with the original feature; no previously fixed behavior is shown to have regressed.

The strongest match is the specific cooldown-aware ignore discussion within the roadmap, not merely the roadmap's broad audit scope. The May 6 exchange requests an `ignore-until-non-excluded-fix` variant for individually accepted vulnerabilities and describes choosing between a manual package exception and a temporary advisory ignore in CI. This is sufficient to centralize the primary proposal there. The other two options have existing policy discussions as described below.

Absent the overlapping discussions, this would be an enhancement to audit/resolver policy coordination. A published fix does not imply that the project has installed it or that its constraints permit installation. The current feature suppresses selected findings only while the advisory has no known fix versions; it does not promise installability checks.

## Related

- astral-sh/uv#18506 — Roadmap: `uv audit` (issue, open). The May 6 discussion explicitly requests keeping an individual ignore active until a fix satisfies exclude-newer, describing the same CI/pre-commit workflow. This is the closest existing discussion for the primary request. The thread also covers CVSS thresholds; maintainers cite missing or misleading scores and favor exposing scores over built-in filtering.
- astral-sh/uv#18995 — Feature request: advisory-aware exclude-newer that exempts CVE fixes from the cooldown window (issue, closed). Reports the same audit-versus-cooldown CI conflict and proposes the automatic security-fix exemption in option 3. Maintainers favored explicit, reviewed package overrides and more precise exemptions; closure does not indicate that advisory-based bypasses were implemented.
- astral-sh/uv#19864 — Structured bypasses for exclude-newer (issue, open). Tracks more precise cooldown exceptions, with concrete security-fix examples and concerns about persistent package-wide bypasses. It is the current discussion for explicit constraint/version-based exceptions, rather than advisory-aware audit suppression or automatic CVE exemptions.
- astral-sh/uv#18737 — uv audit: `--ignore` and `--ignore-until-fixed` (pull request, merged). Merged March 30, 2026, introducing per-advisory ignores until a known fix version appears. Its stated semantics and the current implementation do not test whether that fix satisfies resolver constraints or cooldowns; this establishes feature history, not a previous fix for the reported interaction.
- astral-sh/uv#18921 — Add `allow-bypass` option to `exclude-newer` for allowing direct pinned dependencies (pull request, open). Referenced by astral-sh/uv#19864 as an implementation direction for explicit pinned-dependency cooldown bypasses. Its body says it is not ready for review. It does not implement cooldown-aware ignore-until-fixed or automatic security-advisory exemptions.

## Supporting evidence

- In astral-sh/uv#18506, eitan-protego's May 6, 2026 comments request that `ignore-until-fixed` account for `exclude-newer`, then explain the same pre-commit/CI workflow. Woodruffw's response asks about the intended flow and says users currently make explicit decisions reconciling cooldowns with audit findings. This is a design discussion, not an implemented feature.
- In the same roadmap, Woodruffw's April 14, June 23, and September 10 comments explain concerns about missing or misleading CVSS scores. The September response favors exposing scores for users to filter themselves. This is relevant to the proposed severity cutoff, but is not a commitment to implement such a cutoff.
- In astral-sh/uv#18995, the April 15 maintainer comments favor deliberate, reviewed cooldown bypasses and suggest `exclude-newer-package` or more precise version exceptions. The issue closed that day without establishing an automatic exemption implementation.
- astral-sh/uv#18914 was closed on June 15 with an explicit maintainer redirect to astral-sh/uv#19864. That newer issue includes examples of fresh security fixes, direct and transitive dependency concerns, and a reference to the still-open astral-sh/uv#18921.
- `crates/uv-audit/src/service/osv.rs:460` extracts fix versions from OSV fixed events for the matching PyPI package. This extraction does not evaluate artifact upload times or resolver eligibility.
- `crates/uv-audit-operations/src/lib.rs:112` suppresses a matching `ignore_until_fixed` finding only when `vulnerability.fix_versions.is_empty()`. A nonempty list leaves the vulnerability in the findings.
- `crates/uv-settings/src/settings.rs:3046` and `crates/uv-cli/src/lib.rs:5025` describe ignores lasting while there are no known fix versions. This agrees with the feature introduced by astral-sh/uv#18737 on March 30.
- Existing integration tests in `crates/uv/tests/build/audit.rs:1135` and `crates/uv/tests/build/audit.rs:1190` cover suppression with no fix and exit code 1 when a fix is listed. The second test even pins the dependency to the vulnerable version; it does not require a satisfiable upgrade before reporting the vulnerability. These tests were inspected, not executed.
- `docs/concepts/resolution.md:854` documents `--exclude-newer-package <package>=false` as a package-specific opt-out. The precedence rule at line 865 says package-specific settings override global and index-specific cutoffs. This is a manual exception and does not implement waiting until a fix becomes eligible.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separately searched ignore-until-fixed with exclude-newer, the proposed flag names, and the existing term ignore-until-non-excluded-fix; broadened to audit/cooldown, installable or available fixes, security-fix bypasses, quarantine, CVSS, severity thresholds, and structured exceptions. Inspected area:uv-audit issues, candidate comments, and referenced PRs; followed astral-sh/uv#18914 to its replacement astral-sh/uv#19864. PR keyword searches returned no hits, so linked PRs were inspected directly, including reviews. Ruled out astral-sh/uv#20983 (blanket ignoring of unfixed advisories), astral-sh/uv#19265 (removing stale ignore configuration), and astral-sh/uv#19428 (automatic remediation) as duplicates of cooldown-aware suppression. Checked source, existing tests, and the merged ignore-feature PR; found no prior cooldown-aware fix or regression evidence.

The report includes no exact runtime error, package version, or platform to anchor a regression search. Searches therefore focused on observable policy interactions separately from proposed implementation mechanisms. No candidate established a previously implemented cooldown-aware ignore behavior that later regressed.

## Handoff status

Only this temporary handoff was updated. No GitHub comment, label, issue, or pull request was changed, and no checkout files were modified. Source inspection and existing test inspection support the findings; no builds or test runs were necessary for this triage.
