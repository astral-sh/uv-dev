# Allow user-level `uv.toml` to be split

Issue: astral-sh/uv#22905

Classification: duplicate

## Summary

The reporter maintains a shared user-level `uv.toml` in a configuration repository and needs separate machine-dependent settings. They propose a sibling `uv.toml.d` directory containing fragments such as `000-certificates.toml` and `001-indexes.toml`. Fragments would be applied in lexicographic order after the main file, and only files ending in `.toml` would be loaded. Environment variables, symlinks, and a system-level configuration are acknowledged workarounds.

astral-sh/uv#5596 is the closest existing request for configuration composition. astral-sh/uv#11070 provides related inheritance design discussion; merged astral-sh/uv#7851 supplies the existing system-level layer, not user-level fragments.

## Draft response

You're correct: uv currently reads a single user-level `uv.toml` and does not load a `uv.toml.d` directory. The underlying request to combine shared configuration with local settings is tracked in astral-sh/uv#5596. Please add your machine-specific use case and the ordered `.toml` fragment proposal there so we can keep the design discussion together. That request remains on the wishlist, with no implementation timeline.

## Classification

astral-sh/uv#5596 already tracks the underlying request to compose shared configuration with separately maintained local settings. The machine-specific use case and ordered uv.toml.d proposal can be discussed there as an alternative design. Source confirms that uv currently loads one user-level uv.toml; no incorrect behavior or regression is established.

The proposed directory discovery, ordering, and extension filter are new design details, not features already implemented by the related request. Centralizing discussion in astral-sh/uv#5596 does not imply that its explicit `extend` design or this directory-based alternative has been approved. The issue asks for new composition functionality rather than reporting a failure of supported configuration loading.

## Related

- astral-sh/uv#5596 — Feature Request: `extend` config option (open; issue). Tracks composing separately maintained shared configuration with local settings through a Ruff-like extend option. This is the same underlying need as splitting shared and machine-specific user settings; uv.toml.d, lexicographic ordering, and .toml filtering are an alternative design. The issue has the wish label, and a maintainer said it was not a near-term priority.
- astral-sh/uv#11070 — Inheriting [tool.uv] section from root-level `pyproject.toml` when desired (open; issue). Related configuration composition request, explicitly cross-referenced to astral-sh/uv#5596. A maintainer noted inheritance complexity and existing user/system merging. Its scope is parent/child project configuration in a monorepo, rather than multiple user-level fragments.
- astral-sh/uv#7851 — Add support for system-level `uv.toml` configuration (merged; pull request). Implemented the system-level layer mentioned as a workaround, released in uv 0.4.26. The discussion chose the first discovered system file rather than merging all system files. It does not implement user-level fragments or resolve this request.

## Supporting evidence

- `crates/uv-settings/src/lib.rs:55`: `FilesystemOptions::user()` constructs the user configuration path ending in `uv/uv.toml` and calls `read_file` once. It does not enumerate a fragment directory.
- `docs/concepts/configuration-files.md:45` documents the user and system file locations. At line 55 it defines project-over-user-over-system precedence, with only the first discovered system file used. At line 61 it distinguishes scalar replacement from array concatenation.
- `crates/uv/src/lib.rs:313` and `docs/concepts/configuration-files.md:72` confirm that `--config-file` replaces discovered configuration; it is not an additional file merged into the user configuration.
- astral-sh/uv#5596 requests a Ruff-like `extend` directive for separately maintained shared settings. It remains open with `wish` and `area:configuration` labels. On 2024-10-22, zanieb said the team was unlikely to devote time to it in the near future.
- In astral-sh/uv#11070, charliermarsh explained on 2025-01-30 that configuration inheritance brings complexity and confusion, while user/system inheritance already exists. A later comment explicitly connects that discussion to astral-sh/uv#5596. These comments concern inheritance generally; they are not a decision on the newly proposed fragment directory.
- astral-sh/uv#7851 merged on 2024-10-21. Its discussion chose the first discovered system configuration rather than merging every system file. `changelogs/0.4.x.md:1088` records its release in uv 0.4.26. The originating astral-sh/uv#6742 requested one system-level layer and is closed; neither establishes prior support for user-level fragments.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separately searched file splitting (uv.toml.d, uv.toml split, multiple files), ordering (lexicographical, lexicographic, alphabetical), and filtering (suffix, extension). Conceptual searches covered include, extend, inheritance, fragments, drop-in, machine-specific settings, dotfiles, and area:configuration. Reviewed candidate comments and cross-reference chains, plus historical system-configuration support. Ruled out astral-sh/uv#12013 and astral-sh/uv#6772 as dependency-source override requests, and astral-sh/uv#20995 as sharing a specific setting across workspaces. PR search omitted even a known matching PR, so search results were supplemented with direct PR reads, changelogs, and a PR title inventory. Inspected and ruled out astral-sh/uv#22554 (allocation changes that retain merge semantics) and astral-sh/uv#3203 (fixing a duplicated uv.toml path component), and checked astral-sh/uv#3049 (the original single user-file support).

The report has no error message, version-specific failure, or claimed regression. Searches therefore focused on configuration composition and discovery, with historical implementation checks for the system-level workaround. No earlier match was found for the exact fragment-directory name, lexicographic loading rule, or extension filter.

astral-sh/uv#12013 was followed to astral-sh/uv#6772 through a maintainer comment. Those concern allowing dependency sources outside project metadata and their effect on lockfiles, which splitting ordinary user settings does not require. astral-sh/uv#20995 concerns centralizing `exclude-newer-package` across independent workspaces; it is less direct than the general composition request. The monorepo documentation lead astral-sh/uv#10960 was also inspected and does not track user configuration fragments.

## Handoff

Recommend centralizing the use case and proposed ordering/filtering semantics in astral-sh/uv#5596. No reproduction is needed to establish that fragment loading is absent. There is no supported implementation timeline to offer.

This document and its draft response are for maintainer review. No GitHub changes were made and no checkout files were modified.
