# Implicit inheritance of `uv.tool` section in pyproject.toml across repositories causes confusion between multiple projects in nested checkouts

Issue: astral-sh/uv#22185

Classification: bug

## Summary

Two independent Python repositories normally live side by side, but GitLab CI requires the second checkout to live inside the first. Both have their own `pyproject.toml`. The parent declares `[[tool.uv.dependency-metadata]]`; the child has no `[tool.uv]` table. The reporter says `uv lock --check` then treats the child's lockfile as outdated. Adding an empty `[tool.uv]` to the child avoids the failure.

The report lists uv 0.11 and 0.12, with continued behavior through 0.12.23 explicitly described as an assumption. Platform and Python version are reported as independent. No minimal files or command output were supplied. The only current issue comment offers to investigate; it does not confirm a reproduction or fix.

No exact duplicate found. astral-sh/uv#5931 and astral-sh/uv#5929 explain ancestor discovery; closed, unmerged astral-sh/uv#21894 addresses unused metadata invalidation, and astral-sh/uv#19196 tracks a similar CI failure from user-level configuration.

## Draft response

Your empty `[tool.uv]` workaround matches current discovery: uv skips a `pyproject.toml` without that table and searches parent directories without stopping at Git boundaries. Inherited dependency metadata is then compared with the child’s lockfile. astral-sh/uv#5931 introduced searching above workspace roots; its review also discussed excluding parent `pyproject.toml` files, which current discovery still reads.

For CI, keep the empty table, or use `uv lock --check --no-config` if the child does not need discovered uv settings; that also disables user/system configuration. `uv lock --check -v` can show the selected configuration and metadata mismatch. Bounding discovery for independent checkouts still needs a fix that retains intentional sharing through parent `uv.toml` files.

## Classification

An independent checkout acquires another project’s resolver configuration solely because it is nested, changing validation of its own lockfile. Source confirms that discovery skips a child pyproject.toml without tool.uv, traverses ancestors without a Git boundary, and compares discovered static metadata with the lockfile. Documented ancestor lookup explains the mechanism but does not resolve this cross-project isolation defect. The related items cover different boundaries or configuration inputs; there is no evidence of a previously fixed instance of this bug regressing.

The correctness concern is configuration from a separate project changing an independent checkout's lock validation. The general ancestor search is longstanding, documented, and tested; changing its boundary requires preserving intentional configuration sharing. That history does not establish that the nested-checkout failure was previously fixed. The August 2024 fix in astral-sh/uv#5931 restored discovery of intended parent configuration, while the October 2026 report concerns unwanted parent configuration. The metadata-pruning proposal in astral-sh/uv#21894 was closed without merging on September 24, 2026 and cannot be presented as a released fix.

No inspected existing discussion covers both independent nested checkouts and unwanted parent `pyproject.toml` resolver settings closely enough to centralize this report as a duplicate. In particular, user-level settings in astral-sh/uv#19196 are intentionally global, whereas this report concerns an unrelated project's configuration.

## Related

- astral-sh/uv#5931 — Search beyond workspace root when discovering configuration (pull request, merged). Introduced configuration discovery above workspace roots. Maintainers explicitly discussed ignoring parent pyproject.toml files while continuing to read parent uv.toml files, but deferred that change. This explains the relevant history; it did not fix independent-checkout isolation.
- astral-sh/uv#5929 — Parent uv.toml configuration not used when pyproject.toml present (issue, closed). A child pyproject.toml without uv settings incorrectly blocked intentional parent uv.toml discovery; astral-sh/uv#5931 fixed it in August 2024. The new report concerns unwanted parent pyproject.toml settings crossing independent checkout boundaries, so it is neither a duplicate nor a regression of that opposite failure.
- astral-sh/uv#21894 — Omit unused dependency-metadata entries from lockfiles (pull request, closed). Proposed preventing unused static-metadata declarations from invalidating lockfiles, directly relevant if the inherited entries are unused by the child. Closed without merging before this report; it does not restrict configuration discovery, and the report does not establish that every inherited entry is unused.
- astral-sh/uv#19196 — Configuration merging produces unusable uv.lock files (issue, open). Also reports configuration outside a project making lock validation fail in CI. Its trigger is user-level exclude-newer-package settings, rather than dependency-metadata discovered in a separate parent checkout. The related astral-sh/uv#20652 was explicitly centralized there; independent-checkout discovery remains a distinct problem.

## Report decomposition

- **Configuration isolation:** A child checkout with its own `pyproject.toml`, but no `[tool.uv]`, picks up the parent's settings. The implicit requested improvement is to avoid surprising coupling between otherwise independent projects.
- **Lockfile correctness and diagnostics:** Inherited `[[tool.uv.dependency-metadata]]` makes `uv lock --check` reject a lockfile that passes when the projects are siblings. The report describes difficulty locating the source of the changed configuration, rather than supplying an exact error string.
- **Trigger and workaround:** GitLab CI changes the directory relationship from siblings to nested checkouts. An empty child `[tool.uv]` stops discovery of parent project configuration. GitLab is the reason for the layout, not a confirmed uv-specific platform mechanism.
- **Subsystems and identifiers:** Filesystem settings discovery, project/workspace discovery boundaries, static dependency metadata, and lock validation; `pyproject.toml`, `[tool.uv]`, `[[tool.uv.dependency-metadata]]`, `uv.toml`, `uv lock --check`, and `--no-config`. The title says `uv.tool`; the body and actual configuration use `tool.uv`.

## Supporting evidence

Source inspection used checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`.

- `crates/uv/src/lib.rs:339` starts settings discovery at the discovered workspace's install path. Project/workspace discovery and settings discovery are separate operations; a child having its own project root does not itself stop settings traversal.
- `crates/uv-settings/src/lib.rs:102` iterates over `path.ancestors()` until configuration is found. It has no Git-repository boundary check. At line 172, `from_directory` skips a `pyproject.toml` without `[tool.uv]`; an empty table supplies an options object and stops the ancestor search. This selects the nearest project configuration rather than merging every ancestor's table.
- `docs/concepts/configuration-files.md:27` documents skipping files without the table. Line 69 documents `--no-config`; the same page distinguishes project configuration from merged user/system settings. An empty child table stops ancestor project lookup but still allows user/system settings.
- `crates/uv/tests/sync/show_settings.rs:2945`, `resolve_skip_empty`, already tests skipping a child project without `[tool.uv]`, using an ancestor `uv.toml`, and stopping that lookup when an empty child table is added. It does not specifically test two separate Git checkouts or static metadata invalidating a child lockfile. Any implementation should extend the appropriate integration coverage without duplicating this general discovery test.
- `crates/uv-lock/src/lock/mod.rs:4486` compares configured static metadata, after the applicable lock filter, with the recorded manifest metadata and returns `MismatchedStaticMetadata` on a difference. `crates/uv-lock-operations/src/validated_lock.rs:429` logs “Resolving despite existing lockfile due to mismatched static metadata” with the requested and existing values. This supports the reported causal path, but the reporter's exact metadata and version-specific execution remain unverified.
- In astral-sh/uv#5931, Charlie Marsh proposed considering only `uv.toml` above workspace roots, Zanie agreed, and Charlie said that would be separate work. The merged PR itself enabled traversal; current source still accepts ancestor `pyproject.toml` files. These comments establish historical design discussion, not a promise or completed boundary fix.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues with authenticated gh: literal tool.uv/parent, dependency-metadata/lock/outdated, lock --check/parent, empty tool.uv, and mismatched static metadata; conceptual nested repositories, configuration inheritance/discovery, ancestor traversal, Git/filesystem boundaries, independent projects, and configuration-dependent CI lock failures. Checked version-specific and historical fixes and followed candidate comments and closing PRs. REST search was rate-limited and GraphQL PR searches returned no PR nodes, so enumerated all 12,901 PRs (602 open, 1,428 closed, 10,871 merged), searched their titles for discovery, boundaries, inheritance, static metadata and lock invalidation, then inspected relevant bodies/comments. Ruled out astral-sh/uv#11070 (explicit configuration merging), astral-sh/uv#7351 (filesystem mount boundaries), astral-sh/uv#16502 (explicit workspace membership), astral-sh/uv#3936 and astral-sh/uv#8665 (cached dependency workspace discovery), and astral-sh/uv#13614 and astral-sh/uv#13635 (marker-normalization regression). No exact duplicate or previously merged fix for independent-checkout configuration isolation was found.

The search vocabulary also used the repository's `area:configuration`, `area:workspaces`, `area:lock`, and `area:error-messages` distinctions. Observable lock-check failures were searched separately from the proposed discovery mechanism. Broad searches omitted GitLab, uv versions, and platform details to find the underlying behavior. PR title enumeration was a fallback for unavailable reliable PR text search; relevant candidates were then read in full, including their comments. It is not a claim to have searched every PR comment.

Other inspected candidates do not establish a duplicate or regression:

- astral-sh/uv#11070 requests explicitly merging ancestor configuration even when a child already has its own `[tool.uv]`; astral-sh/uv#5596 requests an `extend` option. Their goal differs from preventing accidental ancestor discovery. The referenced astral-sh/uv#10960 is broader monorepo documentation.
- astral-sh/uv#7351 concerns crossing filesystem mount boundaries and an NFS-created directory named `uv.toml`. Stopping at a mount would not isolate nested repositories on the same filesystem.
- astral-sh/uv#16502 declares the submodule as a workspace member and requests an independent lockfile. Maintainer advice there is to use a path dependency. The current report describes independent projects, not declared workspace membership.
- astral-sh/uv#3936 and its merged fix astral-sh/uv#8665 bound workspace discovery for cached Git dependencies. They do not bound the top-level filesystem settings lookup used here.
- astral-sh/uv#13614 and its merged fix astral-sh/uv#13635 concern equivalent dependency markers comparing differently during lock validation. Their same broad lock-check error does not establish a regression in nested-checkout discovery.
- astral-sh/uv#18053 and astral-sh/uv#18087 concern named-index visibility and a diagnostic hint, not inherited static metadata. astral-sh/uv#21284 improves requirement-mismatch diagnostics after metadata collection fails, rather than identifying or bounding inherited configuration.
- astral-sh/uv#20652 was closed explicitly as a duplicate of astral-sh/uv#19196, so the latter is the canonical adjacent discussion for user-level cooldown configuration in lockfiles.

## Maintainer next step and verification limits

Keep the independent-checkout isolation problem distinct from optional pruning of unused metadata. A fix needs to decide where ancestor `pyproject.toml` lookup stops while preserving intended shared `uv.toml` lookup. A focused regression fixture should include two independent nested projects, parent static metadata, and a lockfile created for the child independently; the report does not establish whether the inherited metadata names packages present in the child.

The existing empty-table workaround is supported by source and tests. `--no-config` is an alternative only when disabling all discovered uv settings is acceptable. Verbose lock-check output can identify the actual metadata mismatch if further confirmation is needed.

This handoff is based on the issue, related GitHub discussions, documentation, and source inspection. No build or runtime reproduction was performed. No checkout files were edited and no GitHub changes were made. Only temporary handoff/search artifacts were written. Existing checkout changes were left untouched.
