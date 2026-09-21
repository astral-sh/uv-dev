# Test `git_version_info_expected` doesn’t compile with custom profile: `PROFILE` is unset

Issue: astral-sh/uv#21884

Classification: bug

## Summary

Fedora builds uv and its tests with a custom Cargo profile named `rpm` that inherits from
`release`. The `git_version_info_expected` integration-test helper now expands
`env!("PROFILE")`, but `PROFILE` is absent from that test target's compile-time environment. The
result is a compiler error before the tests can run.

The failing check was added by astral-sh/uv#21750 as part of making Git stamping opt-in for
development builds. That pull request explicitly intends release-derived profiles to keep Git
metadata. The `uv` crate build script attempts to expose Cargo's build-script `PROFILE` value to
integration tests with `cargo:rustc-env`, but the test's unconditional `env!` still makes absence a
compile error in the reported Fedora build. There is no existing issue or active pull request
tracking this exact custom-profile failure.

## Draft response

Thanks for the detailed report. This is a bug. astral-sh/uv#21750 added the unconditional
`env!("PROFILE")` check while explicitly intending profiles derived from `release` to retain Git
metadata. The crate build script attempts to expose that value to integration tests, but the test
cannot compile when it is absent, as in this Fedora build. The earlier packaging fix in
astral-sh/uv#13566 does not cover custom profiles.

A fix should remove the hard compile-time requirement while preserving the intended distinction
between development-derived and release-derived profiles; simply treating an unset value as a
development build would make the test compile but would not model the Fedora build correctly. A
focused pull request with coverage for a custom profile inheriting from `release` would be an
appropriate next step.

## Classification

This is a bug because checked-in integration-test code fails to compile under a release-derived
custom profile, contrary to the behavior documented and intended by astral-sh/uv#21750. The source
confirms that the test uses an unconditional compile-time lookup; the report demonstrates a target
where the value is not present. The exact reason Cargo's value is not forwarded into this Fedora
test target is not yet confirmed, so it should not be presented as the root cause.

This is not a duplicate. No prior issue or active pull request tracks the same compile failure.
Earlier version-test reports concern related packaging assumptions but have different triggers and
failure modes.

## Related

- astral-sh/uv#21750 (merged pull request), “Make Git stamping opt-in for development builds” —
  directly introduced the unconditional `env!("PROFILE")` branch in
  `git_version_info_expected`. Its stated design includes Git metadata for profiles derived from
  `release`, making it the most relevant source and intent evidence.
- astral-sh/uv#13212 (closed issue), “`version::self_version_json` and
  `version::version_get_fallback_unmanaged_json` test failures (outside git checkout?)” — a
  historical downstream-packaging report showing that version-test expectations must accommodate
  builds without upstream's Git context. Its trigger was an unpacked GitHub archive and its symptom
  was a snapshot failure, not the present compile error.
- astral-sh/uv#13566 (merged pull request), “Fix version json tests to work outside git checkout” —
  fixed astral-sh/uv#13212 and introduced `git_version_info_expected` to select expectations based
  on packaging context. astral-sh/uv#21750 later extended this helper with the profile check that
  now fails.

## Supporting evidence

- `crates/uv/tests/it/version.rs` calls `env!("PROFILE")`, which requires the value to exist when
  the integration test is compiled.
- `crates/uv/build.rs` reads Cargo's build-script `PROFILE` and emits it through
  `cargo:rustc-env`, confirming the intended bridge from the build script to the integration test.
- `crates/uv-cli/build.rs` uses the runtime build-script value to include Git metadata for
  `release` and release-derived profiles, unless development builds explicitly opt in through
  `UV_INTERNAL__BUILD_GIT_INFO=1`.
- The diff and description of astral-sh/uv#21750 state that release-derived profiles retain Git
  stamping, so defaulting an absent test-side value to “development” would not accurately test the
  reported Fedora build.

## Search coverage

The GitHub search covered open and closed issues and open, closed, and merged pull requests. Literal
queries included the exact compiler fragment, `env!("PROFILE")`, `git_version_info_expected`, and
`UV_INTERNAL__BUILD_GIT_INFO`. Conceptual and fix-oriented queries covered custom Cargo profiles,
profile inheritance, build-script environment forwarding, release versus development Git stamping,
and downstream builds without Git metadata. File history and the relevant pull-request discussions
were also inspected.

astral-sh/uv#14785 and its merged fix astral-sh/uv#14786 were plausible because a maintainer called
the same helper's logic wrong, but they were ruled out as matches: they concern a runtime assertion
in an obsolete `version_get_fallback_unmanaged_json` test, not compilation under a custom profile.
