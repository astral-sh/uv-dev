# `--locked`/`--check` accept a lockfile whose resolution violates the declared specifiers (declaration-only validation)

Issue: astral-sh/uv#23073

Classification: duplicate

## Summary

The reporter describes a lockfile whose recorded root requirements match `pyproject.toml` while its resolved package version violates those requirements. Their public-PyPI example first locks `six==1.16.0`, then changes both the project declaration and the lockfile's `requires-dist` declaration to `six==1.17.0`, leaving the resolved package, artifacts, and hashes at 1.16.0. They report that `uv lock --locked`, `uv lock --check`, `uv sync --locked`, and an already synchronized environment's `uv sync --check` accept this state. Changing only the project declaration is the control case and correctly invalidates the lockfile. The updated issue body says the production inconsistency was observed after Git merges: the recorded declaration required `kifv-model-serving==2.3.0rc2`, while the resolution remained at 2.2.7. This is a reporter-provided account of how the problem arose; the supplied minimal reproduction still uses explicit lockfile edits and does not demonstrate the merge sequence.

The report also requests rejection of inconsistent direct dependency edges without fetching metadata, suggests parser validation as an alternative, and notes a related transitive-dependency concern. The body names uv 0.12.3 on Windows and CPython 3.13.15; the issue's version field names uv 0.13.7 dated November 3, 2026. The supplied reproduction uses ordinary registry packages and is not inherently Windows-specific. These runtime outcomes are reporter-provided; no reproduction was executed during this triage.

astral-sh/uv#12276 is a direct duplicate. astral-sh/uv#15445 and astral-sh/uv#18788 cover related merge-induced inconsistencies; astral-sh/uv#21484 requests broader integrity verification. Merged astral-sh/uv#12235 validates wheel versions, a different invariant. An open proposed fix is now available in astral-sh/uv#23109; it has not been merged or verified by this investigation.

## Draft response

Your reproduction matches astral-sh/uv#12276, which already tracks locked versions violating the recorded dependency specifiers. That discussion also explicitly covers exact pins passing `uv lock --check` and `uv sync --locked`. Please add this reproduction there so the validation work can stay in one place.

The validation added in astral-sh/uv#12235 checks that package and wheel versions agree; it does not check dependency specifiers. One clarification: `uv lock --check` is equivalent to `uv lock --locked`, while `uv sync --check` checks whether the environment is synchronized.

## Classification

Open astral-sh/uv#12276 already tracks the same inconsistent declaration/resolution acceptance and requests the same static version-specifier validation, including exact pins and the same CI commands. The more specific six reproduction does not establish a separate problem. No inspected fix previously implemented this check; astral-sh/uv#12235 addresses wheel-version coherence, so the report does not establish a regression.

The incorrect-version acceptance is a correctness concern. Duplicate takes precedence because the same behavior and requested validation already have an open discussion. The enhancement label on astral-sh/uv#12276 and maintainer comments about unsupported manual editing do not change the symptom match.

The issue was opened November 3, 2026 and had a bug label at initial triage. The reporter's follow-up describes astral-sh/uv#12276 and three other discussions as related but not identical, without identifying an additional behavioral distinction from astral-sh/uv#12276. Review of the linked discussions does not change the duplicate classification. The canonical report dates to March 18, 2025, reproduces on uv 0.6.7, and remains open. Its March 2026 comment explicitly extends the original exclusion-constraint example to exact pins and the same locked/check commands. The new versions and reproduction do not establish that an intervening fix regressed.

## Related

- astral-sh/uv#23109 — pull request, open: Validate that locked dependency versions satisfy declared specifiers in `Lock::satisfies`. Opened by myhubstat on November 4, 2026 specifically to address astral-sh/uv#23073. The author proposes checking resolved edges against effective PEP 440 requirements, including overrides, exclusions, and markers, and adds root-requirement validation and tests. This is proposed implementation work, not an established fix or the reason for the duplicate classification.
- astral-sh/uv#12276 — issue, open: Validate locked versions against constraints in lock file. Same underlying problem and requested validation: matching pyproject.toml and requires-dist declarations permit an incompatible locked version after manual merge resolution. The reproduction installs numpy==2.2.4 despite numpy!=2.2.4; a later comment explicitly covers exact pins passing uv lock --check and uv sync --locked. This is the closest canonical discussion.
- astral-sh/uv#15445 — issue, closed: uv lock does not re-evaluate constraints. A merged lockfile records numpy==1.26.3 in both project constraints and the lockfile manifest but resolves 1.26.4; uv lock --check accepts it. This closely matches declaration-only validation, though the declaration is constraint-dependencies rather than a direct requires-dist pin. Maintainers consolidated it into astral-sh/uv#18788 without identifying a fix.
- astral-sh/uv#18788 — issue, open: `uv lock --locked` does not detect transitive dependencies that are no longer depended on by anything. Related merge-induced lockfile inconsistency accepted by uv lock --locked. Its own reproduction concerns unreachable leftover packages rather than a version violating a specifier, but maintainers centralized the incompatible-constraint report astral-sh/uv#15445 and the transitive-version report astral-sh/uv#15378 here.
- astral-sh/uv#21484 — issue, open: Command to verify uv.lock is legitimate. Broader integrity-verification request explicitly includes checking locked versions against pyproject.toml ranges. Comments describe altered URLs and injected dependency edges passing checks, discuss the same immutable-registry validation path, and point to astral-sh/uv#12276. It additionally seeks artifact provenance verification, beyond the proposed static specifier check.
- astral-sh/uv#12235 — pull request, merged: Error on lockfiles with incoherent wheel versions. Historical parser validation cited in astral-sh/uv#12276, merged March 17, 2025 and released in uv 0.6.7. It rejects disagreement between a package version and wheel filename versions; it does not compare resolved versions with dependency declarations. The new reproduction keeps package and wheel versions consistent, so this fix does not establish a regression.

- astral-sh/uv#18781 — issue, closed: Reject locked malware installations. Maintainers explicitly favor installing from lockfiles without querying the package index and addressed known malicious releases through OSV malware checks; a maintainer marked it done with astral-sh/uv#18936 on May 21, 2026. This supplies relevant design context for avoiding index refreshes, but malware-advisory checking does not validate a locked version against project specifiers.

## Proposed fix and validation status

The new issue comment identifies astral-sh/uv#23109 as a concrete implementation attempt. GitHub metadata confirms that it is open, is not a draft, and explicitly targets astral-sh/uv#23073. At inspection on November 4, 2026, it had no comments or submitted reviews; CI checks were queued or skipped, with no completed successful validation shown. No merge or released fix is established.

The PR description proposes initializing dependency modifiers unconditionally and invoking a new `Lock::satisfies_resolved_dependency_versions` check for metadata-bearing packages. It describes checking production, optional, and dependency-group edges across the applicable marker domain, plus root requirements for PEP 723 scripts and projectless workspaces. The changed-file list confirms edits to `crates/uv-lock/src/lock/mod.rs` and `crates/uv/tests/project/lock.rs`; the author reports unit and integration tests for `uv lock --check` and `uv lock --locked`. These implementation and coverage details come from the PR description and file metadata; this update did not review the patch or execute its tests.

The duplicate classification remains based on the pre-existing discussion in astral-sh/uv#12276. A PR created in response to this issue does not independently establish duplicate status. Reviewers now have a concrete proposal to assess for effective specifiers, marker coverage, overrides, exclusions, and the metadata-bearing validation path.

## Supporting evidence

### Existing discussions and release history

In astral-sh/uv#12276, the original project and recorded requirement both exclude numpy 2.2.4, but `uv lock --check` succeeds and `uv sync` installs that version. The reporter attributes the invalid lockfile to merge-conflict resolution and requests a static check of versions against recorded constraints. A maintainer states that external edits are not guaranteed to stay synchronized; a contributor welcomes additional validation. The later exact-pin comment independently matches the new report's principal symptoms. Its suggested refresh workaround is a commenter recommendation, not a reproduced result here.

In astral-sh/uv#15445, the project and lockfile manifest both constrain numpy to 1.26.3 while the resolution uses 1.26.4. Comments explain that merging changes from separate branches produced the inconsistency. A maintainer closed it into astral-sh/uv#18788 on April 6, 2026. The same destination was used for astral-sh/uv#15378, which reports a transitive packaging version outside langfuse's requirements. These closures identify a broader active discussion, not a fixing release.

The historical astral-sh/uv#12164 and astral-sh/uv#12254 concern package versions edited without corresponding wheel updates. Merged astral-sh/uv#12235 rejects that mismatch. The release discussion identifies uv 0.6.7 as containing the validation. It does not cover an internally coherent package/artifact record that contradicts its parent's declaration.

In astral-sh/uv#21484, a maintainer suggests `uv lock --check --refresh --no-build` for broader integrity checking, and the reporter says it meets that use case. This does not establish that ordinary locked checks validate edited resolutions. The related astral-sh/uv#18553 concerns the opposite failure: a refreshed check rejecting an unchanged valid lockfile with workspace conflicts. It is closed and does not establish a fix or regression for this report.

The reporter also points to astral-sh/uv#18781. Its maintainer discussion explains that locked installs intentionally avoid index lookups, including quarantine status checks; OSV malware queries address known malicious releases separately. This supports the relevance of an offline validation proposal, but does not establish that any particular implementation is sufficient or approved. Its completed malware work is not a fix for declaration/resolution inconsistencies.

### Current checkout

- `crates/uv-lock/src/lock/mod.rs:3970`: `satisfies_requires_dist` normalizes and compares refreshed requirements with `package.metadata.requires_dist`; mismatches return `MismatchedPackageRequirements`.
- `crates/uv-lock/src/lock/mod.rs:4687`: the validation walk skips immutable packages and their dependencies. `Source::is_immutable` at line 8051 includes registry and Git sources.
- `crates/uv-lock/src/lock/mod.rs:7534`: the parser checks package/wheel version consistency and can return `InconsistentVersions`, which is a separate invariant.
- The current code also contains version-specifier checks, including `package_satisfies_requirement` at line 5054 and `package_requirement_marker` at line 1736. Therefore, the report's assertion that no such comparison exists anywhere is too broad. Additional dependency validation in `satisfies_requires_dist` is gated by `allow_missing_package_metadata`; `crates/uv-lock-operations/src/validated_lock.rs:305` supplies that flag from the `LockWithoutMetadata` preview feature. Its existence does not establish coverage of the reported ordinary metadata-bearing path.
- `crates/uv-lock-operations/src/validated_lock.rs:254`: explicit refresh forces re-resolution instead of accepting the existing lockfile through the ordinary reuse path.
- `crates/uv-cli/src/lib.rs:3816` documents equivalence of `uv lock --check` and `uv lock --locked`. The sync arguments separately define locked mode at line 3679 and environment checking at line 3806. The reporter's claim that the sync flags are aliases is incorrect.
- `docs/concepts/projects/sync.md:35` describes project/lockfile freshness checks. The source supports the distinction between matching recorded project metadata and comprehensive verification of an externally edited resolution.

These findings support the known validation gap and duplicate relationship; they do not prove that the immutable-source shortcut alone is the complete root cause or that a single unqualified `contains()` check handles markers, forks, overrides, and all dependency contexts.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Separated acceptance of incompatible direct pins, acceptance of edited transitive resolutions, and the requested static validation. Literal searches included locked/specifier, sync --check/specifier, hand-edited lockfiles, does not satisfy, the needs-to-be-updated error fragment, MismatchedRequirements, InconsistentVersions, and Lock::satisfies. Conceptual and fix-oriented searches covered lock validation, constraints, inconsistent/merge-edited lockfiles, requires-dist versions, registry immutability, and integrity verification, without package or platform restrictions. Inspected candidate bodies and comments and followed astral-sh/uv#12276 to astral-sh/uv#12254, astral-sh/uv#12164, and astral-sh/uv#12235; followed astral-sh/uv#15445 and astral-sh/uv#15378 to astral-sh/uv#18788. Ruled out astral-sh/uv#12164 and astral-sh/uv#12254 as the same regression because they concern package/wheel version disagreement. Also inspected astral-sh/uv#11932, which primarily requests environment/hash checking, and astral-sh/uv#18553, a refresh-check false positive with conflicts. Read current validation and CLI source to distinguish confirmed behavior from the reporter's proposed mechanism.

Searches used both GitHub REST search and the gh issue/PR search and listing commands, with all-state and explicit open, closed, and merged PR queries. At initial triage, PR searches returned no additional relevant candidates; linked historical pull requests were inspected directly, and a recent all-state REST pull-request listing was also checked. This search does not establish that no other related work exists.

astral-sh/uv#11932 was inspected because it mentions the same four commands and edited lockfiles. Its primary request is to compare the installed environment and hashes with the lockfile; its maintainer discussion explicitly distinguishes the lock namespace from environment checking and points to the wheel-version fix. It is less specific than astral-sh/uv#12276 for the new requirement-versus-resolution mismatch.

The follow-up's other links were inspected and do not identify a closer duplicate or a prior fix:

- astral-sh/uv#14630 — closed: support to ensure lockfile dev release matches pyproject.toml's. Although the opening report claimed an incompatible development release was retained, the discussion established that the example had changed the consuming project's own version while leaving its dependency bound unchanged. After correcting that bound, the reporter confirmed the expected upgrade and closed the issue because they could not reproduce the original claim. This is not evidence of the declaration/resolution mismatch reported here, or of a fix that later regressed.
- astral-sh/uv#18775 — open: Alternatives for handling issues with `uv lock [--check]` and `exclude-newer`. The reported failure is rejection of a lockfile because global cooldown configuration differs between the locking workstation and the container. A follow-up says the behavior persisted on uv 0.11.15 after the related timestamp change. It requests control over configuration validation; it does not demonstrate acceptance of an incompatible resolved version.

## Handoff scope

Only this temporary issue-context README was authored. No checkout files were changed and no GitHub comments, labels, issues, or pull requests were modified. No builds or tests were run.
