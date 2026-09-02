# Yanked wheel is retained in `uv.lock` even when a non-yanked wheel exists for the same version

Issue: astral-sh/uv#21430

Classification: bug

## Summary

The report demonstrates a PEP 592 partial-yank case against a devpi index: one of two wheels for the same package version is yanked, while the other remains usable. A fresh `uv lock` or `uv sync` avoids selecting the yanked wheel as the best distribution but nevertheless writes both wheel URLs and hashes to the package's `wheels` array in `uv.lock`. Reversing which wheel is yanked produces the same result.

No existing issue or pull request tracks this exact partial-yank lockfile defect. The closest precedent is astral-sh/uv#12296 and its fix astral-sh/uv#12299, where an individual wheel excluded by `--exclude-newer` was correctly rejected during resolution but still appeared in the lockfile. astral-sh/uv#6145 records the repository's recognition that yank status is per-file rather than necessarily uniform across a release.

## Draft response

Thanks for the detailed reproduction. The current source confirms a lockfile filtering gap for partial yanks, and I couldn't find an existing issue tracking this exact case. This is analogous to astral-sh/uv#12296, fixed by astral-sh/uv#12299 for `--exclude-newer`: `PrioritizedDist::built_dist` filters artifacts through `is_excluded`, but that predicate currently covers `ExcludeNewer` and not `Yanked`.

The next step is a focused `uv lock` integration regression test with one yanked and one unyanked wheel for the same version, followed by a fix that preserves the existing cases where a yanked artifact is explicitly allowed.

## Classification

This is a bug. The repository already labels astral-sh/uv#21430 as `bug`, and the checked-out source independently confirms the correctness gap:

- `crates/uv-resolver/src/version_map.rs` documents that yank state is tracked separately for each file and that unyanked distributions from a partially yanked release can be used. Its wheel and source-distribution compatibility functions return `Yanked` incompatibilities for non-allowed yanked files.
- `PrioritizedDist::built_dist` in `crates/uv-distribution-types/src/prioritized_distribution.rs` removes only entries for which `WheelCompatibility::is_excluded()` is true. That predicate currently recognizes `IncompatibleWheel::ExcludeNewer` only, so a non-selected `IncompatibleWheel::Yanked` remains in the returned wheel list. The analogous source-distribution predicate also recognizes only `ExcludeNewer`.
- `Wheel` in `crates/uv-resolver/src/lock/mod.rs` stores URL, hash, size, upload time, and filename, but no yank status. Deserializing a locked wheel reconstructs its `File` with `yanked: None`. Thus the persisted entry cannot retain the yank information observed during fresh resolution.

This is distinct from the expected behavior of continuing to install an artifact that was valid when it was originally locked and yanked later. Here, fresh lock generation already knows that one file is yanked, has an unyanked alternative for the selected version, and still serializes the yanked file. Existing yank tests cover yanked releases and explicit pins, but the searches found no integration coverage for mixed yank status among files of one version.

The reporter's proposed predicate change identifies the relevant code path, but the exact fix requires care: uv deliberately permits yanked versions in established cases such as an exact `==` request. A regression test should define filtering for a fresh, non-opted-in partial yank without changing those semantics.

## Related

- astral-sh/uv#12296 — **Wheels in lockfile are not filtered by `exclude-newer`** (closed). This is the closest structural analogue: uv respected a per-file eligibility condition during resolution but serialized ineligible wheels into `uv.lock`. Its trigger is upload time rather than yank status.
- astral-sh/uv#12299 — **Omit wheels from lockfile based on `--exclude-newer`** (merged). This fix introduced the `PrioritizedDist::built_dist` filtering path implicated here and added a `uv lock` integration snapshot. Its `is_excluded` predicate intentionally recognizes `ExcludeNewer` only, providing both a direct implementation precedent and the nearest test pattern.
- astral-sh/uv#6145 — **Reduce repeated information across `File` in a single distribution** (open). Its discussion explicitly corrected the assumption that yank state is uniform across all files in a package version and notes that PEP 592 defines per-file yanking. It tracks metadata deduplication and performance, not this lockfile correctness defect.
- astral-sh/uv#5928 — **Resolve from lockfile cannot detect yanked versions** (closed). Maintainers established that resolving from an existing lockfile does not query the registry, and astral-sh/uv#6219 documented that limitation. That issue concerns discovering a yank after lock creation; astral-sh/uv#21430 concerns discarding a yank already known during fresh lock generation.
- astral-sh/uv#413 — **Filter out yanked files** (merged). This historical fix filtered yanked files during the older `pip-compile` resolution path and warned during `pip-sync`. It established the general per-file filtering behavior but did not address universal `uv.lock` artifact aggregation, so the new issue is neither a duplicate nor a confirmed regression of that implementation.

## Search scope and ruled-out candidates

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `"yanked wheel" "uv.lock"`, `per-file yank`, `partially yanked`, `PEP 592`, `data-yanked`, `AllowedYanks`, `is_excluded Yanked`, `yanked lockfile wheel`, and exact lockfile-wheel titles. Conceptual queries covered a private index with mixed yank state, artifact-level eligibility versus version selection, frozen lockfile behavior, and analogous individual-file filtering. Fix-oriented searches covered historical yank filtering and the `exclude-newer` implementation that introduced `is_excluded`.

astral-sh/uv#19112 was inspected but ruled out as a duplicate: it asks uv to query for and reject a whole version that became yanked after it was already locked, while maintainers explain that continuing from an existing lock is intentional. astral-sh/uv#3644 and astral-sh/uv#3425 concern whole-version selection during `uv pip compile`, not retaining one yanked file alongside an unyanked file of the same version. astral-sh/uv#17143 also involves bad artifact data persisted in `uv.lock`, but its mechanism is selecting an unsupported hash algorithm from a private index rather than failing to filter a yanked file.
