# Build failures on SPARC due ot missing symbols in `rustix::fs`

Issue: astral-sh/uv#22461

Classification: bug

## Summary

Both reported compiler errors were independently reproduced at uv commit
`4bf8e7be2777dc545cd554ba91a4d9cd4bb9e594` using Rust 1.99.0 and the locked
dependencies. Cross-checking `uv-fs` for `sparc64-unknown-linux-gnu` fails with
E0432 for the FIEMAP imports in `space.rs:13–14` and E0425 for
`rustix::fs::ioctl_ficlone` in `link.rs:443`. The same check for
`x86_64-unknown-linux-gnu` succeeds.

The reporter uses Gentoo Linux SPARC64 and requests disabling the unavailable
paths on SPARC. The exact Rust target triple, compiler version, and original
build invocation were not supplied, but those omissions did not prevent
reproducing both diagnostics with the standard SPARC64 GNU/Linux target.
Python is not involved in these Rust compilation failures.

astral-sh/uv#21056 and astral-sh/uv#18187 introduced the affected code paths.
bytecodealliance/rustix#1700 reports the same upstream failures;
astral-sh/uv#21133 supplies related runtime accounting fallback. The prior
issue search found no existing uv duplicate or SPARC fix.

## Classification

This is a reproduced compile-time portability bug. uv enables both APIs for
all Linux architectures, while the locked dependencies do not provide them
for SPARC64. SPARC64 is outside uv's listed supported platforms in
`docs/reference/policies/platforms.md`, so this does not establish a violation
of a documented build guarantee.

The missing symbols are not runtime filesystem failures. Runtime options or
fallbacks cannot resolve these compiler errors. The LoongArch discussion on
astral-sh/uv#21056 overlaps with the FIEMAP failure, but does not address the
missing SPARC reflink API. Neither the permissions fix in astral-sh/uv#18187
nor the accounting fallback in astral-sh/uv#21133 fixed SPARC compilation, and
no known-good SPARC version was identified.

## Reproduction

**Outcome: reproducible.** Both diagnostics were observed in an actual Rust
cross-compilation check, rather than inferred solely from source inspection.

### Environment and fixture

- Host: Linux x86_64; target checked: `sparc64-unknown-linux-gnu`.
- Compiler: `rustc 1.99.0 (b940084d7 2026-09-28)`, LLVM 23.1.1;
  Cargo: `1.99.0 (5f94df478 2026-08-27)`.
- Source: the GitHub source archive for the exact reported commit,
  `4bf8e7be2777dc545cd554ba91a4d9cd4bb9e594`, extracted under
  `/tmp/uv-22461-repro.gsFUq4/source/` without source changes.
- Package: `uv-fs` 0.0.92, default features; original `Cargo.lock` retained
  with `--locked`. It selects `linux-raw-sys` 0.12.1 and `rustix` 1.1.4.
  The upstream report mentions rustix 1.1.5; that version was not needed to
  reproduce the failure and was not compiled in this check.
- Installed executable on PATH: `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
  Only `uv --version` was needed; this issue concerns building Rust source,
  not executing a Python/project command.
- The existing Rust 1.99.0 toolchain was copied into a temporary rustup home,
  and its SPARC64 standard library was installed there. Cargo downloads,
  build outputs, and logs also remained in the temporary reproduction directory.

### Commands and observed behavior

After extracting the reported revision, the relevant commands were:

```sh
export RUSTUP_HOME=/tmp/uv-22461-repro.gsFUq4/rustup
export CARGO_HOME=/tmp/uv-22461-repro.gsFUq4/cargo-home
export RUSTUP_TOOLCHAIN=stable
export CARGO_TARGET_DIR=/tmp/uv-22461-repro.gsFUq4/target
# The copied stable toolchain is Rust 1.99.0.
rustup target add sparc64-unknown-linux-gnu
cd /tmp/uv-22461-repro.gsFUq4/source/uv-4bf8e7be2777dc545cd554ba91a4d9cd4bb9e594
cargo check --locked -p uv-fs --target sparc64-unknown-linux-gnu
```

The SPARC64 command exits **101**, after successfully checking its dependencies,
with exactly the two reported errors:

- E0432 at `crates/uv-fs/src/space.rs:13:5`: unresolved imports
  `FIEMAP_EXTENT_DATA_INLINE`, `FIEMAP_EXTENT_DELALLOC`,
  `FIEMAP_EXTENT_ENCODED`, `FIEMAP_EXTENT_LAST`,
  `FIEMAP_EXTENT_NOT_ALIGNED`, `FIEMAP_EXTENT_SHARED`,
  `FIEMAP_EXTENT_UNKNOWN`, and `FS_IOC_FIEMAP` from `linux_raw_sys::ioctl`.
- E0425 at `crates/uv-fs/src/link.rs:443:35`: cannot find function
  `ioctl_ficlone` in module `rustix::fs`.

The architecture control, using the same source, features, compiler, and lockfile:

```sh
cargo check --locked -p uv-fs --target x86_64-unknown-linux-gnu
```

exits **0** and finishes the development-profile check successfully.

Full command logs are retained at:

- `/tmp/uv-22461-repro.gsFUq4/sparc64-check.log`
- `/tmp/uv-22461-repro.gsFUq4/x86_64-check.log`

This establishes the reported compile-time failures for SPARC64. It does not
constitute a native Gentoo build, linking the complete uv executable, testing
32-bit SPARC, or validating a fix. No cross-linker or SPARC hardware was needed
for `cargo check`, and no claim is made about kernel ioctl support.

### Existing test coverage

Searched `crates/uv/tests/`, `crates/uv-client/tests/it/`, the relevant `uv-fs`
source, and workflow configuration. No SPARC-specific compilation test was
found. The following existing tests were read, including their setup and
assertions; they cover related runtime behavior, not the missing APIs:

- `crates/uv/tests/pip_install/pip_install.rs::install_executable_clone`
  places the cache and virtual environment on a configured copy-on-write
  filesystem, installs `test/packages/executable_file` using `--link-mode clone`,
  snapshots success, and checks Unix executable permission bits. It skips
  without `UV_INTERNAL__TEST_COW_FS`.
- `crates/uv/tests/build/cache_clean.rs::clean_all_physical_space_unsupported_fs`
  writes a 1 MiB cached file on the configured alternate filesystem and checks
  that `cache clean --preview-features cache-physical-space` succeeds and
  reports 1.0 MiB removed.
- `crates/uv/tests/build/cache_prune.rs::prune_physical_space_unsupported_fs`
  creates a 1 MiB file in a stale cache bucket on that alternate filesystem
  and checks that pruning with the same preview feature reports 1.0 MiB removed.
  Both accounting tests are Unix-only and skip without `UV_INTERNAL__TEST_ALT_FS`.

These integration modules are gated by `test-python` and `test-pypi`.
They cannot verify absent architecture-specific symbols without first compiling
for that architecture. No integration tests were run or added; the targeted
crate checks are the reproduction.

## Related

- astral-sh/uv#21056 — Use linux-raw-sys for FIEMAP constants (merged).
  Introduced the failing imports on 2026-08-11. An October 9 comment reports
  missing extent constants on LoongArch64. SPARC64 additionally lacks
  `FS_IOC_FIEMAP` and `ioctl_ficlone`.
- astral-sh/uv#18187 — Preserve file permissions when using reflinks on Linux
  (merged). Introduced the Linux-wide `rustix::fs::ioctl_ficlone` call on
  2026-02-24 to fix executable permissions. Any fallback must retain correct
  permissions.
- bytecodealliance/rustix#1700 — `ioctl_ficlone` and related symbols seem to
  be missing on SPARC64 (open at the prior context review). The same reporter
  supplied both identical errors and identified rustix 1.1.5. No maintainer
  response or fix was found during that review.
- astral-sh/uv#21133 — Fall back to logical cache accounting on unsupported
  filesystems (merged). Added runtime accounting fallback on 2026-08-14;
  this cannot resolve absent architecture-specific APIs during compilation.

## Supporting evidence

The reported revision's `crates/uv-fs/src/space.rs`,
`crates/uv-fs/src/link.rs`, and `crates/uv-fs/Cargo.toml` are byte-for-byte
identical to those in the provided checkout at
`44b2e5877ef752e360acc02c7d198ba71ddf948d`.

- `space.rs` imports the FIEMAP constants under `#[cfg(target_os = "linux")]`.
  Its capability predicate advertises fine-grained accounting on all Linux
  targets, and its Linux implementation uses those constants.
- `link.rs` enables the reflink helper and direct rustix call for all Linux
  targets. A runtime reflink fallback cannot handle an absent Rust API.
- The downloaded rustix 1.1.4 source used in the failing check explicitly
  excludes `target_arch = "sparc"` and `target_arch = "sparc64"` from
  `ioctl_ficlone` in `src/fs/ioctl.rs`. The prior source review also found
  this exclusion in rustix 1.1.5.
- The downloaded linux-raw-sys 0.12.1 `src/sparc64/ioctl.rs` contains only
  a generated-file comment and no constants.
- `crates/uv-cache/src/removal.rs` falls back to coarse accounting on
  `PhysicalSpaceError::UnsupportedFilesystem`.
  `crates/uv-cache/src/lib.rs` also selects coarse accounting when
  `supports_fine_grained_accounting()` is false. These are mechanisms a
  platform-specific change could use, but no change was implemented here.

## Other candidates and search scope

The prior context review searched open and closed uv issues and pull requests
for the compiler symptoms, FIEMAP constants, SPARC/SPARC64, rustix,
linux-raw-sys, reflinks, physical accounting, unsupported architectures, and
LoongArch. It supplemented empty PR search results with direct PR retrieval,
source history, issue timelines, comments, reviews, and diffs; one final search
was rate-limited. Those results are not proof that no other relevant PR exists.
GitHub statuses in this handoff reflect that review and were not refreshed
during reproduction.

The accounting history leads to astral-sh/uv#20925 and astral-sh/uv#18779,
which concern misleading reclaimed disk-space output. astral-sh/uv#18181
concerns lost executable permissions and was addressed by astral-sh/uv#18187.
These runtime reports are not duplicates of the two compiler errors.

The reporter-linked sunfishcode/linux-raw-sys#193 adds LoongArch64 FIEMAP
constants. sunfishcode/linux-raw-sys#192 states that LoongArch already exports
`FS_IOC_FIEMAP` but lacks extent flags, unlike the empty SPARC64 bindings.
sunfishcode/linux-raw-sys#195 points back to sunfishcode/linux-raw-sys#193.
Those PRs were open at the prior review. bytecodealliance/rustix#1664 was
closed because it belonged in linux-raw-sys, not because a fix had shipped.
None addresses SPARC's missing `ioctl_ficlone`.

## Suggested next step

Consider compile-time architecture guards for both unavailable paths, keeping
the physical-space capability predicate consistent with the implementation
and preserving permissions when falling back from reflinks. Re-run the
SPARC64 crate check, then validate the complete uv build against the reporter's
Gentoo toolchain and build options. The two failures are already reproduced;
additional setup details would help validate a fix rather than establish the
reported behavior. Other possible full-build blockers remain untested.

Only temporary reproduction files and this README were written. The repository
checkout and existing user state were not modified, and no GitHub changes were
made.
