# Test helper function `git_version_info_expected` doesn’t compile with custom profile: `PROFILE` is unset

Issue: astral-sh/uv#21884

Classification: downstream packaging issue (not an upstream bug)

## Summary

Fedora builds uv and its tests with a custom Cargo profile named `rpm` that inherits from
`release`. The reported `git_version_info_expected` compilation failure is explained by Fedora's
Rawhide packaging: `uv.spec` deletes `crates/uv/build.rs` and removes its `embed-manifest` build
dependency. That downstream change was based on the script's former Windows-only responsibility.

astral-sh/uv#21750 gave the same build script an additional cross-platform responsibility. It now
reads Cargo's normalized `PROFILE` value and emits `cargo:rustc-env=PROFILE=...` for the `uv`
package, making the value available to `env!("PROFILE")` in the integration test. Deleting the
script therefore directly removes the compile-time value and produces the reported error. If the
script is retained, Cargo normalizes Fedora's release-derived `rpm` profile to `release`, which is
the value expected by the test and Git-stamping logic.

## Classification

This is a downstream packaging issue, not an upstream bug. A maintainer identified the deletion in
Fedora's release manifest and stated that relying on the forwarded `PROFILE` value is intentional.
The Fedora Rawhide `uv.spec` confirms that it deletes `crates/uv/build.rs` under a stale comment
saying the script only embeds a Windows manifest. Source inspection confirms that the upstream
script now also forwards `PROFILE` on every platform.

The previous “needs more information” classification is resolved: the missing Fedora command and
profile details are no longer required to explain the failure. The ordinary Cargo path and a
minimal equivalent both work because they retain and run the build script.

## Cause and downstream fix

Fedora's preparation step currently performs both of these downstream changes:

```console
rm --verbose crates/uv/build.rs
tomcli set crates/uv/Cargo.toml del build-dependencies.embed-manifest
```

The first command removes the producer of the integration test's compile-time `PROFILE` value. The
second removes a dependency imported by that script, so retaining the script also requires retaining
the build dependency or replacing the downstream patch with an equivalent that preserves the
script's new profile-forwarding behavior.

The appropriate next step is to update Fedora's packaging rather than change
`git_version_info_expected` to tolerate an absent value. Treating an absent value as a development
profile would compile, but would incorrectly model an `rpm` profile that inherits from `release`.
The reporter's temporary patch that returns `false` is a workaround only; it disables the relevant
Git-metadata expectation.

## Reproduction and verification

Outcome: explained by downstream modification; not reproducible with the upstream build script
intact.

The earlier check used Ubuntu 24.04.5 x86_64, Cargo 1.98.1, Rust 1.98.1, and checkout commit
`a1b84bcbda122236faae8fa5fdcbe16cfb76cde2`. With the parent process's `PROFILE` removed, uv's
actual integration-test crate compiled successfully under a custom release-derived profile:

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

The retained `crates/uv/build.rs` emitted:

```text
cargo:rustc-env=PROFILE=release
```

A minimal crate using the same build-script forwarding mechanism also passed. These results align
with the newly identified Fedora-specific deletion: a standard custom profile is supported, while
removing the build script removes the value required by the integration test.

## Related

- astral-sh/uv#21750 (merged pull request), “Make Git stamping opt-in for development builds” —
  added the cross-platform profile-forwarding responsibility to `crates/uv/build.rs` and introduced
  the profile-sensitive test expectation. It is the key change that made Fedora's existing deletion
  invalid.
- astral-sh/uv#13212 (closed issue), “`version::self_version_json` and
  `version::version_get_fallback_unmanaged_json` test failures (outside git checkout?)” — concerns
  snapshot failures from an unpacked source archive, not deletion of the package build script.
- astral-sh/uv#13566 (merged pull request), “Fix version json tests to work outside git checkout” —
  introduced `git_version_info_expected`; astral-sh/uv#21750 later added the profile check.

## Evidence status

Confirmed from upstream source and astral-sh/uv#21750:

- Cargo supplies `PROFILE` to the build script.
- `crates/uv/build.rs` forwards it with `cargo:rustc-env`.
- A profile inheriting from `release` is forwarded as `release`.

Confirmed from Fedora Rawhide's current `uv.spec`:

- The packaging deletes `crates/uv/build.rs`.
- The accompanying comment assumes that the script only handles Windows manifest embedding.
- The packaging also removes the script's `embed-manifest` build dependency.

No upstream source change is currently indicated. The actionable correction is in Fedora's package
preparation steps.
