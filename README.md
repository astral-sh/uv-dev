# Test `git_version_info_expected` doesn’t compile with custom profile: `PROFILE` is unset

Issue: astral-sh/uv#21884

Classification: needs more information

## Summary

The report says Fedora builds uv and its tests with a custom Cargo profile named `rpm` that
inherits from `release`, and that the `git_version_info_expected` integration-test helper fails to
compile because `env!("PROFILE")` has no compile-time value. The reported environment is Fedora
Rawhide x86_64, uv 0.12.7, and Python 3.15.0rc2, but the report does not include the Cargo command,
the `rpm` profile definition, the Cargo/Rust versions, or the Fedora RPM macro expansion.

The ordinary Cargo path does not show the failure. At the exact referenced commit,
`crates/uv/build.rs` receives Cargo's normalized profile value and emits
`cargo:rustc-env=PROFILE=release`. Cargo then makes that value available while compiling the
integration-test target. The downstream setup must differ in some unreported way before the
reported result can be independently reproduced.

## Classification

Needs more information. The compiler error in the report is plausible if the integration-test
target is compiled without the `crates/uv/build.rs` output, but neither uv's actual integration
target nor a minimal equivalent failed under a standard custom profile inheriting from `release`.
Source inspection alone is not sufficient to classify the Fedora-specific behavior as reproduced.

The exact Fedora build/test command and macro expansion, the complete `rpm` profile definition,
Cargo and Rust versions, and any downstream patches or flags that affect build scripts or test
target compilation are needed. In particular, maintainers need to know whether Fedora invokes
`rustc` for the integration test outside the Cargo package build that generated the build-script
metadata.

## Reproduction

Outcome: needs more information.

The check used Ubuntu 24.04.5 x86_64, Cargo 1.98.1, Rust 1.98.1, and checkout commit
`a1b84bcbda122236faae8fa5fdcbe16cfb76cde2`, which is the commit linked by the report. The installed
uv was 0.12.13 and the installed Python was 3.12.3; neither executable participates in expansion of
the Rust compile-time environment variable.

With the parent process's `PROFILE` removed and all Cargo, target, and uv cache state under a fresh
runner temporary directory, uv's actual integration-test crate was checked with a custom profile:

```console
$ env -u PROFILE \
    CARGO_HOME="$RUNNER_TEMP/uv-21884.txkIRM/uv-cargo-home" \
    CARGO_TARGET_DIR="$RUNNER_TEMP/uv-21884.txkIRM/uv-target" \
    UV_CACHE_DIR="$RUNNER_TEMP/uv-21884.txkIRM/uv-cache" \
    RUSTUP_TOOLCHAIN=stable \
    cargo check --package uv --test it --profile rpm \
    --config 'profile.rpm.inherits="release"'
    Finished `rpm` profile [optimized] target(s) in 1m 04s
```

This command compiles `crates/uv/tests/it/version.rs`, including the reported
`env!("PROFILE")` expression. The uv build-script output for this build was:

```text
cargo:rustc-env=PROFILE=release
```

A separate minimal crate reproducing the same mechanism also succeeded with
`env -u PROFILE cargo test --profile rpm`: its build script read `PROFILE`, forwarded it with
`cargo:rustc-env`, and its integration test verified `env!("PROFILE") == "release"`.

Existing integration coverage is `crates/uv/tests/it/version.rs`, test `self_version_json`; it calls
`git_version_info_expected` and therefore confirms that the helper compiles in normal test builds.
There is no existing test that invokes Cargo recursively with a custom release-derived profile or
models Fedora's RPM build macros, so it does not cover the unreported downstream path.

## Related

- astral-sh/uv#21750 (merged pull request), “Make Git stamping opt-in for development builds” —
  introduced the profile-sensitive expectation and intends release-derived profiles to retain Git
  metadata.
- astral-sh/uv#13212 (closed issue), “`version::self_version_json` and
  `version::version_get_fallback_unmanaged_json` test failures (outside git checkout?)” — concerns
  snapshot failures from an unpacked source archive, not a custom-profile compile error.
- astral-sh/uv#13566 (merged pull request), “Fix version json tests to work outside git checkout” —
  introduced `git_version_info_expected`; astral-sh/uv#21750 later added the profile check.

## Draft response

Thanks for the detailed report. We could not reproduce the missing compile-time `PROFILE` value
with standard Cargo behavior at the referenced commit. With `PROFILE` removed from the parent
environment, an `rpm` profile inheriting from `release` successfully compiled uv's `it` target;
Cargo gave `crates/uv/build.rs` the normalized value `release`, and its `cargo:rustc-env` output
made that value available to `env!("PROFILE")`.

Could you provide the exact Fedora build and test commands (including expanded RPM macros), the
complete custom-profile configuration, Cargo and Rust versions, and any downstream patches or
flags affecting build scripts? Those details are needed to reproduce how the integration test is
compiled without the uv build-script environment.
