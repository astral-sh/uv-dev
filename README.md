# `tool.uv.environments` also acts as `required-environments`

Issue: astral-sh/uv#23271

Classification: question

## Summary

The reporter uses uv 0.13.8 to require macOS ARM64, Linux x86-64, and Windows AMD64 support. They initially configured `required-environments`, added the same entries to `environments` to filter unrelated wheels from the lockfile, and found that removing `required-environments` did not remove wheel-availability enforcement.

Their reproduction initializes a project, adds unconditional `pywin32` with `uv add --no-sync pywin32`, configures those three environments, and runs `uv lock`. Resolution fails for the macOS ARM64 split because pywin32 312 has no compatible wheel. They ask whether this overlap is expected given the documentation's distinction between limiting resolution environments and expanding required platform coverage.

The failure matches intentional behavior shipped in uv 0.10.11. The two settings overlap for artifact coverage, while only `environments` narrows the supported resolution scope. No new resolver defect or recurrence of a previously fixed bug is established.

## Draft response

Yes, this is expected. Since uv 0.10.11, astral-sh/uv#18451 checks wheel availability for explicitly listed `environments` as well as `required-environments` when a package has no source distribution. The settings overlap here: `environments` also restricts the resolution scope and filters unreachable wheels, while `required-environments` adds availability requirements without excluding other platforms.

Your unconditional `pywin32` dependency is therefore unsatisfiable for the configured macOS and Linux environments. Declare it as `"pywin32>=312; sys_platform == 'win32'"` to keep those environments while requiring the package only on Windows. If both lists contain the same markers, you can omit `required-environments`. The documentation could state this overlap more explicitly.

## Classification

The reproduction matches intentional behavior introduced by astral-sh/uv#18451 in uv 0.10.11 and retained in the current source and an existing pywin32 integration test. Explicit environments also require artifact coverage for applicable wheel-only dependencies; environments additionally narrows resolution scope, whereas required-environments alone does not. No incorrect resolution or regression is established. The report primarily asks for clarification of this overlap; the historical fixes address the opposite failure or separate pruning behavior, rather than an existing equivalent clarification request.

The quoted documentation distinguishes resolution scope from required availability, but does not explicitly explain that configured supported environments also require artifact coverage. This omission makes the question understandable; the reproduction does not establish that either setting is malfunctioning. Clarifying that overlap would help readers.

The historical availability fix is explanatory context rather than a duplicate clarification request. It corrected acceptance of packages with no usable artifacts; this report asks whether the resulting rejection is intended.

## Related

- astral-sh/uv#18451 — Enforce available distributions for supported environments (pull request, merged). Directly establishes the reported behavior: combines environments and required-environments for artifact-coverage checks, with a test rejecting unconditional pywin32 under Linux-only environments. Merged March 16, 2026 and released in uv 0.10.11, before the reported 0.13.8.
- astral-sh/uv#17067 — Lockfile with package that has no distributions with environments (issue, closed). Explains the correctness rationale for the checks: environments previously allowed wheel-only packages to produce lockfile entries with no usable distributions. Closed by astral-sh/uv#18451; the new report asks whether the resulting rejection is expected.
- astral-sh/uv#17118 — Avoid allowing package-versions with no wheels in supported environments (pull request, closed). Precursor discussion explicitly considers treating environments like required-environments for wheel availability. Closed without merging and superseded by astral-sh/uv#18451; useful historical explanation, not an outstanding fix.
- astral-sh/uv#6961 — Prune unreachable wheels from lockfile (pull request, merged). Addresses the secondary artifact-filtering concern by removing wheels incompatible with a package's applicable platform markers. This differs from required-environments, which adds availability requirements without narrowing the supported resolution platforms.

## Supporting evidence

- `crates/uv-lock-operations/src/lock.rs:638` initializes the supported and required environment lists, combines both into `artifact_environments`, and passes the result to the resolver. This directly confirms why removing the duplicated required list leaves the checks active.
- `crates/uv-resolver/src/resolver/mod.rs:1673` checks configured artifact environments against a candidate's distribution coverage and rejects or forks candidates when coverage is missing. These checks respect whether the dependency applies in the environment.
- `crates/uv/tests/lock/lock.rs:45013` contains `lock_supported_environment_wheel_only_package_requires_compatible_wheels`, gated by `test-universal`. It declares unconditional pywin32 and Linux-only `environments`, without `required-environments`, and snapshots a failed resolution for missing Linux-compatible wheels. The scenario already has direct integration coverage.
- astral-sh/uv#18451 explicitly says it enforces builds for supported environments in addition to required environments. Its diff introduced the combination of the lists and the pywin32 failure test. It merged on March 16, 2026.
- `changelogs/0.10.x.md:518` identifies uv 0.10.11, released March 16, 2026; the bug-fix entry at line 537 names astral-sh/uv#18451. The reporter's uv 0.13.8 is later.
- In astral-sh/uv#17118, a maintainer explicitly described the proposed behavior as potentially equivalent to treating supported environments as required environments for coverage. The pull request was later superseded by astral-sh/uv#18451.
- `crates/uv-lock/src/lock/mod.rs:2485` combines supported environments for lockfile filtering; the wheel-retention logic at line 2540 intersects those markers with the package's applicability markers. Required environments are not used as the scope filter. astral-sh/uv#6961 documents the wheel-pruning purpose.
- `crates/uv-workspace/src/pyproject.rs:696` and `docs/concepts/resolution.md:197` contain the quoted distinction. Neither explicitly describes the shared artifact-coverage checks.

These findings come from source, an existing test snapshot, release notes, and historical issue/pull request discussion. No reproduction commands, builds, or tests were executed.

## Search coverage and exclusions

Separated explicit-environment wheel failures, the settings' overlapping semantics/documentation, and lockfile artifact pruning. Searched astral-sh/uv open and closed issues using environments, required-environments, tool.uv.environments, pywin32, compatible wheels, split, expected, difference, documentation, no source distribution, and wheel/filter/platform combinations, including package-independent queries. Searched open, closed, and merged PRs for required-environments, environments, supported environments, wheels, and pywin32; direct PR searches returned no hits, so followed issue/comment references to inspect astral-sh/uv#17118, astral-sh/uv#18451, astral-sh/uv#6961, and associated fixes. Checked source, existing tests, and uv 0.10.11 release notes. Ruled out astral-sh/uv#11664 (fixed Windows marker casing), astral-sh/uv#18527 and astral-sh/uv#18536 (fixed abi3 Python-version coverage regression), astral-sh/uv#17382 (unsupported-platform marker documentation), and astral-sh/uv#11463 (metadata downloads/403 errors rather than retained lockfile artifacts). Followed astral-sh/uv#6512 to the wheel-pruning work.

Additional distinctions from inspected candidates:

- astral-sh/uv#11664 concerns a correctly Windows-conditional pywin32 dependency being mishandled because of Windows architecture-marker casing. The current report uses the correct AMD64 spelling and fails on macOS for an unconditional dependency.
- astral-sh/uv#18527 superficially resembles this report because enforcement began rejecting a package after uv 0.10.11. Its fixing pull request, astral-sh/uv#18536, establishes an abi3 Python-version interpretation bug. That mechanism does not explain a Windows-only pywin32 wheel missing on macOS.
- astral-sh/uv#17382 requests documentation of marker limitations for platforms such as FreeBSD, rather than the overlap between the settings for supported platforms.
- astral-sh/uv#11463 concerns fetching an excluded platform's wheel for metadata and encountering HTTP 403. The reporter here describes lockfile contents and an availability failure.
- astral-sh/uv#6512 concerns missing pruning despite restricting `environments`. Its discussion leads to astral-sh/uv#6961; it does not establish that `required-environments` should prune other platforms.
- astral-sh/uv#18428 requires conflicting dependency groups and concerns wheel filtering. The current reproduction has no conflicts configuration.

## Recommended next step

Answer the semantics question with the versioned implementation reference and suggest a Windows-only dependency marker for pywin32. A documentation clarification should explain that supported environments also participate in artifact-availability checks, while required environments do not restrict the resolution or prune unrelated platforms.

Only this handoff README was written. No checkout files or GitHub state were changed.
