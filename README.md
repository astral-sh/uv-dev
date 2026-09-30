# Lockfile v2

Issue: astral-sh/uv#22106

Classification: enhancement

## Summary

astral-sh/uv#22106 is a tracking issue for changing the `uv.lock` major format version from 1 to 2. It does not report a failure, command-specific symptom, platform constraint, or error message. The current source writes lockfile version 1, revision 5 and documents the distinction: revisions are for backwards-compatible format changes, while a major-version change is backwards- and forwards-incompatible and each uv version supports a single lockfile major version.

No earlier Lockfile v2 tracker or implementation pull request was found. The closest precedent is astral-sh/uv#4893, the completed tracker that coordinated breaking schema and content changes before the version 1 format was declared ready. Merged astral-sh/uv#5861 was one concrete breaking change under that tracker and explicitly noted that its schema rename invalidated existing lockfiles. More recently, merged astral-sh/uv#22010 advanced the existing format to version 1 revision 5, establishing the current baseline and illustrating the separate use of revisions for compatible additions.

## Draft response

We’ll use astral-sh/uv#22106 as the canonical tracker for the breaking schema and migration work required for uv.lock version 2. The current implementation writes version 1, revision 5; revisions cover backwards-compatible additions, while a major version denotes incompatible format changes. astral-sh/uv#4893 is the corresponding completed tracker for stabilizing version 1, so it is useful precedent rather than a duplicate. As the scope is defined, please add the individual schema, compatibility/migration, and rollout tasks here.

## Classification

This is an enhancement. It tracks planned functionality and coordinated format evolution rather than incorrect current behavior or a support question. The source confirms that a major-version bump represents incompatible changes, so the requested transition is materially different from an ordinary revision bump. It is not a duplicate: astral-sh/uv#4893 completed the stabilization of version 1 in 2024 and does not track version 2.

## Related

- astral-sh/uv#4893 — “Tracking issue: Stabilize `uv.lock` format” (closed). This is the closest historical analogue: it coordinated breaking uv.lock schema and content changes before version 1 was declared ready. It is completed version 1 work, so it is precedent rather than a canonical duplicate.
- astral-sh/uv#5861 — “Rename `distribution` to `packages` in lockfile” (merged). This implemented a breaking-schema item from astral-sh/uv#4893. Its description says the change invalidated existing lockfiles, making it relevant migration precedent for work under a new major version.
- astral-sh/uv#22010 — “Record workspace member default groups in lockfiles” (merged). This recent change bumped the current format to version 1 revision 5. It provides the current baseline and shows that compatible additions use revisions rather than a major-version transition.

## Search evidence

Literal searches covered the exact title, “Lockfile v2,” `uv.lock` with version 2, lockfile major-version bumps, and exact version/revision fields across open and closed issues and open, closed, and merged pull requests. Conceptual searches covered lockfile format and schema stabilization, breaking or incompatible changes, backwards compatibility, migration, and format upgrades. Historical review followed astral-sh/uv#4893 into its linked schema pull requests and inspected the current revision 5 bump.

No earlier version 2 tracker or implementation pull request was found. astral-sh/uv#15220 was an initially plausible format-upgrade result, but it requests a way to force rewriting a lockfile to a newer backwards-compatible version 1 revision and was ultimately resolved with `--refresh`; it does not cover a new major format.
