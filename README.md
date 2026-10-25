# Is there a way to exclude musl support?

Issue: astral-sh/uv#22913

Classification: duplicate

## Summary

The reporter uses uv 0.13.5 on macOS 15 and Ubuntu 26.04 and limits `tool.uv.environments` to macOS and Linux x86_64/aarch64. They want an expression such as `platform_libc == 'glibc'` to exclude musl because some dependencies distribute Go binaries as wheels without musl support. Setting an artificially high musl value in the `minimum-libc-version` preview reportedly caused resolution failure.

The report does not identify the dependencies, provide the failing command or error output, or include the attempted preview configuration. Whether those packages have usable source distributions is unverified.

Duplicate of astral-sh/uv#11743. astral-sh/uv#21671 proposed libc exclusions but closed unmerged; merged astral-sh/uv#21651 provides coverage requirements only.

The supported behavior in uv 0.13.5 is to require glibc-compatible artifacts without requiring musl, using a glibc-only baseline with `required-environments`. That does not prohibit musl installations or remove musl wheels. Merely including Linux in `environments` does not require wheels for every libc.

## Draft response

You cannot currently exclude musl through `tool.uv.environments`: PEP 508 has no libc marker. This request is tracked in astral-sh/uv#11743; let's keep the discussion there.

`minimum-libc-version` controls required artifact coverage. Setting `musl`, even to a very high version, still requires musl-compatible artifacts. To require only glibc coverage, configure your Linux `required-environments` and set only the desired `glibc` baseline, leaving `musl` unset. This does not exclude musl or remove its wheels from the lockfile. The `environments` configuration you posted does not itself require musl wheels.

If resolution still fails after removing the musl requirement, please share the complete error and a minimal `pyproject.toml` including the affected dependencies in that discussion.

## Classification

astral-sh/uv#11743 already tracks the same missing capability to exclude musllinux from supported environments when dependencies lack musl wheels. The preview setting adds libc coverage requirements and does not implement exclusion; the alternative exclusion PR was never merged. The report supplies no error or dependency reproduction establishing a separate correctness bug or regression.

The existing issue asks for the same supported-environment restriction and cites the same missing dependency wheels as its motivation. Its extra emphasis on an early unsupported-platform error does not make the underlying request different. Duplicate takes precedence over enhancement or question.

The failed high-version workaround is not evidence that libc exclusion regressed: no shipped exclusion feature was identified. Configuring musl requires compatible coverage regardless of how high its baseline is. The exact cause of the reporter's resolution failure remains unconfirmed without the error and dependencies.

## Related

- astral-sh/uv#11743 — How to specify that a distribution package is for manylinux, not musllinux? (open). Tracks the same request: exclude musllinux through tool.uv.environments because dependencies lack compatible wheels. Maintainer comments confirm that PEP 508 markers cannot distinguish libc implementations and that a new configuration mechanism needs design.
- astral-sh/uv#21671 — Add minimum libc versions to supported and required environments (closed without merging). Proposed libc configuration in environments that would exclude wheels for omitted libc implementations, directly addressing the requested capability. Closed without merging on 2026-09-18, so its exclusion syntax is not available.
- astral-sh/uv#21651 — Add minimum libc versions for universal resolution (merged). Introduced the preview setting used by the reporter; merged on 2026-09-18 and present in uv 0.13.5. It requires compatible artifacts for configured libc baselines and retains other wheels. Specifying musl adds a coverage requirement rather than excluding musl.

## Supporting evidence

- In astral-sh/uv#11743, maintainer comments dated 2025-02-24 explicitly state that `tool.uv.environments` cannot capture manylinux versus musllinux because PEP 508 has no equivalent libc constraint. The follow-up says a new field or syntax needs configuration design. The issue remains open with `enhancement` and `needs-design` labels.
- `docs/concepts/resolution.md:191` describes the default wheel requirement and distinguishes limiting `environments` from adding `required-environments`. It does not require every Linux libc to be covered by default.
- `docs/concepts/resolution.md:241` states that environment markers do not include libc implementation or version. At line 259 it explicitly says that omitting a libc means it is not required, not excluded. The same text was verified through GitHub at the reporter's exact `0.13.5` tag.
- `crates/uv-workspace/src/pyproject.rs:727` documents `minimum-libc-version` as required coverage for universal resolution, including `uv lock` and `uv pip compile --universal`.
- `crates/uv-distribution-types/src/minimum_libc_version.rs:23` implements separate glibc and musl coverage checks. A musllinux wheel covers a configured musl baseline when its required version is no greater than that baseline. A manylinux wheel does not satisfy a configured musl requirement; unconfigured baselines accept any platform. Generic Linux wheels can satisfy either.
- `crates/uv/tests/lock/minimum_libc.rs:175`, `minimum_libc_switch_families`, demonstrates glibc-only coverage retaining musl wheels, adding musl coverage changing the selected package version, and failure when the requested version lacks the required coverage. This existing test was inspected; no tests were added or run.
- astral-sh/uv#21651 merged on 2026-09-18, before uv 0.13.5 was published on 2026-10-22. Its description and maintainer review explain that compatible older-baseline wheels satisfy coverage while other wheels remain in the lockfile. The feature is not an exclusion mechanism.
- astral-sh/uv#21671 explicitly proposed excluding omitted libc implementations through structured `environments` entries. Its state is closed and `mergedAt` is null. Related alternatives astral-sh/uv#21767, astral-sh/uv#21769, and astral-sh/uv#21785 were also closed without merging; their proposed marker syntax should not be recommended as available configuration.

## Search scope and candidate comparison

Before searching, the report was separated into two claims:

1. Requested capability: exclude musl while retaining glibc Linux on x86_64/aarch64 and macOS, expressed through project resolution environments. Exact identifiers include `platform_libc`, `glibc`, `musl`, and `environments`. The trigger is dependencies whose published binary wheels do not support musl.
2. Reported workaround failure: configuring an artificially high musl baseline with `minimum-libc-version` caused resolution failure. No exact error fragment or affected package was supplied. This was searched separately from the desired exclusion capability.

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Separately searched libc exclusion/markers (platform_libc, musl, musllinux, libc marker, musl environments, manylinux) and resolution/preview behavior (minimum-libc-version, required-environments, musl resolution). Broadened beyond platform/version details and inspected maintainer comments, timelines, linked PRs, and uv 0.13.5 documentation. PR keyword searches returned no hits, so supplemented them with the 1,000 most recent PRs across all states and direct reference traversal. Followed astral-sh/uv#12597 to the preview implementations. Ruled out astral-sh/uv#7816 (missing-wheel warnings), astral-sh/uv#8449 (disable manylinux at installation), astral-sh/uv#17178 (Termux dependency selection), and astral-sh/uv#14407 (platform-specific pip compilation) as duplicate targets.

Broader libc and musllinux searches included closed issues to check for earlier fixes. The strongest candidates were read with their comments, and the reference chain from astral-sh/uv#12597 identified both the merged preview feature and the unmerged exclusion design. A transient search API rate limit was encountered; subsequent explicit searches across open, closed, and merged states completed. Search results were supplemented with direct PR metadata and the recent all-state PR listing.

The adjacent discussion astral-sh/uv#12597 concerns requiring older glibc versions and backtracking, rather than excluding a libc implementation. Its 2025 statement that backtracking was unavailable predates astral-sh/uv#21651 and is not a description of uv 0.13.5.

The link from astral-sh/uv#11743 to astral-sh/uv#7816 was inspected: that issue concerns warnings when wheels are missing for the current Python, not libc environment restrictions. astral-sh/uv#8449 requests disabling manylinux wheels at installation, the opposite artifact family and a different operation. astral-sh/uv#17178 concerns selecting dependencies on Android/Termux and is useful only as confirmation of the marker limitation. astral-sh/uv#14407 concerns a single target for `uv pip compile`, where `--python-platform` is applicable; it does not provide a libc exclusion marker for a universal project lock.

## Handoff

Centralize the exclusion request in astral-sh/uv#11743. Explain the preview setting's coverage semantics and ask for the full error and dependency example only if resolution remains broken after removing the unintended musl requirement. Do not recommend the unmerged structured-environment or libc-marker syntax.

This handoff and proposed reply are for maintainer review. No GitHub changes were made.
