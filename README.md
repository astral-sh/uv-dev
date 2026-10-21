# uv 0.13.5 listed in CHANGELOG but release appears to have been canceled

Issue: astral-sh/uv#22793

Classification: bug

## Summary

The reporter observes that the changelog at commit
`79422c835f214875852bf2b2cb950c44684eb046` lists uv 0.13.5, while GitHub/download
availability appears to stop at 0.13.4. They point to a canceled release workflow,
ask whether cancellation was intentional, and explain that automation deriving a
version from repository metadata can select an unavailable download. No uv command,
Python version, or platform-specific failure is involved.

Inspection on 2026-10-21 confirms the publication mismatch: the changelog explicitly
says “Released on 2026-10-21,” GitHub's latest release is 0.13.4, querying release
0.13.5 returns “release not found,” and the cited workflow concluded `cancelled`.
The workflow metadata establishes cancellation, not its reason or intent.

astral-sh/uv#22787 prepared 0.13.5; astral-sh/uv#22788 subsequently reverted its Python update. astral-sh/uv#22789 separately requests refreshed Docker images. No existing discussion was found that tracks the misleading release status.

## Draft response

You're right: GitHub currently lists 0.13.4 as the latest release, and the linked 0.13.5 workflow was canceled. astral-sh/uv#22787 merged the version bump and changelog before publication, as our release process requires, but the changelog's "Released on 2026-10-21" wording is misleading while 0.13.5 remains unpublished.

astral-sh/uv#22788 subsequently reverted the advertised Python update because of reported SIGBUS crashes. That PR does not explicitly explain the workflow cancellation, so whether it was intentionally canceled still needs confirmation from the release maintainer.

For your automation, use https://api.github.com/repos/astral-sh/uv/releases/latest to discover the published version. The maintainer follow-up is to confirm the cancellation reason and reconcile the changelog with the publication status and reverted Python update.

## Classification

The reported commit explicitly labels 0.13.5 as released, but GitHub has no 0.13.5 release, reports 0.13.4 as latest, and confirms the linked workflow was canceled. Preparing version metadata before publication is documented, but that mechanism does not make the user-facing "Released" statement accurate. This is a confirmed release-status/documentation bug even though the reporter asks for clarification. The related preparation and rollback PRs do not track this mismatch, and no regression of a previously fixed bug was established.

The original issue currently has the bug label and no comments. The report does
not need a runtime reproduction: the changelog text, release API, and workflow
status establish the inconsistency. The cancellation itself is not assumed to be
a software defect. The subsequent Python rollback supplies context but does not
prove a causal connection to cancellation.

## Related

- astral-sh/uv#22787 — Bump version to 0.13.5 (pull request, merged). Merged on 2026-10-21 at 02:55 UTC, adding the 0.13.5 version and the changelog's "Released on 2026-10-21" statement. The canceled workflow ran on this merge commit. This is the release preparation that introduced the mismatch, not an existing discussion resolving it.
- astral-sh/uv#22788 — Revert python-build-standalone release sync (pull request, merged). Merged at 04:26 UTC after the release cancellation, reverting the Python update advertised in the 0.13.5 changelog. Its body documents intermittent SIGBUS crashes during CodSpeed profiling and restoration of the 20261013 metadata. It does not explicitly establish why the release was canceled or correct the changelog's publication status.
- astral-sh/uv#22789 — Rebuild 0.13.4 Docker images to pick up patched ca-certificates (eMudhra CA compromise) (issue, open). Requests rebuilding 0.13.4 images or releasing 0.13.5 to obtain patched system certificates. It concerns the same release window, but tracks stale Docker base images rather than the changelog/publication mismatch; it is not a duplicate or evidence of the cancellation's cause.

## Supporting evidence

- The changelog at the exact reported commit contains the 0.13.5 heading and
  “Released on 2026-10-21” at lines 6–8:
  https://github.com/astral-sh/uv/blob/79422c835f214875852bf2b2cb950c44684eb046/CHANGELOG.md#L6-L14
  Its Python section still advertises the 20261020 python-build-standalone update.
- astral-sh/uv#22786 merged the Python update at 01:46:38 UTC on 2026-10-21.
  astral-sh/uv#22787 merged the version bump and changelog at 02:55:36 UTC.
  Its merge commit is `140db8caa82542e66d444a1a1162a41d8584ea6e`, which matches
  the canceled workflow's head SHA. The PR body and comments provide no
  cancellation explanation.
- Release workflow 39535970462 started at 02:58:22 UTC and was last updated at
  03:03:47 UTC with status `completed` and conclusion `cancelled`:
  https://github.com/astral-sh/uv/actions/runs/39535970462
  Planning and the release gate succeeded; builds were canceled,
  `publish-github` was canceled, and PyPI and Docker publication were skipped.
  No job in the returned job list had a failure conclusion. The recent release
  run listing contains no later retry or replacement run.
- astral-sh/uv#22788 merged at 04:26:17 UTC. Its body reports intermittent SIGBUS
  crashes from the `cpython-3.14.8-linux-x86_64-gnu` archive during CodSpeed
  walltime profiling. Its diff restores Python metadata and sysconfig targets to
  20261013; it does not modify the changelog or version. Its body and comments
  do not explicitly connect the rollback to cancellation of the release workflow.
- The reported changelog commit is later, at 11:50:25 UTC, so its released wording
  remained after both cancellation and the rollback.
- GitHub's latest-release API returns 0.13.4, published on 2026-10-20 at 15:24:59 UTC:
  https://api.github.com/repos/astral-sh/uv/releases/latest
  The published release is https://github.com/astral-sh/uv/releases/tag/0.13.4.
  `gh release view 0.13.5 --repo astral-sh/uv` reports “release not found.”
  Other distribution registries were not independently checked.
- `CONTRIBUTING.md`, lines 364–387, documents generating and merging the
  changelog/version bump before manually starting the release workflow.
  `scripts/release.sh`, lines 13–14, generates the changelog during preparation.
  `.github/workflows/release.yml`, lines 366–379 and 427–433, gates GitHub
  publication on prerequisite jobs and creates the release afterward. This
  explains how repository metadata can precede publication; it does not establish
  that the released wording is accurate.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered 0.13.5, the reported commit SHA, workflow ID, changelog, and release canceled/cancelled. Conceptual searches covered unreleased versions, releases not available/not published, automation selecting the latest release, and internal:releases. Fix-oriented searches covered release/publish/changelog PRs and historical release failures. PR search omitted known matches, so coverage was supplemented with the 200 most recently updated PRs, commit metadata, and direct PR/comment/diff inspection. Followed the Python update/revert chain and astral-sh/uv#3198 to its merged fix astral-sh/uv#3217; that historical partial-publication failure differs from this canceled, unpublished release. Also ruled out astral-sh/uv#1774 (missing release notes) and astral-sh/uv#18813 (dry-run manifest failure). No duplicate was established.

Additional inspected candidates were astral-sh/uv#6440 (changelog-generation
features for user projects), astral-sh/uv#4432 (the 2024 PyPI size-limit incident),
astral-sh/uv#15367 (missing extended DockerHub images for 0.8.12), and
astral-sh/uv#16350 (Docker rate limits). Their documented mechanisms differ from
this report. The historical fix in astral-sh/uv#3217 upgraded cargo-dist after a
release was partially published; this run stopped before publication, so that
history does not establish a regression.

Search results were not treated as exhaustive: PR search returned no matches even
for directly verified release PRs, and one batch encountered a temporary
secondary rate limit before a later retry succeeded. Recent PR listings and
direct inspection supplied the release-specific evidence.

## Maintainer follow-up

Confirm whether the release was intentionally canceled and document the reason.
Reconcile the changelog's release status and Python-update description with the
actual release plan. No publication date or restart is established by the inspected
evidence. The reporter can immediately avoid selecting unpublished repository
versions by using GitHub's published-release metadata.

This handoff is a proposed response for review. No GitHub changes were made, and
no checkout files were modified. Only this issue-context README was updated.
