# `tool.uv.environments` also acts as `required-environments`

Issue: astral-sh/uv#23271

Classification: question

## Summary

The reporter, using uv 0.13.8, asks why listing macOS ARM64, Linux x86-64, and Windows AMD64 in `tool.uv.environments` rejects unconditional `pywin32` even without `required-environments`. They expected `environments` only to limit resolution and filter unrelated wheels, with availability requirements controlled separately by `required-environments`.

**Reproduced with the installed uv 0.13.7:** the reported commands and environment markers produce the same macOS ARM64 resolution failure for pywin32 312. This is consistent with intentional availability enforcement introduced in uv 0.10.11 by astral-sh/uv#18451 and with a dedicated existing integration test. Reproduction establishes the observed behavior, not a new resolver defect. The exact reported uv 0.13.8 binary was not tested.

## Classification

Question about overlapping settings and documentation clarity. Both settings participate in artifact-availability checks for applicable wheel-only dependencies. `environments` additionally restricts the resolution scope and filters unreachable wheels; `required-environments` alone does not narrow that scope. No incorrect resolution or regression is established.

The quoted documentation distinguishes limiting resolution scope from expanding required availability, but does not explicitly explain that supported environments also require artifact coverage. Clarifying that overlap would help readers. The historical availability fix explains the behavior rather than constituting a duplicate clarification request.

## Reproduction

Outcome: **reproducible**.

Environment: Linux x86-64 (`x86_64-unknown-linux-gnu`), CPython 3.12.3, installed `uv 0.13.7` from `/opt/hostedtoolcache/uv/0.13.7/x86_64/uv`. The report specifies uv 0.13.8 and does not specify its host platform or Python version. Resolution uses explicit cross-platform markers, so the macOS failure was observable from Linux.

All reproduction files, caches, and temporary HOME/XDG/Python storage were isolated under `/tmp/uv-23271-3syi8zsh`. Commands used a restricted environment with Python downloads disabled and public PyPI explicitly selected. No packages were synced or installed. No builds or repository test binaries were run.

From the temporary project directory:

```sh
uv init --vcs none --no-workspace --python 3.12 .
uv add --no-sync pywin32
```

Both commands exit 0. The generated project has `requires-python = ">=3.12"` and `dependencies = ["pywin32>=312"]`; the initial lock resolves pywin32 312 without any explicit environment settings. The init flags avoid version-control and workspace side effects.

Append the reported configuration to `pyproject.toml`, without adding `required-environments`:

```toml
[tool.uv]
environments = [
    "sys_platform == 'darwin' and platform_machine == 'arm64'",
    "sys_platform == 'linux' and platform_machine == 'x86_64'",
    "sys_platform == 'win32' and platform_machine == 'AMD64'",
]
```

Run `uv lock`. It exits 1 with the reported failure:

```text
error: No solution found when resolving dependencies for split (markers: platform_machine == 'arm64' and sys_platform == 'darwin')
  cause: Because pywin32==312 has no `platform_machine == 'arm64' and sys_platform == 'darwin'`-compatible wheels and only pywin32<=312 is available, we can conclude that pywin32>=312 cannot be used.
         And because your project depends on pywin32>=312, we can conclude that your project's requirements are unsatisfiable.
```

The output additionally suggests limiting environments because the failed environment differs from the host. This does not alter the failure being investigated.

Explicit controls confirm the settings' overlap and distinction:

- Replace the dependency with `"pywin32>=312; sys_platform == 'win32'"`, retain the three `environments`, and run `uv lock`: exit 0. The lockfile contains four pywin32 wheels, all tagged `win_amd64`.
- With unconditional `pywin32>=312`, replace `environments` with `required-environments` using the same markers and run `uv lock`: exit 1, again citing missing macOS ARM64 wheels. The resolution split is `sys_platform != 'win32'` in this case.
- With the Windows-conditional dependency and only those `required-environments`, run `uv lock`: exit 0. The lockfile contains twelve pywin32 wheels across `win32`, `win_amd64`, and `win_arm64`, demonstrating that this setting alone does not prune unrelated Windows architectures.

Existing test coverage was read, including setup and assertions: `crates/uv/tests/lock/lock.rs`, test `lock_supported_environment_wheel_only_package_requires_compatible_wheels`, gated by `#[cfg(feature = "test-universal")]`. It uses Python 3.12, `requires-python = "~=3.12.0"`, unconditional `pywin32`, Linux-only `environments`, no `required-environments`, and an exclude-newer date of 2025-01-30. Its snapshot expects exit 1 and rejects pywin32 versions with no Linux-compatible wheels. This directly covers the reported enforcement, using a smaller platform list. The test was inspected, not executed; the commands above used the installed executable.

Raw command outputs, original/generated configuration, and comparison lockfiles are retained in `/tmp/uv-23271-3syi8zsh` for this runner session.

## Draft response

Yes, this is expected. Since uv 0.10.11, astral-sh/uv#18451 checks wheel availability for explicitly listed `environments` as well as `required-environments` when a dependency has no source distribution. The settings overlap here: `environments` also restricts resolution scope and filters unreachable wheels, while `required-environments` adds availability requirements without excluding other platforms.

An unconditional `pywin32` dependency therefore cannot satisfy the configured macOS and Linux environments. Declare it as `"pywin32>=312; sys_platform == 'win32'"` to keep those environments while requiring the package only on Windows. That configuration succeeds in the reproduction. For the three markers in this example, duplicating them in `required-environments` is unnecessary. The documentation could state the overlap more explicitly.

## Related

- astral-sh/uv#18451 — Enforce available distributions for supported environments (pull request, merged). Combines supported and required environments for artifact-coverage checks and adds the pywin32 integration test described above. Merged March 16, 2026 and released in uv 0.10.11, before the reported 0.13.8.
- astral-sh/uv#17067 — Lockfile with package that has no distributions with environments (issue, closed). Explains the correctness rationale: supported environments previously allowed wheel-only packages to produce lockfile entries with no usable distributions. Closed by astral-sh/uv#18451; the current report asks whether the resulting rejection is expected.
- astral-sh/uv#17118 — Avoid allowing package-versions with no wheels in supported environments (pull request, closed). Precursor discussion considers treating supported environments like required environments for wheel availability. Closed without merging and superseded by astral-sh/uv#18451; historical explanation rather than an outstanding fix.
- astral-sh/uv#6961 — Prune unreachable wheels from lockfile (pull request, merged). Removes wheels incompatible with a package's applicable platform markers. This differs from required environments, which add availability requirements without narrowing supported resolution platforms.

## Supporting evidence

- `crates/uv-lock-operations/src/lock.rs:638` obtains supported and required environment lists, combines them into `artifact_environments`, and passes the combination to the resolver. This explains why removing a duplicated required list leaves coverage checks active.
- `crates/uv/tests/lock/lock.rs:45013` contains the directly relevant integration test detailed in the reproduction section.
- `changelogs/0.10.x.md:517` identifies uv 0.10.11, released March 16, 2026. Its bug-fix entry names astral-sh/uv#18451, “Enforce available distributions for supported environments.”
- `crates/uv-workspace/src/pyproject.rs:686` documents required-environment coverage; the comparison with supported environments starts at line 696. `docs/concepts/resolution.md:197` contains the same distinction. Neither explicitly describes the shared artifact-coverage checks.
- Earlier issue-context research inspected astral-sh/uv#18451 and its predecessor astral-sh/uv#17118. Their historical explanation agrees with the changelog, source, integration snapshot, and newly observed command results.

## Search coverage and exclusions

Prior issue-context research searched open and closed astral-sh/uv issues for environments, required-environments, tool.uv.environments, pywin32, compatible wheels, split, expected behavior, differences, documentation, no source distribution, and wheel/filter/platform combinations. Pull request searches and issue references led to astral-sh/uv#17118, astral-sh/uv#18451, and astral-sh/uv#6961. The reproduction work independently read the relevant current source, documentation, existing integration test, and uv 0.10.11 changelog.

Related candidates concern different behavior:

- astral-sh/uv#11664 concerns a Windows-conditional pywin32 dependency mishandled because of Windows architecture-marker casing. This report uses the correct AMD64 spelling and fails on macOS for an unconditional dependency.
- astral-sh/uv#18527 and its fixing pull request astral-sh/uv#18536 concern abi3 Python-version interpretation after uv 0.10.11. That mechanism does not explain Windows-only pywin32 wheels missing on macOS.
- astral-sh/uv#17382 concerns documentation of marker limitations for platforms such as FreeBSD, rather than overlap between these settings.
- astral-sh/uv#11463 concerns fetching an excluded platform's wheel for metadata and encountering HTTP 403, rather than an availability failure or retained lockfile artifacts.
- astral-sh/uv#6512 concerns missing pruning despite restricted environments and leads to astral-sh/uv#6961.
- astral-sh/uv#18428 requires conflicting dependency groups and concerns wheel filtering. This reproduction has no dependency groups or conflicts configuration.

## Recommended next step

Answer the semantics question with the versioned implementation reference and the verified Windows-only dependency marker. A documentation clarification should explain that supported environments also participate in artifact-availability checks, while required environments do not restrict resolution scope or prune unrelated platforms.

Only this external handoff README and temporary reproduction artifacts were written. No checkout files or GitHub state were changed.
