# Build failures on SPARC due ot missing symbols in `rustix::fs`

Issue: astral-sh/uv#22461

Classification: bug

## Summary

The reporter cannot compile uv on Gentoo Linux SPARC64 at commit
`4bf8e7be2777dc545cd554ba91a4d9cd4bb9e594` (2026-10-10). The report contains
two independent compilation failures in `uv-fs`:

- `space.rs:13–14`: E0432 for seven `FIEMAP_EXTENT_*` constants and
  `FS_IOC_FIEMAP` imported from `linux_raw_sys::ioctl`. This affects the
  physical-storage accounting used by cache cleanup.
- `link.rs:443`: E0425 for `rustix::fs::ioctl_ficlone`, used by the Linux
  reflink implementation.

The requested mitigation is to disable the unavailable paths on SPARC.
No Python command or Python version is needed to establish these Rust compilation
errors. The exact Rust target triple, toolchain, and build invocation were not supplied.

astral-sh/uv#21056 and astral-sh/uv#18187 introduced the two failing code paths. bytecodealliance/rustix#1700 reports the same upstream failures; astral-sh/uv#21133 supplies related runtime accounting fallback. No existing uv duplicate or SPARC fix was found.

## Draft response

Your errors match two compile-time platform gaps: linux-raw-sys 0.12.1 has no SPARC64 ioctl constants, and rustix explicitly excludes SPARC/SPARC64 from ioctl_ficlone. uv currently enables both paths for all Linux targets. The LoongArch report on astral-sh/uv#21056 covers the FIEMAP portion, but its upstream fixes would not address both SPARC failures.

Disabling these paths on SPARC needs compile-time guards, with coarse cache accounting and the existing hardlink/copy fallback. Changing runtime options cannot avoid these compiler errors. Please share your exact Rust target triple and build command so a change can be validated against your setup.

## Classification

Source confirms that uv enables both APIs for every Linux architecture, while rustix explicitly excludes SPARC/SPARC64 from ioctl_ficlone and linux-raw-sys 0.12.1 has empty SPARC64 ioctl bindings. This is a compile-time portability bug, even though SPARC64 is outside uv's listed supported platforms. No existing uv issue or fix was found that tracks both failures or a previous SPARC fix that regressed. The upstream report concerns dependency support; the related merged uv PRs introduced the affected code or runtime fallback behavior.

The LoongArch comments on astral-sh/uv#21056 overlap with the first failure but
do not cover the second missing API on SPARC. bytecodealliance/rustix#1700 was
opened by the same reporter earlier on the same day; a uv-side fallback remains
a separate actionable change. The permissions fix in astral-sh/uv#18187 and the
accounting fix in astral-sh/uv#21133 did not fix these SPARC compilation failures,
so their merged status does not establish a recurrence of a previously fixed SPARC bug.

## Related

- astral-sh/uv#21056 — Use linux-raw-sys for FIEMAP constants (merged). Introduced the failing FIEMAP imports on 2026-08-11. An October 9 comment reports the same missing extent constants on LoongArch64. SPARC64 additionally lacks FS_IOC_FIEMAP and ioctl_ficlone; this PR introduced the dependency rather than fixing SPARC builds.
- astral-sh/uv#18187 — Preserve file permissions when using reflinks on Linux (merged). Introduced the Linux-wide rustix::fs::ioctl_ficlone call that now fails to compile on SPARC. Merged on 2026-02-24 to fix executable permissions, a different problem; any fallback must retain correct permissions.
- bytecodealliance/rustix#1700 — `ioctl_ficlone` and related symbols seem to be missing on SPARC64 (open). The reporter's upstream issue contains both identical SPARC64 compiler errors and identifies rustix 1.1.5. It has no maintainer response or fix; it tracks dependency support while this issue requests a uv-side fallback.
- astral-sh/uv#21133 — Fall back to logical cache accounting on unsupported filesystems (merged). Added cache-accounting fallback on 2026-08-14. This provides relevant fallback behavior, but handles runtime filesystem errors after compilation and cannot resolve missing architecture-specific symbols.

## Supporting evidence

The affected source was inspected both in this checkout at
`44b2e5877ef752e360acc02c7d198ba71ddf948d` and through GitHub at the reporter's
commit. Both revisions enable the failing imports and call with
`#[cfg(target_os = "linux")]`, without excluding SPARC architectures.

- `crates/uv-fs/src/space.rs:11` imports the FIEMAP constants on all Linux
  targets. The capability predicate at line 33 also advertises fine-grained
  accounting on every Linux target; the Linux implementation starts at line 131.
- `crates/uv-fs/src/link.rs:426` enables the Linux reflink helper, including
  the unavailable call at line 443. Its caller already has a fallback strategy
  for failed reflinks, but a runtime error path cannot handle an absent Rust API.
- rustix tags `v1.1.4` and `v1.1.5`, `src/fs/ioctl.rs`, explicitly gate
  `ioctl_ficlone` with
  `not(any(target_arch = "sparc", target_arch = "sparc64"))`.
  This confirms API absence; it does not establish whether the kernel itself
  could support the operation.
- linux-raw-sys tag `v0.12.1`, `src/sparc64/ioctl.rs`, contains only a
  generated-file comment and no constants. The checkout locks this version and
  rustix 1.1.4; the reporter's upstream issue explicitly uses rustix 1.1.5.
  The rustix exclusion exists in both versions.
- `crates/uv-cache/src/removal.rs:113` falls back to coarse accounting on
  `PhysicalSpaceError::UnsupportedFilesystem`.
  `crates/uv-cache/src/lib.rs:217` also selects coarse accounting when the
  platform capability predicate is false. These are existing mechanisms a
  platform-specific change could use.
- `docs/reference/policies/platforms.md` does not list SPARC64 among supported
  platforms. This limits build guarantees but does not change the confirmed
  compile-time defect.

The change introducing FIEMAP imports merged on 2026-08-11; the direct rustix
cloning call merged on 2026-02-24. Both predate the reported October commit.
No known-good SPARC version or independently reproduced SPARC build was established.

## Other candidates inspected

The source history leads to astral-sh/uv#20925 and its original report,
astral-sh/uv#18779, about misleading reclaimed disk-space output. These explain
why physical accounting exists, but concern runtime output rather than compilation.

astral-sh/uv#18181 reports lost executable permissions during Linux reflinks.
Its closing change, astral-sh/uv#18187, introduced the direct rustix call. The
permissions report is not a duplicate of missing symbols.

The reporter-linked sunfishcode/linux-raw-sys#193 remains open and adds LoongArch64
FIEMAP constants. Its issue, sunfishcode/linux-raw-sys#192, states that LoongArch
already exports `FS_IOC_FIEMAP` but lacks extent flags. That differs from the empty
SPARC64 bindings. The related sunfishcode/linux-raw-sys#195 also remains open;
its comments point back to sunfishcode/linux-raw-sys#193. The linked
bytecodealliance/rustix#1664 was closed because it belonged in the separate
linux-raw-sys repository, not because a fix had shipped. None of these changes
addresses SPARC's missing `ioctl_ficlone`.

## Search scope and limitations

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using FIEMAP, FIEMAP_EXTENT_SHARED, FS_IOC_FIEMAP, ioctl_ficlone, FICLONE, SPARC/SPARC64, rustix, linux-raw-sys, unresolved imports, cannot find function, and space.rs. Conceptual searches covered reflinks, disk usage, physical space, unsupported architectures, Linux builds, and LoongArch; fix-oriented searches covered closed SPARC reports and merged accounting changes. PR searches returned no matches despite relevant PRs found through file history and issue timelines; one final search was rate-limited. Inspected candidate bodies, comments, reviews, and diffs, following the accounting and permissions histories and upstream links. Ruled out astral-sh/uv#18181 and astral-sh/uv#18779 as duplicates: they concern runtime permissions and misleading reclaimed-size output. sunfishcode/linux-raw-sys#192, sunfishcode/linux-raw-sys#193, sunfishcode/linux-raw-sys#195, and bytecodealliance/rustix#1664 concern LoongArch FIEMAP bindings, not both SPARC failures.

Search terms were decomposed before searching into the two compiler symptoms,
their distinct subsystems, the SPARC64/Linux trigger, and the requested
platform-specific fallback. Dependency-cause searches were kept separate from
symptom searches. Repository labels such as `area:linux` and the vocabulary
in accounting and reflink discussions informed the conceptual searches.

Authenticated GitHub CLI operations were read-only. PR search results were
supplemented with direct PR retrieval, commit history, issue timelines, and
source inspection; the empty search results should not be interpreted as
proof that no relevant PR exists.

## Suggested next step

Confirm the reporter's target triple and build invocation, then implement and
validate architecture guards for both unavailable paths. Keep the physical-space
capability predicate consistent with the compiled implementation, and preserve
correct file permissions when falling back from reflinks. A SPARC compile check
is needed before claiming the full build is fixed; additional build blockers
were not investigated.

This handoff is based on compiler diagnostics and source inspection. No builds
or tests were run, no checkout files were changed, and no GitHub changes were made.
