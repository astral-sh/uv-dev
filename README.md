# Please update `reqsign-core` to 3.3.2

Issue: astral-sh/uv#22906

Classification: bug

## Summary

The reporter requests `reqsign-core` 3.3.2 because the currently selected version
depends on `mea`, which is broken on PPC32. They report that development moved
to `asyncband` and that the replacement removed the problematic code.

The checkout locks `reqsign` 0.20.6, `reqsign-core` 3.3.1, and `mea` 0.6.7.
Published source confirms an unconditional `AtomicU64` import in `mea`; the
32-bit PowerPC Linux Rust target only supports atomics through 32 bits.
`reqsign-core` 3.3.2 replaces `mea` with `asyncband` 0.7.1, whose published
source no longer contains that import or the overflow broadcast module.

apache/reqsign#868 supplies the fix released in reqsign-core 3.3.2; astral-sh/uv#2658 queues a broader reqsign update that includes it. No existing uv issue or PR tracks this specific PPC32 failure.

The issue was opened on October 24, 2026, and has no comments. It does not
provide an exact target triple, build command, diagnostic, uv version, or a
before-and-after build result.

## Draft response

The lockfile still selects reqsign-core 3.3.1 and mea 0.6.7. mea unconditionally uses AtomicU64, which is unavailable on 32-bit PowerPC Linux. reqsign-core 3.3.2 replaces it with asyncband via apache/reqsign#868 and removes that dependency blocker. The next step is a targeted lockfile update and PPC32 build verification. Please share your exact Rust target triple and whether updating to 3.3.2 resolves the failure for you.

## Classification

The requested update addresses a source-confirmed build incompatibility: uv locks mea 0.6.7 through reqsign-core 3.3.1, and mea unconditionally imports AtomicU64 while the 32-bit PowerPC Linux target supports atomics only through 32 bits. The upstream replacement is released but not adopted in this checkout. The general dependency dashboard and unrelated historical PowerPC fixes do not make this a duplicate or an established regression.

The missing reproduction does not change the correctness classification.
Source inspection establishes a dependency-level compile incompatibility for
`powerpc-unknown-linux-gnu`; it does not establish that a complete uv build
passes after the update, or identify the reporter's exact target. No
reproduction or build was run.

The dashboard is a general dependency queue, not a canonical discussion of
this failure. The merged upstream change is available for uv to consume;
it does not update uv's lockfile.

## Related

- apache/reqsign#868 — refactor: replace mea with asyncband 0.7.1 (merged). apache/reqsign#868 replaces mea with asyncband and is included in reqsign-core 3.3.2, published September 25, 2026. This supplies the upstream dependency change needed here; uv still locks reqsign-core 3.3.1.
- astral-sh/uv#2658 — Dependency Dashboard (open). astral-sh/uv#2658 lists reqsign 0.20.7 under Pending Status Checks. That release requires reqsign-core ^3.3.2, so the queued update would incorporate the requested version. The dashboard does not track the PPC32 failure or link an implementing PR for this update.

## Supporting evidence

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
  pending dashboard update to this report.
  Source: https://crates.io/api/v1/crates/reqsign/0.20.7/dependencies

## Search coverage and excluded candidates

The report was decomposed before searching into the requested patch update,
PPC32 compatibility, the maintenance/rename claim, and the removal of broken
dependency code. No specific uv command or error text was supplied. Literal
package/version searches and platform/build symptom searches were conducted
separately; the later `AtomicU64` search followed source inspection.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh: reqsign-core, reqsign, 3.3.2, mea, asyncband, PPC32, PowerPC, ppc, powerpc-unknown-linux-gnu, 32-bit builds, unmaintained dependencies, Rust dependency updates, and AtomicU64. Inspected candidate bodies, comments, timelines, referenced fixes, and recent PRs. Ruled out astral-sh/uv#3423 and its merged fix astral-sh/uv#7271 (tokio-tar), astral-sh/uv#9793 and astral-sh/uv#9810 (64-bit PowerPC musl linking), astral-sh/uv#22273 (GCS identity fallback), and astral-sh/uv#18287 (JWT advisory). Followed the upstream release comparison to apache/reqsign#868 and checked published crate sources and dependency metadata. No existing uv discussion or PR for this specific failure was found.

The historical `tokio-tar` issue was particularly plausible because it also
reported an unmaintained dependency broken on PowerPC. Its comments and
closing PR establish a different archive-processing dependency, fixed in
September 2024, rather than this synchronization dependency. The older musl
case concerns `powerpc64le-unknown-linux-musl` linker relocations, not PPC32
atomic support. The other `reqsign` reports concern authentication fallback
and JWT decoding exposure, not compilation.

## Recommended next step

Prepare a focused lockfile update using
`cargo update -p reqsign-core --precise 3.3.2`, review the resulting
dependency changes, and verify that `mea` leaves the dependency graph.
Check a build for the reporter's exact PPC32 target and run the relevant
repository checks before claiming the issue is resolved. The broader
`reqsign` 0.20.7 update queued in astral-sh/uv#2658 is another route to
the required core version.

This handoff is based on read-only repository and upstream inspection.
Only this README was updated; no checkout files or GitHub state were changed.
