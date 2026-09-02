# Yanked wheel is retained in `uv.lock` even when a non-yanked wheel exists for the same version

Issue: astral-sh/uv#21430

Classification: bug

## Summary

The reported PEP 592 partial-yank behavior is independently reproducible. With one of two wheels for `demo==1.0.0` marked `data-yanked` in a local PEP 503 index, a fresh `uv lock` selected the unyanked wheel for resolution but wrote both the yanked and unyanked wheel URLs and SHA-256 hashes to `uv.lock`. Reversing which wheel was yanked produced the same result.

No existing issue or pull request tracks this exact partial-yank lockfile defect. The closest precedent is astral-sh/uv#12296 and its fix astral-sh/uv#12299, where an individual wheel excluded by `--exclude-newer` was correctly rejected during resolution but still appeared in the lockfile. astral-sh/uv#6145 records the repository's recognition that yank status is per-file rather than necessarily uniform across a release.

## Classification

Bug. The independent fixture confirms that fresh resolution knows which file is yanked and chooses the other file, while lockfile serialization still retains both files. This is distinct from uv intentionally retaining a previously valid artifact that is yanked only after lock creation.

Source inspection is consistent with the reporter's hypothesis: `version_map.rs` computes file-level yank compatibility, while `PrioritizedDist::built_dist` removes artifacts only when `WheelCompatibility::is_excluded()` returns true, and that predicate recognizes `ExcludeNewer` but not `Yanked`. This inspection supports where to investigate, but the reproduction alone establishes the observed defect; the exact fix still needs to preserve cases where users explicitly opt into a yanked version.

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

No existing test was found that covers mixed yanked and unyanked artifacts for one version. A focused `uv lock` integration regression test should capture this fixture before changing filtering behavior.

## Related

- astral-sh/uv#12296 — **Wheels in lockfile are not filtered by `exclude-newer`** (closed). This is the closest structural analogue: uv respected a per-file eligibility condition during resolution but serialized ineligible wheels into `uv.lock`. Its trigger is upload time rather than yank status.
- astral-sh/uv#12299 — **Omit wheels from lockfile based on `--exclude-newer`** (merged). This fix introduced the `PrioritizedDist::built_dist` filtering path implicated here and added a `uv lock` integration snapshot. Its `is_excluded` predicate intentionally recognizes `ExcludeNewer` only, providing both a direct implementation precedent and the nearest test pattern.
- astral-sh/uv#6145 — **Reduce repeated information across `File` in a single distribution** (open). Its discussion explicitly corrected the assumption that yank state is uniform across all files in a package version and notes that PEP 592 defines per-file yanking. It tracks metadata deduplication and performance, not this lockfile correctness defect.
- astral-sh/uv#5928 — **Resolve from lockfile cannot detect yanked versions** (closed). Maintainers established that resolving from an existing lockfile does not query the registry, and astral-sh/uv#6219 documented that limitation. That issue concerns discovering a yank after lock creation; astral-sh/uv#21430 concerns discarding a yank already known during fresh lock generation.
- astral-sh/uv#413 — **Filter out yanked files** (merged). This historical fix filtered yanked files during the older `pip-compile` resolution path and warned during `pip-sync`. It established the general per-file filtering behavior but did not address universal `uv.lock` artifact aggregation, so the new issue is neither a duplicate nor a confirmed regression of that implementation.

## Search scope and ruled-out candidates

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `"yanked wheel" "uv.lock"`, `per-file yank`, `partially yanked`, `PEP 592`, `data-yanked`, `AllowedYanks`, `is_excluded Yanked`, `yanked lockfile wheel`, and exact lockfile-wheel titles. Conceptual queries covered a private index with mixed yank state, artifact-level eligibility versus version selection, frozen lockfile behavior, and analogous individual-file filtering. Fix-oriented searches covered historical yank filtering and the `exclude-newer` implementation that introduced `is_excluded`.

astral-sh/uv#19112 was inspected but ruled out as a duplicate: it asks uv to query for and reject a whole version that became yanked after it was already locked, while maintainers explain that continuing from an existing lock is intentional. astral-sh/uv#3644 and astral-sh/uv#3425 concern whole-version selection during `uv pip compile`, not retaining one yanked file alongside an unyanked file of the same version. astral-sh/uv#17143 also involves bad artifact data persisted in `uv.lock`, but its mechanism is selecting an unsupported hash algorithm from a private index rather than failing to filter a yanked file.
