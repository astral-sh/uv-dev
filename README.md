# Add a command or flag to verify `uv.lock` is using the latest supported revision

Issue: astral-sh/uv#22912

Classification: duplicate

## Summary

astral-sh/uv#15220 is the closest existing discussion. Merged astral-sh/uv#15994 enables --check --refresh for non-writing validation; astral-sh/uv#22911 separately proposes preventing downgrades.

The reporter wants CI and pre-commit to reject a lockfile whose revision is older than the latest revision supported by the executing uv binary. They report a contributor changing revision 5 to revision 3 with an older uv, followed by:

```text
error: Frozen lockfile discovery requires a lockfile with revision 5 or later; run `uv lock` to update it
```

The suggested interfaces are a stricter `uv lock --check` or a dedicated revision-checking flag. The issue does not identify the older binary's exact version or provide a reproducer. The companion issue astral-sh/uv#22911 lists uv 0.13.5, but that does not establish which binary performed the downgrade.

## Draft response

You can use `uv lock --check --refresh` in CI with the uv version you want to enforce. It re-resolves and fails without rewriting `uv.lock` if the result differs, including its revision. This also refreshes dependency metadata and checks other lockfile changes, so it is broader than a revision-only check.

Let's consolidate this with astral-sh/uv#15220, where `--refresh` is the recommended approach. If you need a revision-only check without re-resolution, please describe that constraint there. You can also set `tool.uv.required-version` to reject older uv versions; preventing revision downgrades directly is discussed in astral-sh/uv#22911.

## Classification

astral-sh/uv#15220 already tracks enforcing the newest lockfile format in pre-commit when contributors use different uv versions. The new report emphasizes non-writing CI validation, but the existing --check --refresh combination provides that workflow through astral-sh/uv#15994. A dedicated revision-only interface can be discussed in the existing thread. Ordinary checks intentionally accept compatible older revisions; no regression in the refresh-based check is established. astral-sh/uv#22911 is related but requests a different write-prevention policy.

The duplicate recommendation is based on the shared requested capability and CI/pre-commit use case, not merely a shared error message. astral-sh/uv#15220 explicitly asks whether locking can check format and revision after different contributors use different uv versions. Although its proposed interface rewrites the file, the existing refresh operation can be combined with the non-writing check.

This does not establish that a dedicated revision-only flag exists. The existing command performs resolution and may access package indexes; it can fail for differences other than revision. A requirement to avoid resolution or network access would be an additional design constraint to discuss in the canonical thread.

The frozen-discovery error is an intentional requirement of discovery without the workspace manifest: revision 5 records metadata needed for that mode. It is not evidence that every use of `uv sync --frozen` requires revision 5. The older binary's reported downgrade was not reproduced, and no previously fixed regression in the refresh-based check was identified.

## Related

- Issue astral-sh/uv#15220 — Force newest lock file format (revision = 3, upload-time=...) (closed). Tracks the same mixed-uv-version problem: older developers' binaries change the lockfile revision, while a pinned pre-commit uv does not enforce the newest format. It explicitly asks to check the format and revision. Closed on 2026-05-27 with the maintainer recommendation to use --refresh. Its proposed remedy rewrites the file; the new report emphasizes non-writing CI validation.
- Pull request astral-sh/uv#15994 — Refresh lockfile when `--refresh` is provided (astral-sh/uv#15991) (merged). Merged on 2025-09-23. Makes --refresh force resolution and supports --refresh --locked, failing only when the resulting lockfile changes. Since --check is equivalent to --locked and lockfile equality includes revision, this supplies an existing non-writing check for revision changes, although it also refreshes metadata and checks other differences.
- Issue astral-sh/uv#22911 — Prevent older uv versions from downgrading the lockfile revision (open). Companion report describes the same revision 5-to-3 downgrade and exact frozen-discovery error. Its requested behavior differs: refuse writes by an older uv binary unless explicitly forced, rather than validate the revision with a newer binary in CI.
- Pull request astral-sh/uv#19579 — Add `--force` flag to `uv lock` for lockfile format migration (closed). Proposed a dedicated format-migration flag for astral-sh/uv#15220. Closed unmerged on 2026-05-27 after maintainers pointed to --refresh and found insufficient justification for another flag. Relevant design precedent, but its write-oriented flag explicitly conflicted with --check.

## Supporting evidence

- `docs/concepts/resolution.md:943` distinguishes schema `version` from backwards-compatible `revision`; older uv versions may read newer revisions. `docs/concepts/projects/sync.md:35` describes ordinary lockfile checks in terms of project metadata.
- `crates/uv-cli/src/lib.rs:3811` documents `uv lock --check` as equivalent to `--locked`. `crates/uv-project-commands/src/lock.rs:150` selects `LockMode::Locked` for checking.
- `crates/uv-lock-operations/src/validated_lock.rs:254` forces re-resolution for `--refresh`. `crates/uv-lock/src/lock/mod.rs:132` defines the current revision as 5; construction from a new resolution uses that revision at line 2641.
- `crates/uv-lock/src/lock/mod.rs:310` derives equality for `Lock`, including its `revision` field. `crates/uv-lock-operations/src/lock.rs:1030` compares the newly resolved lock to the previous one. The locked-mode branch at line 171 does not write the lockfile and returns a mismatch error if it changes. Together these establish the revision-checking behavior of `uv lock --check --refresh`.
- Existing `legacy_default_groups_in_lockfile` in `crates/uv/tests/project/lock.rs:929` deliberately accepts a compatible revision-3 lock with `--locked` and leaves it unchanged under ordinary locking. Existing `lock_refresh` in `crates/uv/tests/lock/lock.rs:43597` verifies that ordinary locking retains an older revision, refresh rewrites it to the current revision, and a subsequent `--refresh --locked` succeeds.
- `crates/uv-lock-operations/src/lockfile.rs:24` discovers a frozen workspace when its manifest is missing; line 68 contains the exact reported error. Revision 5 supplies default-group and dependency-group metadata, as documented in `crates/uv-lock/src/lock/mod.rs:124`.
- `crates/uv-settings/src/settings.rs:260` documents `required-version`: uv exits with an error when its version does not satisfy the configured requirement. Maintainers recommend this option in astral-sh/uv#15220 and astral-sh/uv#11319.
- In astral-sh/uv#15220, the final maintainer comment on 2026-05-27 recommends `--refresh`. The linked astral-sh/uv#15757 and later astral-sh/uv#19217 were closed unmerged; neither shipped a `--force` or `--rewrite` command. astral-sh/uv#15994 had already merged on 2025-09-23, before this report. Its description explicitly supports `--refresh --locked`, and review feedback calls for asserting that refresh changes the revision.
- The checkout's `lockfile-format-check` preview concerns canonical formatting. It is not required for comparing revision fields after refresh, so it is not proposed as the solution here.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separately searched revision validation (--check-revision, --check-latest-revision, lock/check/revision), downgrades (revision/downgrade and mixed-version format enforcement), and the exact 'Frozen lockfile discovery requires' error. Expanded to latest format, schema validation, strict checks, backward compatibility, pre-commit enforcement, and refresh-based fixes. Direct PR searches returned no matches; followed issue comments and timeline references to inspect astral-sh/uv#15757, astral-sh/uv#15994, astral-sh/uv#19217, and astral-sh/uv#19579, including maintainer feedback. Also inspected astral-sh/uv#13738, astral-sh/uv#11319, and astral-sh/uv#15997. Ruled out astral-sh/uv#16839: its false-positive dry-run changes concern marker comparison, not revision enforcement. astral-sh/uv#11319 concerns recording the generating uv version, not validating the lockfile revision. Checked current source, documentation, and existing tests; found no evidence of a regression in --check --refresh.

The claims were investigated independently: detecting stale revisions, preventing an older writer from downgrading a newer revision, and the frozen-discovery failure. Literal identifiers and errors were followed by conceptual searches without revision numbers or platform details. Historical PRs and their comments were followed through references rather than assuming that a matching title established an implementation.

astral-sh/uv#13738 provides historical context about revision changes and upload timestamps; it resolved a version mismatch rather than requesting a CI check. astral-sh/uv#11319 recommends `required-version` but concerns recording the generating uv binary version, so it is not the duplicate target. astral-sh/uv#16839 was inspected and excluded because its false-positive change detection concerns marker representation. astral-sh/uv#15997 explains the lock comparison work associated with the merged refresh change.

## Verification and limits

This handoff is based on authenticated read-only GitHub queries, repository documentation, source inspection, and existing test definitions. No tests or builds were run, and the reporter's environment was not reproduced. No checkout files or GitHub objects were changed. Only this issue-context README was updated.
