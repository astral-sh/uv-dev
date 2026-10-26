# Please update `reqsign-core` to 3.3.2

Issue: astral-sh/uv#22906

Classification: bug

## Summary

The reporter requests `reqsign-core` 3.3.2 because the currently selected version
depends on `mea`, which is broken on PPC32. They report that development moved
to `asyncband` and that the replacement removed the problematic code.

The checkout examined during triage and reproduction locked `reqsign` 0.20.6,
`reqsign-core` 3.3.1, and `mea` 0.6.7.
A minimal dependency check reproduces a compilation failure in `mea` 0.6.7
for `powerpc-unknown-linux-gnu`: its unconditional `AtomicU64` import fails
with Rust error E0432. An equivalent check using `asyncband` 0.7.1 with the
`mutex` feature succeeds for the same target. Published `reqsign-core` 3.3.2
uses that replacement and feature. A complete uv build was not attempted.

The requested dependency update is now merged in astral-sh/uv#22925. It
updates `reqsign` to 0.20.7 and `reqsign-core` to 3.3.2, removes `mea`, and
locks `asyncband` 0.7.4. The maintainer comment links this PR to the report.
A complete PPC32 uv build and availability in a published uv release remain
unverified.

The issue was opened on October 24, 2026. The supplied discussion contains
a maintainer comment linking astral-sh/uv#22925, but does not
provide an exact target triple, build command, diagnostic, uv version, or a
before-and-after build result. `powerpc-unknown-linux-gnu` was selected as a
representative PPC32 target, not supplied by the reporter.

## Fix status

Maintainer EliteTK linked astral-sh/uv#22925 in the issue discussion. GitHub
confirms that PR merged on October 26, 2026, at 17:40:11 UTC, with merge commit
`c3bcf4bb9d51ffc918cfdfcb15d716e061f3ba2e`.

The PR changes only `Cargo.lock`. Its diff confirms `reqsign` 0.20.6 → 0.20.7,
`reqsign-core` 3.3.1 → 3.3.2, removal of the `mea` 0.6.7 package, and addition
of `asyncband` 0.7.4. This incorporates the requested dependency replacement;
a separate lockfile-update PR is no longer needed.

The PR's required-jobs check succeeded, including Linux and Windows Cargo
tests and Clippy checks. The returned check list contains no dedicated PPC32
build. Release-artifact builds, integration tests, and system tests were
skipped. The earlier isolated reproduction checked `asyncband` 0.7.1, whereas
the merged lockfile selects 0.7.4; it does not verify the complete merged
uv dependency graph on PPC32. No reporter confirmation or released uv version
containing the change is established by the supplied discussion or PR evidence.

## Classification

Bug: a dependency-level PPC32 compilation incompatibility is reproduced,
not merely inferred from source. The published `mea` 0.6.7 archive matches
the checkout lockfile checksum and fails to compile on the selected PPC32
target. Its replacement compiles with the feature used by `reqsign-core`.

The report does not establish a regression between particular uv releases.
The observed checks establish the dependency blocker and the replacement's
compatibility for this target; they do not establish that a complete uv
build passes after updating `reqsign-core`.

The dashboard is a general dependency queue, not a canonical discussion of
this failure. astral-sh/uv#22925 now incorporates the upstream change into
uv's lockfile. It was created after this report and is the implementing PR;
the classification remains bug rather than duplicate.

## Reproduction

**Outcome: reproducible**, at the dependency-compilation level.

### Environment and scope

- Host: Linux x86_64; cross-check target: `powerpc-unknown-linux-gnu`
  (32-bit, big-endian GNU/Linux).
- Rust: `rustc 1.99.0 (b940084d7 2026-09-28)`;
  Cargo: `1.99.0 (5f94df478 2026-08-27)`.
- Installed uv on PATH: `uv 0.13.5 (x86_64-unknown-linux-gnu)`;
  Python: `3.12.3`. Neither is involved in this Rust compilation failure.
- A separate minimal Rust toolchain and PPC32 standard library were installed
  under the temporary reproduction directory. Cargo caches, manifests,
  lockfiles, downloaded sources, logs, and build outputs are all there.
- No release build, target execution, or complete uv build was performed.
  `cargo check` is sufficient to observe the import failure and requires no
  PPC32 linker or emulator.

### Minimal fixture and commands

Initialize an isolated temporary directory and toolchain:

```sh
repro_dir=$(mktemp -d /tmp/uv-22906-XXXXXX)
cd "$repro_dir"
export RUSTUP_HOME="$repro_dir/rustup-home"
export CARGO_HOME="$repro_dir/cargo-home"
export CARGO_TARGET_DIR="$repro_dir/target"
rustup toolchain install 1.99.0 --profile minimal --target powerpc-unknown-linux-gnu --no-self-update
mkdir -p mea-check/src asyncband-check/src
```

Create `mea-check/Cargo.toml`:

```toml
[package]
name = "mea-ppc32-check"
version = "0.0.0"
edition = "2024"

[dependencies]
mea = "=0.6.7"
```

Create `mea-check/src/lib.rs` containing:

```rust
pub use mea::mutex::Mutex;
```

Run:

```sh
cargo +1.99.0 check --manifest-path mea-check/Cargo.toml --target powerpc-unknown-linux-gnu
```

Observed exit status: **101**. Cargo resolved `mea` 0.6.7, `hashbrown`
0.17.1, and `slab` 0.4.12. The diagnostic is:

```text
error[E0432]: unresolved import `std::sync::atomic::AtomicU64`
  --> .../mea-0.6.7/src/broadcast/overflow/mod.rs:67:5
67 | use std::sync::atomic::AtomicU64;
   |     ^^^^^^^^^^^^^^^^^^^---------
   |                        |
   |                        no `AtomicU64` in `sync::atomic`
error: could not compile `mea` (lib) due to 1 previous error
```

Even a consumer of only the mutex triggers this failure because `mea`
unconditionally compiles the broadcast overflow module. `reqsign-core`
3.3.1 imports `mea::mutex::Mutex` in `src/signer.rs`.

For the replacement comparison, create `asyncband-check/Cargo.toml`:

```toml
[package]
name = "asyncband-ppc32-check"
version = "0.0.0"
edition = "2024"

[dependencies]
asyncband = { version = "=0.7.1", features = ["mutex"] }
```

Create `asyncband-check/src/lib.rs` containing:

```rust
pub use asyncband::mutex::Mutex;
```

Run:

```sh
cargo +1.99.0 check --manifest-path asyncband-check/Cargo.toml --target powerpc-unknown-linux-gnu
```

Observed exit status: **0**; both `asyncband` 0.7.1 and the fixture finish
checking successfully in the dev profile. Published `reqsign-core` 3.3.2
declares `asyncband` version `0.7.1` with `features = ["mutex"]` and imports
`asyncband::mutex::Mutex` instead of `mea::mutex::Mutex`.

`rustc --print cfg --target powerpc-unknown-linux-gnu` reports
`target_pointer_width="32"` and atomic widths 8, 16, 32, and pointer-sized;
it does not report 64-bit atomic support. This agrees with the actual
compiler diagnostic.

### Existing test coverage and remaining verification

Searched `crates/uv/tests/`, `crates/uv-client/tests/it/`, and
`crates/uv-auth/` for PPC32/PowerPC, `AtomicU64`, `reqsign`, `mea`, and
`asyncband` coverage. No test covering this dependency's PPC32 compilation
was found. No repository tests were added or run; the isolated cross-target
checks directly exercise the reported build blocker.

The exact reporter target triple, Rust version, uv revision, build command,
and features remain unspecified. They are needed to verify the full uv
build in the reporter's environment, but not to reproduce this dependency
failure. A successful `asyncband` check does not prove that updating the
complete uv dependency graph resolves every PPC32 build issue.

Artifacts are retained at `/tmp/uv-22906-flgmniia`, including both fixtures,
their lockfiles, `mea-check.log`, `asyncband-check.log`, published crate
sources, and isolated toolchain/cache/output directories.

## Related

- astral-sh/uv#22925 — Update Rust crate reqsign to v0.20.7 (merged October 26, 2026). Linked by a maintainer in the issue discussion. The lockfile diff updates reqsign-core to 3.3.2, removes mea 0.6.7, and selects asyncband 0.7.4. This implements the requested dependency update; complete PPC32 uv build verification remains outstanding.
- apache/reqsign#868 — refactor: replace mea with asyncband 0.7.1 (merged). apache/reqsign#868 replaces mea with asyncband and is included in reqsign-core 3.3.2, published September 25, 2026. This supplies the upstream dependency replacement now incorporated into uv by astral-sh/uv#22925.
- astral-sh/uv#2658 — Dependency Dashboard (open at initial triage). The initial inspection listed reqsign 0.20.7 under Pending Status Checks. That release requires reqsign-core ^3.3.2. The update has since merged through astral-sh/uv#22925; the dashboard entry is historical context rather than an outstanding fix.

## Supporting evidence

- The issue discussion links astral-sh/uv#22925. Read-only inspection of the
  PR metadata and `Cargo.lock` diff confirms the merged versions and removal
  of `mea` described above.
- The local file references below document the affected checkout examined
  during the original triage and reproduction, not the merged PR state.
- `Cargo.toml:255` declares `reqsign`; `crates/uv-auth/Cargo.toml:36`
  consumes it for uv's cloud authentication providers.
- `Cargo.lock:3812` selects `reqsign` 0.20.6.
  `Cargo.lock:3894` selects `reqsign-core` 3.3.1 and includes `mea`
  in its dependencies. `Cargo.lock:2723` selects `mea` 0.6.7.
- The published `mea` 0.6.7 archive has an unconditional
  `pub mod broadcast;` in `src/lib.rs:82`, an unconditional
  `pub mod overflow;` in `src/broadcast/mod.rs:21`, and
  `use std::sync::atomic::AtomicU64;` in
  `src/broadcast/overflow/mod.rs:67`.
  Its README says future development and releases will use the
  `asyncband` name.
  Source: https://static.crates.io/crates/mea/mea-0.6.7.crate
- Rust's `powerpc-unknown-linux-gnu` target specifies
  `max_atomic_width = Some(32)` and a 32-bit pointer width.
  Source:
  https://raw.githubusercontent.com/rust-lang/rust/1.90.0/compiler/rustc_target/src/spec/targets/powerpc_unknown_linux_gnu.rs
- apache/reqsign#868 merged on September 4, 2026. It replaces
  `mea::mutex::Mutex` with `asyncband::mutex::Mutex`, enables the
  `mutex` feature, and raises the upstream minimum Rust version to 1.86.
  The uv workspace already requires Rust 1.97.
- Crates.io records `reqsign-core` 3.3.2 as published on September 25,
  2026, and not yanked. Its normal dependencies include `asyncband ^0.7.1`
  and exclude `mea`.
  Sources:
  https://crates.io/api/v1/crates/reqsign-core/3.3.2
  and https://crates.io/api/v1/crates/reqsign-core/3.3.2/dependencies
- The published `asyncband` 0.7.1 source contains no `AtomicU64`
  reference or `broadcast/overflow` module; synchronization modules
  are feature-gated.
  Source: https://static.crates.io/crates/asyncband/asyncband-0.7.1.crate
- `reqsign` 0.20.7 requires `reqsign-core ^3.3.2`, connecting the
  dashboard update, now merged as astral-sh/uv#22925, to this report.
  Source: https://crates.io/api/v1/crates/reqsign/0.20.7/dependencies

## Search coverage and excluded candidates

The report was decomposed before searching into the requested patch update,
PPC32 compatibility, the maintenance/rename claim, and the removal of broken
dependency code. No specific uv command or error text was supplied. Literal
package/version searches and platform/build symptom searches were conducted
separately; the later `AtomicU64` search followed source inspection.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh: reqsign-core, reqsign, 3.3.2, mea, asyncband, PPC32, PowerPC, ppc, powerpc-unknown-linux-gnu, 32-bit builds, unmaintained dependencies, Rust dependency updates, and AtomicU64. Inspected candidate bodies, comments, timelines, referenced fixes, and recent PRs. Ruled out astral-sh/uv#3423 and its merged fix astral-sh/uv#7271 (tokio-tar), astral-sh/uv#9793 and astral-sh/uv#9810 (64-bit PowerPC musl linking), astral-sh/uv#22273 (GCS identity fallback), and astral-sh/uv#18287 (JWT advisory). Followed the upstream release comparison to apache/reqsign#868 and checked published crate sources and dependency metadata. The initial search found no existing uv discussion or PR for this specific failure. The later maintainer comment identifies astral-sh/uv#22925, whose merged lockfile diff was inspected for this update.

The historical `tokio-tar` issue was particularly plausible because it also
reported an unmaintained dependency broken on PowerPC. Its comments and
closing PR establish a different archive-processing dependency, fixed in
September 2024, rather than this synchronization dependency. The older musl
case concerns `powerpc64le-unknown-linux-musl` linker relocations, not PPC32
atomic support. The other `reqsign` reports concern authentication fallback
and JWT decoding exposure, not compilation.

## Recommended next step

Use merge commit `c3bcf4bb9d51ffc918cfdfcb15d716e061f3ba2e` from
astral-sh/uv#22925, or a revision containing it, for any remaining complete
uv PPC32 build verification. Verify the reporter's exact target and build
configuration with the merged dependency graph, including `asyncband` 0.7.4.
The requested version update and removal of `mea` are already merged; no
additional dependency-update PR is indicated by the available evidence.
Confirm which published uv release includes the merge before recommending
a released version as containing the fix.

The existing isolated reproduction findings and commands above are unchanged.
This context update modified only this README; no checkout files, other
context artifacts, or GitHub state were changed.
