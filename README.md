# Dynamic version runs the build backend for a project with `package = false`

Issue: astral-sh/uv#22932

Classification: duplicate

## Summary

astral-sh/uv#9390 and astral-sh/uv#12467 report the same behavior. astral-sh/uv#10622 and astral-sh/uv#10703 explain version omission and successful lockfile reuse, but do not eliminate fresh-resolution backend calls.

The reporter uses uv 0.13.5 (Homebrew, October 22, 2026), Python 3.13.13, and macOS 27.0 arm64. Their application declares `dynamic = ["version"]`, `dependencies = []`, `requires-python = ">=3.13"`, and `[tool.uv] package = false`, with no build system. A fresh `uv lock` installs setuptools and calls `setuptools.build_meta:__legacy__.get_requires_for_build_wheel`. Two top-level packages, `foo` and `bar`, make setuptools reject the layout with `Multiple top-level packages discovered in a flat-layout`.

Without those directories, the reporter says locking succeeds but still invokes the backend. The resulting root entry has `source = { virtual = "." }` and no version. With a reusable lockfile, `uv lock --locked`, `uv sync --locked`, and `uv run` work; `uv lock --upgrade`, `uv lock --upgrade-package <name>`, and `uv add` trigger resolution and fail again. The requested behavior is to avoid acquiring the unused dynamic version for this non-package application.

## Draft response

Your reproduction matches astral-sh/uv#9390, including the flat-layout error with `package = false` and a dynamic version. Although that issue was closed as fixed, astral-sh/uv#10703 only avoids backend calls when validating an existing lockfile; it does not cover a fresh resolve. The next step is to reopen astral-sh/uv#9390 for that remaining case and consolidate this reproduction there. For now, replacing `dynamic = ["version"]` with a static `version = "0.0.0"` avoids this metadata lookup.

## Classification

astral-sh/uv#9390 already reports the same configuration, command failure, and setuptools error. The new report adds the useful distinction between lockfile reuse and fresh resolution. Although the earlier issue was closed as fixed, astral-sh/uv#10703 only changes validation, and the same fresh-resolution metadata fallback is present in uv 0.9.17 and the current checkout. The evidence supports an incompletely addressed case, not a demonstrated regression; discussion can be centralized by reopening astral-sh/uv#9390.

This is a correctness complaint about backend execution for a non-package project, rather than merely a request to explain packaging standards. Duplicate takes precedence because the precise behavior is already documented. The December 10, 2025 closure comment on astral-sh/uv#9390 says the behavior was fixed, but does not identify a fixing change. That statement alone does not establish that fresh resolution ever worked. The source from uv 0.9.17, released the preceding day, retains the relevant dynamic-version fallback.

astral-sh/uv#7930 is open and closely related, but its original input lacks both a version and a dynamic declaration. astral-sh/uv#9912 subsequently added early validation for that case. The explicit dynamic declaration in the new report passes this validation, so astral-sh/uv#9390 is the more precise place to consolidate discussion.

## Related

- astral-sh/uv#9390 — Doc: clarify "virtual projects" static version required[/-] (issue, closed). Closest match: the same dynamic = ["version"], package = false, uv lock failure, and setuptools flat-layout discovery error; a static version avoided it. Closed as fixed in December 2025, but the inspected fixes cover lockfile validation, and the fresh-resolution fallback also exists in the source from that time.
- astral-sh/uv#12467 — egg-info directory keep being created (issue, closed). Reports backend execution during uv add/remove with package = false and a dynamic version, including the same flat-layout error. A maintainer explains the fallback to setuptools for dynamic metadata; the reporter adopts a static version. This corroborates the unresolved fresh-resolution behavior after the January 2025 validation fix.
- astral-sh/uv#7930 — Error when creating a lock file with disabled package management mode (issue, open). Broader predecessor: uv lock invokes setuptools despite package = false because no static version is available, and a maintainer acknowledges the problem. Its reproduction omits both version and a dynamic declaration; that invalid-input case now receives an early parser error, making astral-sh/uv#9390 the closer discussion.
- astral-sh/uv#10689 — uv lock should not run build backend for dynamic versions (issue, closed). Reporter-cited history: backend calls persisted after dynamic versions were omitted from uv.lock. Closed by astral-sh/uv#10703, whose implementation addresses validating an existing lockfile, not a fresh resolve.
- astral-sh/uv#10703 — Avoid building dynamic versions when validating lockfile (pull request, merged). Merged January 17, 2025 and released in uv 0.5.21. Extracts static requirements during lockfile validation; its regression test supplies an existing lockfile and runs --locked --offline. Explains why lockfile reuse works without fixing fresh-resolution backend calls.
- astral-sh/uv#10622 — Omit dynamic versions from the lockfile (pull request, merged). Merged January 15, 2025. Omits versions for dynamically versioned source trees. Its discussion explicitly acknowledges that backend calls still occur, establishing that omission from uv.lock does not itself remove metadata acquisition.

## Supporting evidence

- In `crates/uv-pypi-types/src/metadata/metadata_resolver.rs:175`, `ResolutionMetadata::parse_pyproject_toml` rejects a dynamic version when no sdist version is available. This branch also exists in the uv 0.9.17 source.
- In `crates/uv-distribution/src/source/mod.rs:1507`, `source_tree_metadata` reads static metadata, tries cached metadata, and then calls the backend when necessary. `StaticMetadata::read` at line 3316 classifies the dynamic-version case without providing complete metadata. The analogous uv 0.9.17 path has the same fallback.
- In `crates/uv-lock/src/lock/mod.rs:4848`, existing-lock validation explicitly avoids acquiring a dynamic version when static requirements suffice. This supports the reported difference between reuse and fresh resolution.
- In `crates/uv-lock/src/lock/mod.rs:7624`, `PackageId::from_annotated_dist` omits versions for dynamically versioned source trees. Omission happens when constructing lockfile identities; it does not mean the resolver never requested a version.
- `crates/uv/tests/lock/lock.rs:34632` contains `lock_dynamic_version_no_build`, gated by `test-universal`. It prewrites a lockfile and checks validation with `--locked --offline`. This existing coverage does not establish backend-free fresh resolution for a virtual project.
- `docs/concepts/projects/config.md:158` says `package = false` prevents building and installing the project into its environment. The new report supplies a concrete failure caused by the metadata acquisition path.
- astral-sh/uv#10703 merged January 17, 2025 and appears in the uv 0.5.21 release notes as avoiding builds during lockfile validation. astral-sh/uv#12467 reports the dynamic-version/flat-layout behavior with uv 0.6.3 after that release.
- astral-sh/uv#17827 proposed documentation for this exact limitation in February 2026, but was closed without merging; it is not evidence of a behavioral fix.

These findings come from the report, repository source, existing tests, release history, and maintainer discussions. No runtime reproduction or test execution was performed.

## Search scope and distinctions

Used authenticated gh to search open/closed issues and open/closed/merged PRs. Literal searches covered "package = false", dynamic versions, get_requires_for_build_wheel, the exact flat-layout error, --locked, and upgrades. Conceptual searches covered virtual/non-package projects, metadata extraction, unnecessary backend calls, and omitting application versions. Followed issue timelines, maintainer comments, and historical fixes; supplemented empty PR search results with a paginated all-state PR title/body scan. Compared current source with uv 0.9.17 and the uv 0.5.21 release. Ruled out astral-sh/uv#17975 as static-version lockfile churn, astral-sh/uv#4058 as a pip compile fix, and astral-sh/uv#9549 as an sdist-filename optimization. astral-sh/uv#10522 and astral-sh/uv#10204 concern metadata requirements rather than this backend failure.

Additional inspected candidates include astral-sh/uv#12311 (a static virtual project rejected by `--no-build`), astral-sh/uv#15193 (`--no-install-project --no-sources` on an installable Hatch project), astral-sh/uv#21988 (permission for first-party metadata builds under `--no-build`), and astral-sh/uv#21728 (source-discovery consistency during lock validation). Their triggers and requested behavior differ from suppressing an unnecessary version lookup for a virtual root.

The broader metadata-free application discussion in astral-sh/uv#10204 and parsing explanation in astral-sh/uv#10522 do not require changing the standard to address this report: its explicit dynamic declaration already passes project validation.

## Maintainer next step

Reopen astral-sh/uv#9390 and carry over the minimal fresh-lock reproduction and the distinction from successful lockfile reuse. A future fix should cover a virtual root whose only dynamic field is its version while retaining required backend calls for genuinely dynamic dependency metadata. A static project version is the source-backed workaround discussed in the matching reports.
