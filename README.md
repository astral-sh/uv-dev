# Prevent older uv versions from downgrading the lockfile revision

Issue: astral-sh/uv#22911

Classification: bug

## Summary

A contributor using an older uv release reportedly rewrote a shared `uv.lock` from revision 5 to revision 3. A subsequent `uv sync --frozen` failed with:

```text
error: Frozen lockfile discovery requires a lockfile with revision 5 or later; run `uv lock` to update it
```

The reporter requests that uv refuse to overwrite a lockfile with a revision newer than it supports, with an explicit `--force` override. Windows 11 and uv 0.13.5 are reported, but the older writer's exact version, the full rewrite sequence, and the workspace layout are not supplied.

astral-sh/uv#15220 is the closest historical discussion of mixed-version metadata loss. astral-sh/uv#22010 and astral-sh/uv#22018 establish the revision-5 requirement behind the failure; astral-sh/uv#19579 shows that the earlier --force proposal was closed unmerged. No matching downgrade-prevention fix was found.

## Draft response

The downgrade can remove metadata needed by frozen sync. The quoted error specifically comes from discovering uv.lock without the workspace’s pyproject.toml, which requires revision 5 or later (astral-sh/uv#22018). The related discussion in astral-sh/uv#15220 covers upgrading the format, but does not provide a downgrade guard.

For now, set `required-version = ">=0.12.23"` under `[tool.uv]` in pyproject.toml to reject older writers, and regenerate with `uv lock --refresh` using a compatible uv version while the workspace manifest is present. Please share the exact older uv version and command that rewrote the file; 0.13.5 alone does not identify the writer. Any write guard needs to preserve support for reading compatible newer revisions.

## Classification

Rewriting a newer lockfile with an older revision can discard metadata needed by manifest-free frozen sync, breaking a previously usable lockfile. Historical revision-3 writer code and current source support this mechanism; accepting newer revisions for reading does not protect them when rewriting. The requested refusal and override need design, but the reported metadata loss is a correctness problem. The related format-upgrade request does not track downgrade prevention, and no previously fixed downgrade guard was found.

The historical issue is related rather than a duplicate: forcing a newer uv to regenerate an old format does not protect a newer lockfile from an older writer. The earlier `--force` proposals were closed without merging; their closure is not evidence that downgrade prevention was fixed. The manifest-free sync feature added a use for revision-5 metadata, not a guard against losing it. There is no evidence here of a previously fixed bug returning.

The source confirms the rewrite mechanism and the error condition. The reporter's particular command sequence has not been reproduced. In particular, a lockfile that satisfies validation can be returned unchanged; not every invocation of `uv lock` necessarily rewrites it.

## Related

- astral-sh/uv#15220 — Force newest lock file format (revision = 3, upload-time=...) (issue, closed). Reports the same mixed-version collaboration problem: older uv releases remove newer lockfile metadata. Its request was to force a format upgrade, rather than prevent downgrades. Maintainers recommended required-version and closed it with guidance to use --refresh.
- astral-sh/uv#22018 — Support frozen sync without workspace manifests (pull request, merged). Introduced the affected manifest-free uv sync --frozen workflow, requiring lockfile revision 5 or later. Released in uv 0.12.23, before the reported 0.13.5; explains the downstream error but does not prevent older writers from downgrading the lockfile.
- astral-sh/uv#22010 — Record workspace member default groups in lockfiles (pull request, merged). Bumped the lockfile to revision 5 to record workspace-member default groups. Released in uv 0.12.22; establishes why losing newer metadata matters for frozen discovery, rather than being only a cosmetic revision change.
- astral-sh/uv#19579 — Add `--force` flag to `uv lock` for lockfile format migration (pull request, closed). Proposed --force to regenerate an older format while retaining package-version preferences. Closed unmerged because --refresh already provides that capability. It did not implement the requested downgrade refusal or an override for that refusal.

## Supporting evidence

### Reading and rewriting are separate compatibility concerns

- `docs/concepts/resolution.md:943` distinguishes schema `version` from `revision`. An unsupported schema version is rejected; revision changes are backward compatible and are deliberately readable by older uv.
- `crates/uv-lock/src/lock/mod.rs:123` lists revision history: revision 3 records package-specific upload cutoffs, while revision 5 records default groups and dependency-group metadata for workspace members and roots.
- `crates/uv-lock/src/lock/mod.rs:2641` constructs a resolved lockfile with the running binary's `REVISION`. `Lock::from_toml` at line 3776 checks schema version, without imposing a maximum readable revision.
- `crates/uv-lock-operations/src/lock.rs:270` commits a changed resolution without a newer-revision write guard. At line 839, a satisfying existing lockfile instead returns unchanged. The serialization code writes the in-memory lock's revision.
- Historical source retrieved read-only through GitHub at tag `0.8.15` confirms an actual revision-3 writer: `crates/uv-resolver/src/lock/mod.rs:75` defines `REVISION = 3`, lines 363–365 use it for a new resolution, and lines 2250–2252 retain the incoming revision during deserialization. `crates/uv/src/commands/project/lock.rs:390` commits changed locks. This version is an inspected example, not a claim about the reporter's unidentified older executable.

### The reported error requires manifest-free discovery

- `crates/uv-lock-operations/src/lockfile.rs:25` implements discovery when the workspace manifest is unavailable. Lines 66–68 reject a lockfile lacking revision-5 default-group support with the exact reported message.
- `crates/uv-preview/src/lib.rs:441` documents `frozen-lockfile`: frozen project commands can use the lockfile without the workspace's `pyproject.toml`, requiring revision 5 or later.
- Existing test `sync_lockfile_requires_revision` in `crates/uv/tests/sync/sync.rs:900` supplies a revision-4 lockfile without a manifest and snapshots this error. It confirms the error condition, not the cross-version rewrite.
- astral-sh/uv#22010 merged on September 30, 2026; the revision-5 metadata change is listed under uv 0.12.22 in `changelogs/0.12.x.md:1314`.
- astral-sh/uv#22018 merged on October 3, 2026; manifest-free frozen sync is listed under uv 0.12.23 in `changelogs/0.12.x.md:1371`. Both changes predate the reported uv 0.13.5, released October 21, 2026.

### Existing mitigation and prior decisions

- `crates/uv-settings/src/settings.rs:261` documents that `required-version` rejects uv versions outside the configured requirement; `crates/uv/src/lib.rs:553` enforces it.
- A minimum of uv 0.12.23 supports both revision-5 metadata and the reported manifest-free frozen-sync capability. Set the requirement in the source project's `pyproject.toml`, where lockfile-writing commands read it. Restore the lockfile using a compatible uv with that manifest present.
- Maintainer comments on astral-sh/uv#15220 recommend a `tool.uv.required-version` lower bound. Its final May 27, 2026 comment recommends `--refresh`.
- astral-sh/uv#15757 was an earlier `--force` implementation and closed unmerged. astral-sh/uv#19579 was also closed unmerged after maintainers noted that `--refresh` already triggers a new resolution and format rewrite.
- Merged astral-sh/uv#15994 made `uv lock --refresh` always resolve. Current `crates/uv-lock-operations/src/validated_lock.rs:255` treats refresh as a reason to reuse the existing lock as a preference rather than accept it unchanged.
- On astral-sh/uv#21890, a maintainer explicitly affirmed that older uv versions should be able to read newer revisions, while potentially missing newer features. That PR only adds guidance after parsing failures; it does not refuse a readable newer lockfile or prevent a rewrite.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues using the exact frozen-discovery error, revision/downgrade/downgrading, revision 5, overwrite, older uv, lockfile format/compatibility, preserve, force, and required-version, including area:lock terminology and searches without platform or version constraints. Searched open, closed, and merged PRs for revision, downgrade, lockfile, and frozen discovery. PR keyword searches returned no hits, so additionally scanned 3,000 recent PR titles across all states and followed historical references. Inspected candidate bodies, comments, relevant reviews, source, and release notes. Ruled out astral-sh/uv#21890 and its linked astral-sh/uv#21864 as downgrade fixes: they concern parsing newer formats and explicitly retain readable newer revisions. Also ruled out astral-sh/uv#16348 (invalid TOML recovery), astral-sh/uv#20646 (whitespace validation), and astral-sh/uv#10988 (marker changes). Followed astral-sh/uv#15220 through astral-sh/uv#15757, astral-sh/uv#19579, and astral-sh/uv#15994; no prior downgrade-prevention fix was found.

The report was decomposed before searching into: the downward revision rewrite under mixed uv versions; the frozen-discovery error after that rewrite; and the requested refusal plus explicit override. Literal symptom searches and broader capability searches were kept separate from mechanism-oriented searches. GitHub labels supplied `area:lock`, `area:projects`, and compatibility terminology. Source and release history supplied the manifest-free discovery distinction and the revision-5 timeline.

Other inspected historical context includes astral-sh/uv#13738 and its linked merged astral-sh/uv#12968: they explain the upload-time revision change and why a no-op lock may not migrate an old format. They do not provide downgrade protection. The PR-title scan is bounded to the 3,000 most recent entries; older PRs were inspected through search leads and cross-references.

## Follow-up

Ask for the older writer's exact version and command, including any dependency or configuration change that caused re-resolution. If the failure is investigated further, obtain the workspace layout and confirm whether the workspace manifest was absent when frozen sync ran. These details identify the affected path; they are not prerequisites for recognizing the metadata-loss bug.

A prospective write guard should distinguish reading a forward-compatible revision from overwriting it. Its override semantics require a maintainer decision. A change in a future uv release cannot alter already distributed older executables, so a project-level version requirement remains useful.

## Validation and scope

Evidence was gathered through read-only GitHub queries and source inspection at checkout commit `6fe4f0f5cfa8322e97af4479cbfb3d53a3ce9596`, plus the historical source tag noted above. No runtime reproduction or tests were run. No checkout files or GitHub state were changed. Only this maintainer handoff was authored.
