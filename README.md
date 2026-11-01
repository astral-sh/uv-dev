# universal-lock: dependency without wheels for a matrix environment fails the whole resolution (marked per-dependency override works but undocumented)

Issue: astral-sh/uv#23038

Classification: duplicate

## Summary

Exact duplicate of astral-sh/uv#23037. Related discussions explain marked overrides and wheel-based platform forking; an application-side PR documents a closely matching workspace failure.

The reporter describes a workspace containing hermes-agent and a mnemosyne plugin, using uv 0.12.3 and 0.13.5 with `[tool.uv] environments = ["python_version >= '3.14'"]`. An unmarked transitive requirement on sqlite-vec 0.1.9 allegedly aborts the entire lock because some environments lack wheels. The supplied error says there are no `python_full_version >= '3.14'`-compatible wheels and recommends limiting `tool.uv.environments`. The full failing split and reproducible workspace are absent.

The reported workaround supplies Linux/x86_64-marked overrides for onnxruntime and sqlite-vec. The requested improvements are best-effort dependency omission in unsupported environments, automatic availability markers inferred from wheels, and clearer documentation or hints for per-dependency overrides.

## Draft response

This duplicates astral-sh/uv#23037; please continue there so we can keep the investigation together.

Marked `override-dependencies` can restrict transitive dependencies to selected platforms, as explained in astral-sh/uv#17642. The dependency-overrides documentation also describes marker replacement. To investigate the lock failure, please add a minimal workspace configuration and the complete resolver error, including the failing split, to astral-sh/uv#23037. The excerpt alone does not establish which platform lacks coverage.

## Classification

astral-sh/uv#23037 already contains the identical report and both requested capabilities, so discussion can be centralized there. This classification does not assume the proposed missing-wheel mechanism is correct. No regression of a historical fix is established; the same current report already has an open issue.

The two issues were opened on 2026-11-01 at 15:21:30 UTC and 15:21:47 UTC respectively. Their live bodies and titles are identical and both remain open without comments. This is a direct duplicate irrespective of the eventual diagnosis or whether the canonical issue is categorized as a bug, enhancement, or question.

## Related

- astral-sh/uv#23037 — universal-lock: dependency without wheels for a matrix environment fails the whole resolution (marked per-dependency override works but undocumented) (open; issue). Exact duplicate: identical title, body, reported uv versions, workspace, error, marked overrides, and requested capabilities. Opened 17 seconds before this issue; neither thread has comments.
- NousResearch/hermes-agent#144162 — fix(pm): a plugin workspace stops resolving the Android split on desktop hosts (closed; pull request). Same hermes-agent/mnemosyne workspace and uv 0.13.5. Supplies a full onnxruntime failure in an Android resolver split and proposes excluding Android from generated desktop workspaces. Closed without merging. Strong application-side context, but it does not establish the cause of the reported sqlite-vec failure.
- astral-sh/uv#17642 — Add platform dependence to `exclude-dependencies` (closed; issue). Maintainer explicitly explains that a marked override can omit a transitive dependency on an excluded platform. Directly supports the reported workaround; the original example concerns GDAL supplied outside uv, rather than missing wheels during universal locking.
- astral-sh/uv#9711 — Tracking issue: package without source distribution gets an "incomplete" resolution (open; issue). Canonical broader discussion of inferring platform coverage from available wheels and forking/backtracking accordingly. Its symptom is successful locking followed by installation failure, unlike this report's lock-time failure; it is related to the requested automatic availability handling.
- astral-sh/uv#9928 — Fork when wheels are missing platform support (closed; pull request). Attempted general wheel-platform-based forking for astral-sh/uv#9711. Closed without merging in favor of astral-sh/uv#10046, whose merged implementation only falls back from local versions to their base versions. Neither implements silently omitting unavailable dependencies.

## Supporting evidence

### Override behavior and documentation

The checkout's `docs/concepts/resolution.md:445` explains dependency overrides; at line 499 it explicitly describes markers and unconditional replacement of a dependency's existing marker. The capability is therefore documented in general terms, although this section does not give the report's specific missing-wheel workaround.

`crates/uv-configuration/src/overrides.rs:352` implements requirement replacement. Its comments and code show that overrides replace environmental conditions while retaining optional-dependency extra conditions. This supports the semantics of the workaround; it does not prove that omitting those dependencies leaves the application functional on excluded platforms.

In astral-sh/uv#17642, a maintainer gives the concrete example `override-dependencies = ["gdal ; sys_platform != 'win32'"]` and explains that any dependency on GDAL is overridden to omit it on Windows.

The supplied Linux/x86_64 marker does not distinguish glibc from musl. `docs/concepts/resolution.md:241` explicitly states that environment markers do not include libc implementation or version. The workaround is not evidence that musl was the failing environment.

### Wheel coverage and resolver splits

`docs/concepts/resolution.md:191` says that, by default, a package without a source distribution needs at least one wheel compatible with the target Python version. It does not require wheels for every possible platform.

The report explicitly configures `environments`, which matters: `crates/uv-lock-operations/src/lock.rs:639` combines supported and required environments into artifact-coverage environments. `crates/uv-resolver/src/resolver/mod.rs:1627` checks that coverage within the current resolver fork and can reject a candidate with `MissingPlatform`. Thus the default rule alone cannot dismiss this report, and the abbreviated error alone cannot identify its cause.

Merged astral-sh/uv#21672 scoped these checks to the current fork, allowing backtracking or an unsupported-environment error. It merged on 2026-09-14. The reporter also claims the behavior on uv 0.12.3, so this change cannot be assigned as the sole cause from the available report.

Following the references on astral-sh/uv#22805 led to NousResearch/hermes-agent#144162. That closed, unmerged application PR describes the same plugin workspace, uv 0.13.5, and a full Android split error for onnxruntime; it says the core project's Android markers introduce that split and proposes narrowing generated desktop workspaces. This is useful context, not independent verification of the sqlite-vec diagnosis.

### Historical fixes and candidates not treated as duplicates

astral-sh/uv#22805 closely resembles the reported error and Python environment setting, but its demonstrated bug was failure to recognize Android wheels that actually existed. Merged astral-sh/uv#22806 added Android platform-marker inference and fixed both reproductions. `CHANGELOG.md:49` places that fix in uv 0.13.5, released on 2026-10-21. No evidence here shows that fix regressed; failure when a dependency has no Android wheel is a different condition.

astral-sh/uv#8492 concerned selecting an old scientific-package version across Python versions. Its closing fix, merged astral-sh/uv#9827, forks on dependency `requires-python` lower bounds. That does not implement optional omission or general platform availability inference.

astral-sh/uv#5921 concerns preferring current-platform distributions when fetching metadata during universal resolution. Maintainer discussion emphasizes deterministic resolution and metadata-fetch overhead; it does not track the reported whole-lock failure.

astral-sh/uv#11664 concerned Windows architecture-marker case handling. astral-sh/uv#21820 successfully created a lock but selected an onnxruntime distribution unusable with Python 3.10. Neither matches the supplied Python 3.14+ lock-time failure.

The broader wheel-availability proposal in astral-sh/uv#9928 was not merged. Maintainer comments redirect to merged astral-sh/uv#10046, explicitly limited to considering a non-local version as an alternative to a local version, primarily for PyTorch. This is not a previously shipped implementation of best-effort dependency omission.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered sqlite-vec, onnxruntime, compatible wheels, python_full_version, and override-dependencies; conceptual searches covered universal wheel availability, optional/best-effort resolution, automatic markers, and platform-specific overrides/documentation. Inspected candidate comments and followed references to closed proposals, merged fixes, and NousResearch/hermes-agent#144162. PR keyword searches returned no results; several REST searches were rate-limited, so GraphQL searches and direct linked-PR inspection supplemented them. Reviewed astral-sh/uv#21672, astral-sh/uv#10046, and astral-sh/uv#9827 for historical fixes. Ruled out astral-sh/uv#22805 as an established duplicate: it rejected existing Android wheels and was fixed by astral-sh/uv#22806 in uv 0.13.5; this report alleges absent wheels and supplies no complete failing split. Also ruled out astral-sh/uv#5921, which concerns metadata-fetch preference, and astral-sh/uv#8492, whose Python-version selection issue was fixed by astral-sh/uv#9827.

The searches kept the reported lock-time symptom separate from hypotheses about platform coverage. Claim-specific queries covered the quoted error fragments and package identifiers, optional or best-effort resolution, automatic wheel-derived markers, and override-marker documentation. Searches without package names or versions identified the broader resolver and configuration discussions.

## Maintainer next step

Centralize discussion in astral-sh/uv#23037. Request the smallest root/member configuration, relevant index settings, exact command, and complete resolver error with the failing split there. That information would distinguish missing wheel coverage in an explicit fork from an incorrect compatibility decision and would clarify whether the two claimed uv versions fail in the same way.

This handoff is based on repository documentation, source inspection, and read-only GitHub issue/PR review. The reported workspace was not reproduced and no tests were run. No GitHub changes were made.
