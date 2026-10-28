# Modifying a file in a `.venv` silently corrupts the shared cache and all other venvs (hardlink mode), and uv has no way to detect or repair it

Issue: astral-sh/uv#22979

Classification: duplicate

## Summary

astral-sh/uv#15700 and astral-sh/uv#10896 match the hardlink mutation report; astral-sh/uv#888 covers RECORD verification, declined as not planned. astral-sh/uv#21968 is an adjacent open installed-file validation report.

The reporter describes deliberate dependency edits or accidental IDE refactoring inside a virtual environment changing both the shared cache and other environments. They request detection, warnings, cache invalidation, and reinstallation, preferably using per-file hashes in `.dist-info/RECORD`, or clearer documentation if verification is not added.

The hardlink mechanism is confirmed by source and earlier maintainer discussions. The affected scope is paths sharing the same underlying file, not every installation of a package regardless of cache entry or link mode. Manual recovery exists; automatic integrity checking and repair are the missing capabilities.

## Draft response

An in-place edit to a hardlinked file changes the cache and every environment sharing that file. This behavior is already discussed in astral-sh/uv#15700 and astral-sh/uv#10896. Please keep follow-up on the hardlink behavior and recovery documentation in astral-sh/uv#15700. RECORD-based verification was considered in astral-sh/uv#888 and closed as not planned; uv currently has no automatic check-and-repair workflow for these edits.

You can recover by running `uv cache clean <pkg>`, then `uv sync --reinstall-package <pkg>` in each affected project. Cleaning the cache alone does not repair existing environments. If you intend to edit installed files, add `--link-mode copy` when reinstalling, or use an editable checkout of the dependency. Changing link mode alone does not replace existing hardlinks.

## Classification

astral-sh/uv#15700 and astral-sh/uv#10896 already document the same hardlink-induced cache and cross-environment mutation; astral-sh/uv#888 covers the proposed RECORD verification. Source inspection confirms shared-file installation and metadata-based reuse without per-file validation. These discussions were closed without fixing this behavior, so this is not evidence of a regression. The additional automatic-repair and documentation proposals can be discussed with those existing reports; duplicate classification takes precedence.

The duplicate decision rests on matching behavior and the existing verification request, not on treating unintended file mutation as correct merely because hardlinks explain it. The historical issues were not closed after a fix for this behavior. The report supplies no uv version, operating system, filesystem, or evidence that an earlier integrity-checking feature regressed.

The recovery and documentation suggestions are useful refinements. The current caching documentation already describes cleaning a package's cache before reinstallation, but its Cache safety section does not explicitly explain how an in-place edit inside a hardlinked venv also modifies the cache.

## Related

- astral-sh/uv#15700 — Changing python package source files in venv changes cached uv files too (closed). Same mechanism and outcome: editing an installed dependency changes the cached files and subsequently created environments. Maintainers confirmed the hardlink side effect and recommended copy mode, editable installs, or targeted cache cleaning; closure did not represent a fix.
- astral-sh/uv#10896 — Accidental edits in .venv carry over to other projects using the same dependency (closed). Reproduces accidental editor changes propagating through hardlinks to another project and the cache. Maintainers declined making installed files read-only over compatibility concerns and discussed copy-on-write; the issue was closed without eliminating hardlink mutation.
- astral-sh/uv#888 — Support hash checking against `RECORD` (closed). Covers the requested per-file RECORD verification, with maintainer discussion of checking during cache extraction or later venv validation. Closed as not planned in August 2026 in favor of whole-archive verification and attestations; it did not implement detection or repair of later local edits.
- astral-sh/uv#21968 — uv sync leaves an environment with missing NCCL library unchanged (open). Adjacent installed-file validation gap: sync accepts an environment whose RECORD lists a missing library, and targeted reinstallation restores it. Unlike this report, files are missing rather than modified through hardlinks, and cache corruption is not established.

## Report decomposition

- **Mutation and isolation:** An in-place edit under `.venv` reaches the unpacked cache and other environments sharing the hardlink. Triggers include manually testing upstream changes and IDE refactoring. Commands and subsystems include wheel installation, link mode, the shared dependency cache, and reuse by future installs.
- **Detection:** Verify cached or installed files against `.dist-info/RECORD`; optionally use size or modification time before hashing. Suggested interfaces include `uv sync --verify`, verification in `uv pip install` or `uv pip sync`, `uv cache check`, `uv cache verify`, and `uv sync --check --verify`. These are proposals, not existing commands established by the report.
- **Repair:** Warn or exit nonzero on mismatch, invalidate the contaminated cache entry, and reinstall the affected distribution, potentially through `--fix`. Reinstalling one environment does not repair other environments still sharing the modified inode.
- **Documentation:** Explain the hardlink write-through behavior in Cache safety and link-mode documentation, and describe targeted cache cleaning followed by reinstallation in each affected environment.
- **Identifiers and comparison:** `.venv`, `.dist-info/RECORD`, `link-mode`, `uv cache clean <pkg>`, and `uv sync --reinstall-package <pkg>` are the important exact identifiers. Conda's SafetyWarnings are a comparison, not a uv error message. No specific dependency or release is necessary to the reported mechanism.

## Supporting evidence

### Earlier discussions and resolution

In astral-sh/uv#15700, the reporter reproduced editing a dependency in an existing environment, observing the changed bytes in the cache, and receiving those changed bytes in a newly created environment. On September 5, 2025, maintainer konstin described this as an unfortunate side effect of hardlinking and suggested `--link-mode copy` or an editable checkout. On September 6, charliermarsh recommended targeted cache cleaning when needed, noting the disk-space and installation-time costs of copying.

In astral-sh/uv#10896, an accidental editor change was reproduced across two projects and the cache. On January 23, 2025, charliermarsh confirmed the mechanism and explained the compatibility concern with making installed files read-only. The February 1 closure said a change was unlikely and suggested broader use of copy-on-write in the future; it was not a correctness fix.

In astral-sh/uv#888, maintainers explicitly discussed validation while extracting into the cache and later validation of an existing environment. The December 2025 discussion cited performance and favored whole-archive checks. The August 5, 2026 closure declined RECORD verification in favor of attestations, referring to PEP 815. That decision does not mean archive verification detects subsequent edits to unpacked files.

astral-sh/uv#21968 reports a different form of missing installed-file validation: metadata and RECORD remain present while a library is missing, and ordinary sync does not restore it. The reporter explicitly leaves the cause of the missing files unconfirmed. It is useful adjacent context, but the hardlink reports are closer matches.

### Current source and documentation

- `crates/uv-install-wheel/src/linker.rs:264` configures linking from the unpacked wheel into site-packages and explicitly copies RECORD because installation modifies it.
- `crates/uv-fs/src/link.rs:775` implements file hardlinking through `fs_err::hard_link`. An in-place write changes the shared inode. Replacing a pathname with a new file can break that link, so not every editor save necessarily propagates.
- `crates/uv-fs/src/link.rs:39` defaults to clone mode on macOS and Linux. `crates/uv-fs/src/link.rs:266` defines the clone-to-hardlink-to-copy fallback. Older comments about Linux defaults should not be repeated as current behavior; clone mode can still fall back to hardlinks on unsupported filesystems.
- `crates/uv-installer/src/plan.rs:350` handles explicit reinstallation and otherwise skips satisfied installed distributions. The registry branch in `crates/uv-installer/src/satisfies.rs:84` checks distribution origin and version constraints rather than installed-file contents.
- `crates/uv-cli/src/lib.rs:691` exposes cache clean, prune, dir, and size, with no cache check or verify subcommand. `crates/uv-cli/src/lib.rs:7083` documents reinstall options, including targeted reinstallation.
- `docs/concepts/cache.md:24` documents cleaning a package's cache and forcing reinstallation, explicitly suggesting cache cleaning first. `docs/concepts/cache.md:126` describes concurrency safety and warns against modifying the cache directly, but does not explicitly connect hardlinked venv edits to cache mutation.
- `crates/uv-cli/src/lib.rs:5771` documents link-mode defaults and warns about symlink coupling; it does not provide the requested hardlink-edit warning.

The supported recovery advice is to clean the affected package's cache and reinstall it in every affected project. Cache cleanup removes cache paths; it does not rewrite the bytes behind hardlinks retained in installed environments. Copy mode prevents future write-through for files actually installed with that mode, so switching the setting must be accompanied by reinstallation for existing files.

No new reproduction or test suite was run. The mechanism and existing interfaces were checked against source, documentation, and the earlier reproductions and maintainer replies.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues using literal hardlink/edit/modify/corrupt, RECORD/hash/verify, cache check/verify/repair, and link-mode documentation queries; conceptual searches included cache poisoning, integrity, checksums, read-only protection, and installed-file validation. Searches omitted incidental packages and platforms. Searched pull requests across all states, including closed and merged filters, for hardlink, clone, RECORD, cache integrity/verification, repair, and documentation. No matching integrity-repair PR was identified. Inspected candidate comments and followed astral-sh/uv#20907 to astral-sh/uv#15357 and merged astral-sh/uv#13437: those concern collisions between distributions, not later edits to hardlinked files. Ruled out astral-sh/uv#11723 because its apparent hash mismatch was an encoding misunderstanding; astral-sh/uv#5611 concerns multi-user cache permissions. Some broad searches were capped at 100 results, and GitHub rate-limited later follow-up queries, limiting completeness.

The closest candidates were inspected in full, including their comments and closure explanations. No previously merged fix for post-install hardlink mutation or automatic integrity repair was established.

Especially plausible exclusions:

- astral-sh/uv#11723 — The apparent mismatch between RECORD hashes and installed files was resolved as base64 versus hexadecimal encoding, not modified cache contents.
- astral-sh/uv#20907 — Shares a request for sync to detect missing files, but its primary problem is overlapping distributions. Maintainers redirected it to astral-sh/uv#15357.
- astral-sh/uv#15357 and astral-sh/uv#13437 — The tracking issue and merged warning implementation concern two distributions writing the same module during installation. They do not detect later user edits or repair a poisoned cache.
- astral-sh/uv#5611 — Documentation for sharing a cache across users, chiefly ownership and permissions. Comments mention poisoning and edits affecting other applications, but the direct single-user hardlink reports are better canonical references.
- astral-sh/uv#19204 — Discusses link-mode selection, disk usage, and repeated downloads; it does not track integrity verification or repair.

## Maintainer next step

Centralize the hardlink behavior and recovery-documentation discussion in astral-sh/uv#15700 and point to the existing RECORD-verification decision in astral-sh/uv#888. The proposed reply gives the reporter a concrete manual recovery and prevention workflow without promising an automatic verifier or treating the historical closures as fixes.
