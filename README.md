# universal-lock: dependency without wheels for a matrix environment fails the whole resolution (marked per-dependency override works but undocumented)

Issue: astral-sh/uv#23037

Classification: duplicate

## Summary

The reporter describes a workspace containing hermes-agent and the mnemosyne plugin, using uv 0.12.3/0.13.5 and `[tool.uv] environments = ["python_version >= '3.14'"]`. An unmarked transitive dependency, sqlite-vec 0.1.9, reportedly prevents the entire workspace from locking. The excerpt says it has no `python_full_version >= '3.14'`-compatible wheels and suggests limiting `tool.uv.environments`.

The reporter attributes this to platforms without wheels, such as musl Linux and Windows ARM64. They report success after adding Linux x86_64 markers to overrides for sqlite-vec and onnxruntime. They request either automatic omission of unavailable dependencies, inferred availability markers, or clearer guidance for per-dependency overrides.

astral-sh/uv#23038 is an exact duplicate. astral-sh/uv#17642 confirms conditional override semantics, astral-sh/uv#14974 shows a similar marker-based resolution, and astral-sh/uv#21672 explains relevant branch-specific wheel checks.

The exact failing resolver branch and package metadata are not supplied. The general override semantics are confirmed, but the proposed explanation for this particular failure is not.

## Draft response

astral-sh/uv#23038 contains the identical report. Since astral-sh/uv#23037 was opened first, please keep follow-up details here so the reports can be consolidated.

Your marked overrides are supported in the workspace root's `tool.uv.override-dependencies`. They replace dependency environment markers, as explained in astral-sh/uv#17642 and https://docs.astral.sh/uv/concepts/resolution/#dependency-overrides. They explicitly omit those dependencies outside the selected platforms; wheel availability alone does not establish that the parent package works without them.

Universal locking does not generally require wheels for every platform, but configured `environments` participate in artifact checks within each resolver branch. To understand this failure, please provide a minimal root/member `pyproject.toml` pair, the exact command, and the complete error including the `split (markers: ...)` line, with the overrides removed.

## Classification

The open astral-sh/uv#23038 repeats this report exactly, so duplicate classification takes precedence. Because astral-sh/uv#23037 was opened first, consolidate the later copy into this discussion. Marked overrides are supported; the abbreviated error does not establish a regression of a previously fixed bug.

The identical reports were opened on 2026-11-01 at 15:21:30 UTC (astral-sh/uv#23037) and 15:21:47 UTC (astral-sh/uv#23038). Neither has comments. The duplicate classification identifies the repeated report; the recommended handling is to retain the earlier issue and consolidate the later copy. No GitHub changes were made.

Automatic omission of a declared dependency would change the dependency graph. The existing override mechanism explicitly authorizes that change; a missing wheel does not itself indicate that the dependency is unnecessary. Without the duplicate, the report would still require separating its requested functionality/documentation improvements from a potentially incorrect resolution failure.

## Related

- astral-sh/uv#23038 — universal-lock: dependency without wheels for a matrix environment fails the whole resolution (marked per-dependency override works but undocumented) (open; issue). Identical title and body, including uv versions, workspace configuration, wheel error, marked overrides, and both proposed capabilities. Opened 17 seconds after astral-sh/uv#23037; neither has comments. Keep the earlier report as the discussion thread.
- astral-sh/uv#17642 — Add platform dependence to `exclude-dependencies` (closed; issue). Direct precedent for the per-dependency workaround: a maintainer explains that a marked override replaces incoming requirements and omits the dependency where its marker is false. That report concerns GDAL supplied externally on Windows, rather than automatic omission based on wheel availability.
- astral-sh/uv#14974 — `required-environments`: only require that some extras build in some environments (closed; issue). An optional dependency lacking wheels for another requested platform caused resolution failure with the same environment-limiting hint. Adding platform markers to the dependency resolved it. Unlike this report, it used extra-qualified required-environments and direct optional dependencies.
- astral-sh/uv#21672 — Scope required-environment checks to the current fork (merged; pull request). Makes artifact compatibility checks specific to the current resolver branch, allowing backtracking or failure when no compatible release exists. Source confirms configured environments also enter these checks. Relevant mechanism, not a confirmed cause or fix: the report also names uv 0.12.3, predating this change.

## Supporting evidence

- `docs/concepts/resolution.md:175` explains that wheel-only packages normally need a wheel compatible with the target Python version, while required environments add platform coverage requirements. Universal locking does not inherently require a wheel for every possible platform.
- `crates/uv-lock-operations/src/lock.rs:638` combines supported `environments` and `required-environments` into the artifact environments passed to the resolver. Therefore the reporter's explicitly configured Python-only environment is relevant even without a `required-environments` setting.
- `crates/uv-resolver/src/resolver/mod.rs:1627` checks artifact coverage and dependency relevance within the current fork. A missing match can produce an unavailable candidate or a new fork; it does not silently drop the dependency.
- `crates/uv-resolver/src/prioritized_distribution.rs:314` formats `MissingPlatform(marker)` using the supplied environment marker. A message mentioning only a Python range does not by itself identify the platform split responsible for failure.
- `docs/concepts/resolution.md:445` documents dependency replacement and marker behavior. `crates/uv-configuration/src/overrides.rs:352` replaces environmental conditions while retaining optional-extra conditions. This confirms that marked overrides are supported behavior, although the documentation could make the platform-omission use case more explicit.
- `crates/uv-workspace/src/pyproject.rs:552` specifies that project commands read overrides only from the workspace root's `pyproject.toml`.
- In astral-sh/uv#17642, charliermarsh's 2026-04-13 comment explicitly explains that `override-dependencies = ["gdal ; sys_platform != 'win32'"]` means any requirement for GDAL is replaced so GDAL is omitted on Windows.
- In astral-sh/uv#14974, zanieb recommends putting platform markers on the optional dependency itself; the reporter confirms that this resolves the failure.
- `crates/uv/tests/lock/lock.rs:44086`, `lock_required_environment_python_fork`, covers branch-specific wheel checks and backtracking. It is gated by `test-universal`. This test was inspected, not executed.

## Historical fixes and candidates ruled out

astral-sh/uv#22805 has a particularly similar error and Python-only `environments` configuration. It concerns Android wheels that actually exist but were not recognized by inferred platform markers. Its second reproduction started failing after astral-sh/uv#21672, reported as uv 0.12.14. The fixing PR, astral-sh/uv#22806, adds Android platform-marker inference and was merged on 2026-10-21. `CHANGELOG.md:49` lists it in uv 0.13.5, which the new reporter already names. No evidence establishes that Android recognition has regressed, and the new report also names uv 0.12.3, before the branch-scoping change.

astral-sh/uv#8492 and its merged fix astral-sh/uv#9827 concern selecting different package versions across Python ranges based on `Requires-Python`. They do not implement silently omitting an unavailable dependency on a platform.

astral-sh/uv#11153 concerns enabling overrides only for a selected extra, a different capability from a platform marker on an override. astral-sh/uv#9803 concerns conflicting direct wheel URLs with disjoint markers. Neither is a canonical duplicate of this report.

astral-sh/uv#9174 points to astral-sh/uv#4422 for dependency elimination. Their comments confirm replacement semantics and discuss unconditional exclusions, but astral-sh/uv#17642 is the closer precedent for conditional platform omission.

astral-sh/uv#22913 discusses excluding musl support or artifacts and minimum-libc configuration; it does not establish that this workspace failed because a musl wheel was absent. astral-sh/uv#17382 discusses documented limits in mapping wheel platforms to environment markers, rather than dependency-override discoverability. astral-sh/uv#16589 and astral-sh/uv#11664 involve Windows architecture-marker spelling/casing, which is not established here.

## Search scope and limitations

Used authenticated gh to search open and closed issues with literal sqlite-vec, compatible-wheels errors, the full non-current-environment hint, and override-dependencies/markers. Separately searched conceptual optional/platform wheels, inferred or derived markers, best-effort resolution, skipping unavailable dependencies, and per-package environments. Searched open, closed, merged, and all-state PRs for wheel/universal, override/marker, required-environments, and compatible wheels; dedicated PR searches returned no matches, so inspected referenced merged PRs directly. Followed astral-sh/uv#9174 to astral-sh/uv#4422 and astral-sh/uv#8492 to astral-sh/uv#9827. Ruled out astral-sh/uv#22805 and astral-sh/uv#22806 as an established duplicate/regression: they concern existing Android wheels being unrecognized, fixed in uv 0.13.5, whereas this report describes absent wheels and includes that release. Also distinguished extra-scoped overrides in astral-sh/uv#11153 and conflicting override URLs in astral-sh/uv#9803. REST searches eventually hit an installation rate limit; GraphQL searches, issue/PR reads, local source, tests, and release notes supplemented the results.

Before searching, the report was separated into three claims: universal lock failure for unavailable artifacts; automatic per-environment omission or inferred markers; and support/discoverability of consumer-side marked overrides. Literal symptom searches were kept separate from possible-mechanism searches. Repository vocabulary from labels and maintainer discussions included universal resolution, required environments, forks, overrides, exclusions, and platform markers.

The report does not include complete root/member project files, the exact command, or the full split header. No reproduction, build, or test was run. The next useful evidence is that minimal configuration and full diagnostic without the workaround; it would identify whether an unsupported branch, an artifact-tag interpretation error, or another condition is involved.

Only this handoff was written outside the checkout. No repository files or GitHub resources were changed.
