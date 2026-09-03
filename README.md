# Yanked wheel is retained in `uv.lock` even when a non-yanked wheel exists for the same version

Issue: astral-sh/uv#21430

Classification: bug

## Summary

The reported PEP 592 partial-yank behavior is independently reproducible. With one of two wheels for `demo==1.0.0` marked `data-yanked` in a local PEP 503 index, a fresh `uv lock` selected the unyanked wheel for resolution but wrote both the yanked and unyanked wheel URLs and SHA-256 hashes to `uv.lock`. Reversing which wheel was yanked produced the same result.

No existing issue or pull request tracks this exact partial-yank lockfile defect. The closest precedent is astral-sh/uv#12296 and its fix astral-sh/uv#12299, where an individual wheel excluded by `--exclude-newer` was correctly rejected during resolution but still appeared in the lockfile. astral-sh/uv#6145 records the repository's recognition that yank status is per-file rather than necessarily uniform across a release.

A follow-up comment adds two adjacent requests: warn or otherwise diagnose yank-related outcomes during `--locked` checks, and explain when `--upgrade` moves a package to an older unyanked version. These do not change the reproduced bug or its fixed status; they concern diagnostics and CLI wording beyond the artifact-filtering fix.

A maintainer subsequently questioned whether the lockfile-content behavior should be classified as a bug because uv does not currently promise specific `uv.lock` contents, while agreeing that the behavior could be improved. This introduces classification uncertainty but does not dispute the reproduction or the proposed improvement.

## Classification

Bug. The independent fixture confirms that fresh resolution knows which file is yanked and chooses the other file, while lockfile serialization still retains both files. This is distinct from uv intentionally retaining a previously valid artifact that is yanked only after lock creation.

Source inspection is consistent with the reporter's hypothesis: `version_map.rs` computes file-level yank compatibility, while `PrioritizedDist::built_dist` removes artifacts only when `WheelCompatibility::is_excluded()` returns true, and that predicate recognizes `ExcludeNewer` but not `Yanked`. This inspection supports where to investigate, but the reproduction alone establishes the observed defect; the exact fix still needs to preserve cases where users explicitly opt into a yanked version.

Maintainer feedback leaves the label open to reconsideration. The maintainer's stated reason is that uv provides no known guarantee about which artifacts appear in `uv.lock`, so they may view omission of the yanked file as an improvement rather than correction of a contractual violation. The existing handoff retains `bug` because the behavior is reproducible, uv already applies the same artifact-level omission for `--exclude-newer`, and retaining a known-ineligible artifact can affect later lockfile consumers. However, the maintainer comment is the current project-position signal and should be resolved before merging or presenting the change as a required correctness fix.

## Reproduction

Outcome: **reproducible**.

Environment:

- uv 0.12.9 (`x86_64-unknown-linux-gnu`), newer than the reported 0.12.4
- Linux x86_64
- CPython 3.12.3, with project `requires-python = ">=3.12"`
- Local unauthenticated HTTP PEP 503 index; no devpi-specific behavior was required

The temporary index exposed two minimal valid wheels for the same name and version:

```html
<a href="../../files/demo-1.0.0-1-py3-none-any.whl#sha256=e1b7517e..." data-yanked="broken build 1">demo-1.0.0-1-py3-none-any.whl</a>
<a href="../../files/demo-1.0.0-2-py3-none-any.whl#sha256=e1b7517e...">demo-1.0.0-2-py3-none-any.whl</a>
```

The project configuration was:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["demo>=1.0.0"]

[[tool.uv.index]]
name = "local"
url = "http://127.0.0.1:38889/simple"
default = true
```

After serving the index locally, the targeted command was run with a fresh temporary cache and no existing lockfile:

```console
env -u UV_LOCKED UV_CACHE_DIR=/tmp/uv-issue-21430-repro/cache-a uv lock --refresh --verbose
```

uv's verbose output explicitly selected the unyanked artifact:

```text
Selecting: demo==1.0.0 [compatible] (demo-1.0.0-2-py3-none-any.whl)
```

However, the new `uv.lock` contained both artifacts, including the yanked build 1, with their complete URLs and hashes:

```toml
wheels = [
    { url = "http://127.0.0.1:38889/files/demo-1.0.0-1-py3-none-any.whl", hash = "sha256:e1b7517e38028749d8892d124dff910ab75c67a3d81014a564e31fc148c1e7a1" },
    { url = "http://127.0.0.1:38889/files/demo-1.0.0-2-py3-none-any.whl", hash = "sha256:e1b7517e38028749d8892d124dff910ab75c67a3d81014a564e31fc148c1e7a1" },
]
```

The `data-yanked` attribute was then moved from build 1 to build 2, the lockfile was regenerated with another empty cache, and verbose output selected build 1. The regenerated lockfile again contained both build 1 and yanked build 2. This confirms the reported symmetry.

Existing coverage checked:

- `crates/uv/tests/lock/lock.rs::lock_project_with_scoped_override_yank` covers an explicitly opted-in yanked release and its warning, but does not create mixed yank states among files of one version or assert artifact filtering in `uv.lock`.
- `crates/uv/tests/pip/pip_install_scenarios.rs::package_yanked_specified_mixed_available` covers choosing an unyanked version over yanked versions, not a partially yanked set of files for one version and not lockfile contents.
- `crates/uv/tests/lock/lock.rs::lock_omit_wheels_exclude_newer` verifies that individual wheels excluded by upload time are omitted from `uv.lock`; it is the nearest lockfile artifact-filtering precedent but does not exercise yank metadata.

At reproduction time, no existing test covered mixed yanked and unyanked artifacts for one version. The parent regression described below now provides that coverage.

## Fix

Outcome: fixed in the checkout.

The reproduction and implementation confirm that the defect was in artifact aggregation after candidate selection. `version_map.rs` correctly assigns `IncompatibleWheel::Yanked` or `IncompatibleSource::Yanked` to a non-allowed yanked file, but `PrioritizedDist` treated only `ExcludeNewer` as an excluded artifact when constructing the registry distribution later serialized into `uv.lock`. As a result, candidate selection could use an unyanked file while the aggregate still carried yanked wheels or source distributions.

`crates/uv-distribution-types/src/prioritized_distribution.rs` now treats incompatible yanked wheels and source distributions as excluded alongside `ExcludeNewer`. Both aggregation directions apply that predicate: a wheel-selected `RegistryBuiltDist` filters its wheel list and optional source distribution, and a source-selected `RegistrySourceDist` filters its attached wheel list. Explicitly allowed yanks retain their existing behavior because `version_map.rs` classifies them as compatible before this filter is applied.

The parent integration regression `crates/uv/tests/lock/lock.rs::lock_partially_yanked_release` now asserts the desired lockfile instead of snapshotting the defect. Its fixture covers a yanked wheel and yanked source distribution beside a usable wheel, plus a source-selected release whose yanked wheel must not be attached. The resulting lockfile retains the usable artifacts and omits every non-allowed yanked artifact. The test also validates an offline `uv lock --locked` round trip against the generated lockfile.

Successful focused validation:

- `cargo test --package uv --test lock --features test-universal lock_partially_yanked_release`
- `cargo test --package uv --test lock --features test-universal lock_project_with_scoped_override_yank`
- `cargo test --package uv --test lock --features test-universal lock_omit_wheels_exclude_newer`
- `cargo test --package uv --test pip_compile --features test-universal omit_wheels_exclude_newer`
- `cargo test --package uv-distribution-types`
- `cargo +stable clippy --package uv-distribution-types --all-targets -- -D warnings`
- `cargo +stable fmt --all -- --check`
- `git diff --check`

The active Rust 1.98 toolchain did not have rustfmt or clippy installed, and its read-only toolchain location prevented adding those components. The installed stable rustfmt and clippy were therefore used for the successful formatting and lint checks.

## Follow-up diagnostic and documentation requests

The reporter subsequently described two silent outcomes and proposed clarifying the `--locked` help text:

- `uv sync --locked` does not warn when an artifact represented by an existing lock is now yanked upstream, or when the pre-fix lock carries an unused yanked wheel.
- `uv lock --upgrade` can select an older unyanked version without explaining that a newer version was skipped because it was yanked.
- The current help text says `--locked` will "Assert that the `uv.lock` will remain unchanged"; the reporter suggests explicitly scoping that assertion to the current project definition.

Source inspection supports the narrow diagnostic observation. `ResolutionDiagnostic::YankedVersion` is populated in `crates/uv-resolver/src/resolution/output.rs` only from `ResolvedDist::yanked()`, and `ResolvedDist::yanked()` in `crates/uv-distribution-types/src/resolved.rs` checks the selected source distribution or `best_wheel()`. It does not report rejected yanked candidates. A wheel reconstructed from `uv.lock` also has no yank metadata, so detecting a yank applied after lock creation would require obtaining current registry state through some separate check.

The `--locked` implementation is narrower than a general registry-health guarantee: `LockMode::Locked` runs the lock operation without writing and errors if the resulting lock would change, while its source comment defines it as checking whether the lockfile is up to date with project requirements. The CLI help already follows its one-line assertion with "Requires that the lockfile is up-to-date" and errors if it is missing or needs updating. Whether the first line should additionally say "for the current project definition" is a documentation decision, not part of the fixed partial-yank serialization bug.

The follow-up requests do not share the same immediate root cause as the lockfile defect. The completed fix filters known, non-allowed yanked artifacts during registry-distribution aggregation. Reporting rejected candidates would require new diagnostic plumbing, while reporting a yank applied after lock creation may require registry access. The existing discussions below should guide whether these behaviors belong in this issue or separate follow-ups.

## Related

- astral-sh/uv#12296 — **Wheels in lockfile are not filtered by `exclude-newer`** (closed). This is the closest structural analogue: uv respected a per-file eligibility condition during resolution but serialized ineligible wheels into `uv.lock`. Its trigger is upload time rather than yank status.
- astral-sh/uv#12299 — **Omit wheels from lockfile based on `--exclude-newer`** (merged). This fix introduced the `PrioritizedDist::built_dist` filtering path implicated here and added a `uv lock` integration snapshot. Its `is_excluded` predicate intentionally recognizes `ExcludeNewer` only, providing both a direct implementation precedent and the nearest test pattern.
- astral-sh/uv#6145 — **Reduce repeated information across `File` in a single distribution** (open). Its discussion explicitly corrected the assumption that yank state is uniform across all files in a package version and notes that PEP 592 defines per-file yanking. It tracks metadata deduplication and performance, not this lockfile correctness defect.
- astral-sh/uv#5928 — **Resolve from lockfile cannot detect yanked versions** (closed). Maintainers established that resolving from an existing lockfile does not query the registry, and astral-sh/uv#6219 documented that limitation. That issue concerns discovering a yank after lock creation; astral-sh/uv#21430 concerns discarding a yank already known during fresh lock generation.
- astral-sh/uv#413 — **Filter out yanked files** (merged). This historical fix filtered yanked files during the older `pip-compile` resolution path and warned during `pip-sync`. It established the general per-file filtering behavior but did not address universal `uv.lock` artifact aggregation, so the new issue is neither a duplicate nor a confirmed regression of that implementation.
- astral-sh/uv#19112 — **Prevent installing yanked packages** (open). This directly overlaps the follow-up request for warnings or rejection when an already-locked artifact is later yanked. Maintainers state that continued installation from an existing lock is intentional under the packaging specification and suggest `uv audit` or a future diagnostic interface for current yank checks.
- astral-sh/uv#3644 — **`pip compile --upgrade` behavior with yanked packages** (open). This is the closest precedent for the follow-up's silent-downgrade concern. Maintainers favored selecting the older unyanked version with a warning; a later comment confirmed that the downgrade currently occurs without one. It covers `uv pip compile --upgrade`, not `uv lock --upgrade`.

## Search scope and ruled-out candidates

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `"yanked wheel" "uv.lock"`, `per-file yank`, `partially yanked`, `PEP 592`, `data-yanked`, `AllowedYanks`, `is_excluded Yanked`, `yanked lockfile wheel`, and exact lockfile-wheel titles. Conceptual queries covered a private index with mixed yank state, artifact-level eligibility versus version selection, frozen lockfile behavior, and analogous individual-file filtering. Fix-oriented searches covered historical yank filtering and the `exclude-newer` implementation that introduced `is_excluded`.

astral-sh/uv#19112 and astral-sh/uv#3644 were not duplicates of the original serialization report, but the follow-up comment makes them directly relevant to the newly requested diagnostics. astral-sh/uv#3425 concerns whole-version selection during `uv pip compile`, not retaining one yanked file alongside an unyanked file of the same version. astral-sh/uv#17143 also involves bad artifact data persisted in `uv.lock`, but its mechanism is selecting an unsupported hash algorithm from a private index rather than failing to filter a yanked file.

Pull request: astral-sh/uv-dev#963
