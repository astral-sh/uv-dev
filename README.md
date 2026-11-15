# `uv upgrade` is not aware of other dependency groups

Issue: astral-sh/uv#23248

Classification: duplicate

## Summary

The reporter uses uv 0.13.7 on Ubuntu 24.04 with Python 3.14.7 and declares `ipython` under `[dependency-groups].dev`. They want to update its locked version without manually editing the dependency declaration.

Two commands fail:

- `uv upgrade ipython` reports `Dependency ipython was not found in project.dependencies` (the actual diagnostic quotes the identifiers with backticks).
- `uv upgrade --dev ipython` reports `unexpected argument '--dev' found`.

Open astral-sh/uv#20783 reproduces the same package-selection error for a dependency in a named group. Its linked implementation, astral-sh/uv#20590, is still open. The current implementation selects production dependencies only and has no `--dev` option. Separately, `uv upgrade` edits manifest constraints without updating the lockfile; the supported command for the reporter's stated lockfile goal is `uv lock --upgrade-package ipython`.

## Draft response

Group support for `uv upgrade` is tracked in astral-sh/uv#20783, with an open implementation in astral-sh/uv#20590. The current command only selects `project.dependencies` and edits constraints in `pyproject.toml`; it does not update `uv.lock`.

For the lockfile update you described, use `uv lock --upgrade-package ipython`. This works for dependencies in `dev` without `--dev`. To upgrade the whole group, use `uv lock --upgrade-group dev`. Both respect existing version constraints, so an exact pin still needs to be changed to permit a newer version.

## Classification

Open astral-sh/uv#20783 already tracks the same command rejecting dependencies outside project.dependencies with the same error; the dev-group reproduction fits that discussion. Its linked implementation, astral-sh/uv#20590, predates this report and is still open with no merge date. Current source and tests confirm the restriction, so the 'Fixed by' comment does not establish a released fix or regression. Duplicate classification takes precedence over independently assessing the missing capability.

The matching issue was opened on July 28, 2026, before this November 15 report. The implementation PR was opened on July 21 and has no merge date. A maintainer's July 28 comment on astral-sh/uv#20783 says “Fixed by” astral-sh/uv#20590, but the PR is still open and the checkout retains the rejection. Do not describe the fix as released or recommend a uv version as containing it.

No platform-specific failure or Python-version mechanism is established by this report. The source confirms the selection restriction without requiring a new reproduction.

## Related

- astral-sh/uv#20783 — `uv upgrade` should allow upgrading dependencies in extras or groups (issue, open). Exact match: a package declared only in a dependency group fails with the same 'was not found in project.dependencies' error. Tracks group and extras support for the same command; the dev group is a specific instance.
- astral-sh/uv#20590 — Allow `uv upgrade` to update optional and group dependencies (pull request, open). Linked from astral-sh/uv#20783 by a maintainer. Extends selection and requirement rewriting to optional dependencies and dependency groups, including include-group. It predates this report and remains unmerged, despite the issue comment saying 'Fixed by'.
- astral-sh/uv#18266 — Add support for `--upgrade-group` (pull request, merged). Provides the existing alternative `uv lock --upgrade-group dev`, released in 0.11.4 before the reported 0.13.7. Upgrades direct group members in the lockfile within declared constraints; it does not add group support or constraint rewriting to `uv upgrade`.

## Supporting evidence

- `crates/uv-project-commands/src/upgrade.rs:614`, `select_requirements`: reads `project.dependencies`, marks selected declarations as production dependencies, and emits the reported not-found diagnostic at line 694 for requested names absent from that list.
- `crates/uv-cli/src/lib.rs:3899`, `UpgradeArgs`: accepts package names, exclusions, index/client settings, and build-hash options; declares no `--dev` or group selector.
- `crates/uv/tests/it/upgrade.rs:634`, `upgrade_requires_production_dependency`: explicitly checks rejection of both an optional dependency and a dependency declared in the dev group with the same not-found diagnostic. The reported group behavior is already covered by an existing test.
- `crates/uv/tests/it/upgrade.rs:464`, `upgrade_updates_requirement_without_updating_lockfile_or_environment`: checks that manifest constraints change while the lockfile and environment remain unchanged. Merged astral-sh/uv#19738 also describes that deliberate command scope.
- `docs/concepts/projects/sync.md:157`: documents `uv lock --upgrade-package <package>` and states that upgrades remain within project dependency constraints. No dev-group selector is required for a named locked package.
- `changelogs/0.11.x.md:212`: uv 0.11.4, released April 7, 2026, includes the `--upgrade-group` support from astral-sh/uv#18266. That PR describes upgrading direct group members rather than all their transitive dependencies.
- `CHANGELOG.md:40`: the reported uv 0.13.7 was released November 3, 2026. The inspected group-support implementation remains open; its earlier creation date does not imply inclusion in this release.
- A maintainer closed astral-sh/uv#20782 into astral-sh/uv#20783 after identifying that a no-argument upgrade failed because the project had only dependency groups. This reinforces astral-sh/uv#20783 as the canonical discussion.

The lockfile commands respect all declared constraints. They cannot upgrade past an exact pin or restrictive upper bound without changing that declaration. Group lock upgrades are adjacent existing functionality, not evidence that `uv upgrade` group support previously shipped.

## Search coverage and other candidates

Searched astral-sh/uv with authenticated gh across open and closed issues and open, closed, and merged PRs. Decomposed the report into group-only package rejection, unsupported --dev, and lockfile updates versus manifest constraint changes. Literal searches included uv upgrade, dependency-groups, project.dependencies, the exact not-found error, and unexpected argument --dev; conceptual searches used extras, groups, production dependencies, upgrade-package, and upgrade-group without ipython, platform, or version restrictions. Followed maintainer references to astral-sh/uv#20590 and inspected its comments and merge status; inspected merged astral-sh/uv#19738 and astral-sh/uv#18266 for historical scope and available functionality. General PR searches returned no hits; referenced PRs were inspected directly. Inspected astral-sh/uv#20782, which was centralized into astral-sh/uv#20783. Ruled out astral-sh/uv#20574 as the canonical match because it concerns indirect conflict constraints, and astral-sh/uv#19160 because it requests main-dependency selection for lock upgrades. Also inspected broader astral-sh/uv#6794 and the implemented lock-group request astral-sh/uv#13705. One additional lockfile/dev search was rate-limited. Checked current source, existing tests, documentation, and release notes.

astral-sh/uv#6794 is the broader request for manifest upgrades; astral-sh/uv#20783 isolates this exact group-selection failure. Closed astral-sh/uv#13705 concerns upgrading only one group's locked packages and is addressed by astral-sh/uv#18266. Open astral-sh/uv#19160 concerns selecting main dependencies through lock-group upgrade semantics, the opposite selection gap in a different command. Open astral-sh/uv#20574 concerns assigning resolved versions when conflicts indirectly constrain a selected dependency; the reporter does not describe conflicting groups or a resolution failure.

## Verification and handoff

Findings are based on read-only GitHub metadata and source, test, documentation, and changelog inspection. No builds or package commands were run. Only this temporary handoff README was updated; no checkout files or GitHub state were changed. The proposed response is for maintainer review.
