# Index from a path/git dependency's `tool.uv.sources` is used for the dependent project's packages

Issue: astral-sh/uv#23196

Classification: duplicate

## Summary

The reporter uses uv 0.13.7 on Linux x86_64 with Python 3.14.10. An application depends on a library through a local directory or Git source. The library depends on torch and pins it to the PyTorch CPU index in its own `tool.uv.sources`. Running `uv lock` in the application selects that CPU build even though the application defines no index. When the application adds a different torch index pin, locking fails with `Requirements contain conflicting indexes for package torch`.

The report also explains why the available switches do not provide the desired isolation: `--no-sources` discards the application's sources too, while `--no-sources-package lib` removes the source used to locate lib instead of ignoring sources declared inside lib. The reporter asks whether this is intended and offers documentation clarification.

astral-sh/uv#16471 is the canonical match for inherited dependency index pins and consumer conflicts. Merged astral-sh/uv#14910 explains the narrower semantics of --no-sources-package.

## Draft response

uv currently reads `tool.uv.sources` from Git and local-directory dependencies. Those index pins participate in the consuming project's resolution, so incompatible pins for `torch` conflict. The same Git dependency case is discussed in astral-sh/uv#16471, where disabling a dependency's own sources was proposed.

`--no-sources-package lib` disables source selection for `lib`; it does not ignore the sources declared inside `lib`. A per-dependency switch for that is not implemented. Please add your local-path example and documentation proposal to astral-sh/uv#16471 so we can keep that discussion together.

## Classification

Open astral-sh/uv#16471 already tracks the same dependency-owned PyTorch index pin conflicting with a consumer's chosen index, with a confirmed Git reproduction and maintainer discussion of a dependency-local source-disable option. The local-path reproduction and documentation proposal extend that discussion without creating a separate underlying problem. Current source explicitly reads Git/directory dependency sources and rejects conflicting indexes in one resolution environment; no evidence establishes that a previously fixed isolation feature regressed.

The strongest match is the actual Git reproduction in the comments of astral-sh/uv#16471, rather than its more general title about local overrides. The library's cu118 pin conflicts with the application's cu128 pin. On October 27, 2025, maintainer konstin proposed a per-dependency `sources = false` option specifically to disable sources, especially index sources, belonging to a dependency. That is the same isolation boundary requested here.

This classification does not depend on treating all current behavior as correct. It centralizes the already tracked behavior and configuration limitation. The reporter's question wording and documentation proposal do not outweigh the direct existing discussion. No evidence shows that the previously proposed dependency-local switch was implemented and then regressed.

## Related

- astral-sh/uv#16471 — Support local package-specific index overrides without modifying pyproject.toml (open; issue). Same underlying problem: a Git-installed library pins torch to cu118 while its consumer pins cu128, producing conflicting indexes. The reporter confirms the Git setup in comments; maintainer konstin proposes disabling the dependency's own sources with a per-dependency sources=false option. This directly covers the requested isolation; the new report also supplies a local-path example and documentation proposal.
- astral-sh/uv#14910 — Add `--no-sources-package` (merged; pull request). Introduced the flag tested in this report, merged January 15, 2026. It disables sources by the name of the requirement being resolved, rather than by the project declaring those sources. Thus disabling lib also removes the consumer's path/Git source for lib; this PR does not implement dependency-local source isolation.

## Supporting evidence

### Canonical discussion and flag history

In astral-sh/uv#16471, the reporter's October 27, 2025 follow-up supplies a consumer with a Git source for the library and a cu128 torch pin. The library's own cu118 pin produces the same conflicting-index error. Konstin distinguishes Git dependencies from installed wheels/source distributions and subsequently proposes dependency-local source disabling. He explicitly says the earlier dependency-group suggestion does not work for that setup. Do not recommend it as an established workaround.

The broader alternative linked there, astral-sh/uv#6772, concerns defining sources outside `pyproject.toml`. It is an alternative configuration design, not a closer canonical discussion of dependency-owned pins. Its maintainer comments describe that external-source configuration as unsupported and not on the roadmap at the time of those comments.

Closed astral-sh/uv#14694 points to astral-sh/uv#14910. The merged PR introduces selective source disabling by requirement name, including the use case of disabling a build dependency's source while retaining a workspace member's source. It does not introduce a switch keyed by the project whose source declarations should be ignored.

### Source and documentation

- `crates/uv-distribution/src/metadata/requires_dist.rs:30` explicitly describes reading `tool.uv` for Git and directory dependencies. Its project-lowering code collects the dependency project's indexes and sources at lines 115 and 126, then applies them to that project's requirements at line 204.
- `crates/uv-distribution/src/metadata/requires_dist.rs:196` checks `no_sources.for_package(&requirement.name)`. Together with `crates/uv-configuration/src/sources.rs:39`, this confirms that selective disabling follows the target requirement's name. Selecting lib does not mean ignoring every source declaration inside lib.
- `crates/uv-resolver/src/fork_indexes.rs:18` requires a package to use one index within a resolution fork and emits `ConflictingIndexesForEnvironment` when a different index is inserted. This explains why a consumer pin does not automatically override a dependency pin.
- `crates/uv-workspace/src/pyproject.rs:1214` defines source variants; the Git source fields do not include the proposed `sources = false` switch.
- `docs/concepts/projects/dependencies.md`, under Dependency sources and Disabling sources, describes development sources and global disabling. `docs/concepts/indexes.md`, under Pinning a package to an index, describes strict index pinning. The inspected passages do not explicitly explain that Git/directory dependency sources participate in downstream resolution. A clarification can be discussed alongside the canonical issue.

### Existing test coverage

`crates/uv/tests/lock/lock.rs:19226` contains `lock_no_sources_package`, gated by `test-universal` and `test-git`, which verifies disabling a named Git source while retaining another. This supports the flag semantics but does not test ignoring sources belonging to a dependency project.

The reporter's suggested neighbor, `lock_multiple_sources_index_overlapping_extras` at `crates/uv/tests/lock/lock.rs:32807`, is gated by `test-universal`. It checks overlapping extra-specific index pins in a single project, not source inheritance from a Git or directory dependency. No tests were added or run for this triage.

## Other candidates and historical fixes

The exact-error candidate astral-sh/uv#9942 initially looks close because it combines path dependencies, torch, and conflicting indexes. Its scenarios require conflicting optional extras declared on both library and application. Merged astral-sh/uv#10875, dated January 22, 2025, relaxes unconditional conflicting-extra checks and moves validation to installation. The present report has ordinary unconditional dependencies and incompatible unconditional pins, with no extras. That historical fix does not establish a regression here.

astral-sh/uv#10410 and astral-sh/uv#13353 concern source pins being omitted from published wheel metadata, the opposite direction and a different dependency form. astral-sh/uv#9446 concerns making a consumer's sources apply to indirect dependencies without declaring those dependencies directly. astral-sh/uv#20765 concerns mirror overrides in `uv.toml`, without a dependency project introducing a competing pin. Broader discussions in astral-sh/uv#7945, astral-sh/uv#9258, astral-sh/uv#19701, and astral-sh/uv#7055 concern development overrides, source-type selection, or child workspace discovery; none supersedes the direct match.

## Search scope and verification

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Split the report into dependency-owned index inheritance, consumer/dependency index conflicts, and selective source disabling. Literal searches included tool.uv.sources, conflicting indexes, no-sources-package, and sources = false; conceptual searches included transitive sources, sources/git/index, dependency source deactivation, inheritance, consumer overrides, and development versus published metadata. Followed astral-sh/uv#16471 to astral-sh/uv#6772 and astral-sh/uv#14694 to merged astral-sh/uv#14910. Inspected astral-sh/uv#9942 and its merged fix astral-sh/uv#10875 but ruled out a regression match: those concern conflicting optional extras, absent here. Also distinguished astral-sh/uv#10410 and astral-sh/uv#13353 (pins omitted from published wheels), astral-sh/uv#9446 (consumer sources for indirect dependencies), astral-sh/uv#20765 (uv.toml mirror precedence), and broader development/source-control discussions. PR keyword searches returned no additional candidates; referenced PRs were inspected directly. Checked source, existing integration tests, and the two documentation sections named in the report.

This handoff is based on repository source, existing snapshots, and issue/PR discussion. The supplied reproduction was not executed, so the exact reported build/version selection was not independently verified. Only this temporary README was written; no checkout files or GitHub state were changed.
