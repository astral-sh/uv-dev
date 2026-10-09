# uv_build: optionally respect .gitignore / VCS ignore rules

Issue: astral-sh/uv#22183

Classification: enhancement

Status checked: October 9, 2026. The issue is open; its directly associated implementation proposal, astral-sh/uv#22240, is also open and unmerged.

## Summary

The reporter asks for optional VCS-aware package selection in `uv_build`, or alternatively a warning when ignored files would be packaged. Their example places `debug_dump.json` and `local_notes.md` inside `src/avite/generate/` and lists those paths in `.gitignore`. Those files remain eligible for inclusion unless excluded through the backend configuration. The concern is accidental publication of local, generated, scratch, or sensitive files.

astral-sh/uv#22240 directly proposes the requested opt-in filtering and remains open. astral-sh/uv#19403 and astral-sh/uv#19438 explain the existing defaults; astral-sh/uv#16075 concerns a distinct VCS inclusion feature, and astral-sh/uv#8779 provides design history.

## Requested capabilities

- Opt-in exclusion: use VCS ignore rules when selecting files for both sdists and wheels, with `respect-vcs-ignore = true` suggested as a possible setting name.
- Alternative opt-in warning: leave file selection unchanged but warn when an ignored file would be included, with `warn-vcs-ignored-files = true` suggested.
- Preserve current defaults: the reporter acknowledges generated files and builds outside Git checkouts and explicitly allows an opt-in design.

The report supplies no version-specific regression or exact error message. The proposed setting names are requests, not existing configuration options.

## Draft response

Your requested opt-in filtering is proposed in astral-sh/uv#22240, opened in response to this issue. It adds `tool.uv.build-backend.respect-ignore = true` for `.gitignore` filtering in sdists and wheels. The PR is still open; it does not include a warning-only mode or support global Git excludes.

The current defaults are intentional, as discussed in astral-sh/uv#19403. For now, add the files to `tool.uv.build-backend.source-exclude` to exclude them from both sdists and wheels; `wheel-exclude` alone leaves them eligible for the sdist. You can follow astral-sh/uv#22240 for the proposed option's review and scope.

## Classification

The report requests new opt-in exclusion or warning behavior. Source and maintainer comments confirm the current VCS-independent selection is deliberate; no regression or failure of existing exclusion settings is established. The earlier default-filtering and tracked-file-inclusion discussions have different scopes. astral-sh/uv#22240 was created in response to this issue and therefore does not make it a duplicate.

The implementation proposal was opened on October 6, 2026, after this issue's October 5 opening, and explicitly names this issue in its closing references. Its existence is evidence of proposed implementation, not an earlier canonical request.

The current selection behavior is supported by the implementation and an explicit maintainer decision, rather than inferred solely from a limitation in documentation. The issue does not show explicit exclusions failing. The adjacent requests differ materially: changing default hidden-file filtering and adding tracked repository files to sdists do not provide either requested opt-in mode.

## Related

- astral-sh/uv#22240 — Add opt-in Git ignore filtering to `uv_build` (open; pull request). Direct implementation proposal opened on October 6 in response to astral-sh/uv#22183, opened October 5. Adds disabled-by-default respect-ignore filtering for sdists and wheels. Its patch excludes global Git rules and .git/info/exclude and does not add the alternative warning-only mode. It is unmerged, so it does not establish released support or make this issue a duplicate.
- astral-sh/uv#19403 — uv-build includes .mypy_cache in subdirectories by default (open; issue). Closest earlier report of unwanted files inside the module tree entering distributions. A maintainer explicitly confirmed that avoiding gitignore and limiting default exclusions are intentional choices for predictable builds. That discussion concerns default filtering; this issue requests optional filtering or warnings.
- astral-sh/uv#16075 — Option to include vcs tracked files in source distribution with uv build backend (open; issue). Related opt-in VCS-based file selection request, but it adds all tracked files, including documentation and tests, to sdists. This report instead removes ignored files from otherwise selected sdist and wheel contents, or warns about them; Git tracking and ignore matching are different criteria.
- astral-sh/uv#19438 — fix(build): exclude dotfiles by default to match setuptools (closed; pull request). Unmerged proposal addressing astral-sh/uv#19403 by adding .* to default exclusions. A maintainer rejected matching setuptools defaults. It provides historical context but neither implements optional Git-ignore filtering nor constitutes a previous fix that has regressed.
- astral-sh/uv#8779 — Build backend tracking issue (closed; issue). Reporter-cited design history surveys VCS-ignore behavior in Poetry, Hatchling, and other backends and links the include/exclude implementation. It tracks the overall backend and its stabilization, not this specific opt-in capability.

## Supporting evidence

### Current checkout

- `docs/concepts/build-backend.md:195` describes selection of module contents and configured data, followed by exclusions. Line 204 lists only `__pycache__`, `*.pyc`, and `*.pyo` as default exclusions.
- `crates/uv-build-backend/src/settings.rs:62` documents those defaults. The source-exclude documentation at line 71 explicitly applies source exclusions to wheels to keep direct wheel builds consistent with builds through an sdist. The settings struct contains neither requested option.
- `crates/uv-build-backend/src/metadata.rs:35` defines the same three default patterns.
- `crates/uv-build-backend/src/source_dist.rs:192` combines defaults with `source_exclude`; the traversal at line 290 filters through the include and exclude matchers.
- `crates/uv-build-backend/src/wheel.rs:389` combines defaults, `wheel_exclude`, and `source_exclude`. The module traversal at line 159 uses `WalkDir`, without Git-ignore selection.
- `crates/uv-cli/src/lib.rs:2803` describes `uv build --list`, which can inspect selected contents. Existing integration snapshots in `crates/uv/tests/build/build_backend.rs:1174` exercise `--wheel --list`.

For the sample paths, the current supported mitigation is:

```toml
[tool.uv.build-backend]
source-exclude = [
    "/src/avite/generate/debug_dump.json",
    "/src/avite/generate/local_notes.md",
]
```

These anchored exclusions apply to both distribution types. A wheel-only exclusion does not protect the sdist.

### Maintainer decisions and proposed implementation

In astral-sh/uv#19403 on May 18, 2026, konstin explains that earlier Git-dependent behavior in other backends caused confusing or broken packages, so the backend deliberately includes source-tree data and excludes only the basic Python-generated files by default. The same discussion explains that non-Python data and dot-prefixed files can be legitimate package contents.

The patch and documentation in astral-sh/uv#22240 propose `respect-ignore = true`, disabled by default. It reads project and nested `.gitignore` files, plus ancestor files up to the Git repository root when present. It does not require Git, consult the Git index, use global excludes, or use `.git/info/exclude`. Ignore patterns also apply to tracked files. Required package files matching ignore rules cause an error. There is no warning-only setting in the proposal.

The proposed documentation also calls out a design consideration: ignore rules are reapplied when building a wheel from an sdist, and omitted ignore files can change the result relative to a direct wheel build. This is a limitation of the open proposal, not a reported regression in the checkout. Review comments ask whether the setting might eventually become the default; charliermarsh says that is uncertain and cites astral-sh/uv#19403. No default change or release is established.

Historical merged astral-sh/uv#9525 explains why source exclusions also apply to wheels. Merged astral-sh/uv#9610 introduced manual file-list inspection specifically to help identify unintended temporary files. Merged astral-sh/uv#9523 adds a warning for traversing over 10,000 files, which does not detect ignored-file inclusion.

## Search coverage and ruled-out candidates

Used authenticated gh to search open/closed astral-sh/uv issues and open/closed/merged PRs. Separated optional exclusion from optional warnings; searched gitignore/build, VCS/ignore, respect-vcs-ignore, warn-vcs-ignored-files, respect-gitignore, uv_build/exclude, VCS-tracked files, sensitive files, and packaging warnings, including area:build-backend queries without the example package name. Read candidate bodies/comments, followed astral-sh/uv#8779 to astral-sh/uv#3957 and merged implementation PRs, and followed issue timelines to astral-sh/uv#22240 and astral-sh/uv#19438. Reviewed merged astral-sh/uv#9013, astral-sh/uv#9525, astral-sh/uv#9523, astral-sh/uv#9610, and astral-sh/uv#19991 for existing exclusion/warning support. Ruled out astral-sh/uv#12970 (Hatchling/global Git excludes), astral-sh/uv#8833 (missing Hatchling artifacts), and astral-sh/uv#19990 with astral-sh/uv#19991 (a uv-cache containment guard). Search coverage was limited by REST rate limits and PR-filtered GraphQL searches returning issue nodes; linked PRs were inspected directly, so empty PR search results were not treated as proof of absence.

Additional inspected leads were astral-sh/uv#7938, which predates the uv backend and concerns third-party backend configuration; astral-sh/uv#16438, which asks about independent includes, path remapping, and extensions; and astral-sh/uv#14037, linked from astral-sh/uv#16075, which concerns Git-derived versions rather than file selection.

The strongest apparent safety-related match, astral-sh/uv#12970, was confirmed by its reporter to involve Hatchling ignoring global Git rules. The cache-containment fix in astral-sh/uv#19991 is a frontend check for the uv cache directory, not a general ignored-file warning. Neither establishes a previously fixed uv_build Git-ignore feature.

## Handoff

The next maintainer action is to review the scope of astral-sh/uv#22240, including its relationship to the warning-only alternative and behavior when rebuilding an sdist without all original ignore files. Keep the classification as enhancement; do not close this issue as a duplicate merely because the responding PR exists.

This handoff is based on source, documentation, issue discussions, PR metadata, and the proposed patch. No runtime reproduction or tests were run. No checkout files or GitHub state were changed.
