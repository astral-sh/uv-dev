# Linux nextest cache

These composite actions separate Cargo downloads from the compiled workspace cache for the Linux
`fast-build-nightly` nextest workload. The compiled cache has an exact source-commit key and a
fallback restricted to the same dependency, compiler, configuration, and workload identity. A
source-only change can therefore consume a compatible older entry and publish its own entry after
the complete workload succeeds.

`restore` and `save` use the official, separately pinned `actions/cache/restore` and
`actions/cache/save` actions. They do not copy native cache-service archives or import entries from
another repository. Adding these actions does not change any CI planner decision or existing cache
caller.

## Supported workload

The first contract is deliberately specific:

- Ubuntu 24.04, `x86_64-unknown-linux-gnu`, the source's numeric `rust-toolchain.toml` pin,
  Rustup-managed `cargo` and `rustc`, `cargo-nextest` 0.9.143, and `mold` as the selected `ld`.
- A clean, full source checkout inside `GITHUB_WORKSPACE`, with the expected repository and full
  commit supplied explicitly. The manifest records the actual commit, tree, checkout path, action
  source, and selected executable paths and hashes; `GITHUB_SHA` is recorded separately.
- `CARGO_INCREMENTAL=0`, `RUSTC_BOOTSTRAP=1`, `UV_LOCKED=1`, the default Cargo home, and
  `CARGO_TARGET_DIR=$GITHUB_WORKSPACE/target`. An explicit Cargo target, custom build directory,
  compiler wrapper, or custom target linker is outside this layout.
- The `fast-build-nightly` profile, checksum freshness, and the same features and `ci-linux` nextest
  profile as the Linux test job. The manifest's `workload.cargo_arguments` is the exact command
  suffix.

The target archive includes `target/.rustc_info.json` and the selected profile, including useful
workspace executables. It excludes incrementals and `.cargo-lock`. Downloads include Cargo's
registry/index/source and Git checkout stores, not credentials or installed tool binaries. Relevant
environment values and Cargo configuration contents contribute digests; the manifest does not dump
the environment or configuration contents.

## Caller contract

Call `restore` after installing the pinned Rust toolchain, nextest, and native build tools. Pass the
actual source checkout and its expected full commit. Use the returned absolute `cargo` invocation
path and `cargo-target-dir` for the workload. Invocation paths retain the `cargo` basename required
by Rustup; resolved executable paths are used only for identity checks.

The caller supplies `save-if` using its existing cache-publication policy. A writable cache miss or
fallback needs a freshness marker before the build and `CARGO_UNSTABLE_MTIME_ON_USE=true` during the
workload. After a successful complete workload, run `scripts/prune_cargo_workspace_cache.py` with
the selected profile directory and that marker, then pass `manifest` and `manifest-sha256` to
`save`. The save action rechecks the source, selected tools, paths, configuration, original
permission, and restore observations. It never turns a read-only restore into a writable one.

The normalized `cache-hit` output reports an exact compiled-target hit. `cache-matched-key` reports
the compatible entry actually restored, or an empty string. Downloads have their own exact-hit
output and key. A save step's successful outcome is not confirmation that the cache service
committed a new entry; a fresh consumer must restore the requested exact key before publication is
reported as confirmed.

Cargo's own freshness checks remain authoritative. Recording a different checkout path does not make
compiled artifacts hermetic or promise that Cargo can reuse them after relocation. The helper
detects ordinary source/tool changes at its observation boundaries; it cannot close races with an
uncooperative process that replaces and restores filesystem entries between those observations.

Run the source-only contract tests with:

```console
uv --no-config run --locked --python 3.12 --script scripts/tests/test_ci_rust_cache.py
```
