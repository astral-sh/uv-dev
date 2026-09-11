# A pure-Rust allocator for uv-performance-memory-allocator

Issue: astral-sh/uv#21622

Classification: enhancement

## Summary

The issue proposes adding `rusty_alloc-api` to `uv-performance-memory-allocator` behind an
off-by-default Cargo feature. The proposed allocator is a safe-Rust implementation based on the
mimalloc algorithm. The stated goals are to make a pure-Rust allocator available for measurement,
avoid the native C build requirements of mimalloc and jemalloc for opt-in builds, and potentially
provide one allocator across Windows, macOS, and Linux without changing uv's defaults.

The checkout confirms that `uv-performance-memory-allocator` currently selects mimalloc on Windows
and tikv-jemallocator on supported non-Windows platforms, and that uv exposes this through its
existing performance feature. No earlier issue or pull request was found that requests rusty_alloc
or another pure-Rust allocator option. The closest work established the current allocator strategy,
centralized it in this crate, and evaluates a newer mimalloc version with detailed uv-specific
timing and peak-memory measurements.

## Draft response

Thanks for the proposal. uv's allocator choices are performance-sensitive: astral-sh/uv#399
introduced the current strategy from uv-specific resolver measurements, astral-sh/uv#7686
centralized it in `uv-performance-memory-allocator`, and the ongoing work in astral-sh/uv#19743
shows that allocator changes can trade timing improvements against peak-memory regressions across
uv workloads. The upstream `xthread` result is therefore a useful lead, but it does not establish a
benefit for uv.

A concrete next step would be to provide reproducible uv-specific benchmarks comparing rusty_alloc
with the current allocator on representative warm and cold resolution and installation workloads,
including wall time and peak RSS on the proposed platforms. An off-by-default feature still adds
dependency and configuration maintenance, so we would want that evidence before deciding whether
to add it.

## Classification

This is an enhancement. It requests a new Cargo feature, optional dependency, and allocator choice;
it does not report that current uv behavior is incorrect. No existing issue or pull request tracks
the same rusty_alloc integration closely enough for the report to be a duplicate.

The issue's cross-thread-free performance claim comes from the proposed allocator's own benchmark.
Repository evidence supports allocator sensitivity in uv, but it does not confirm that rusty_alloc
would improve uv's resolver or installer. In particular, the current mimalloc v3 experiments show
that workload-specific timing gains can coexist with peak-RSS regressions, which is why uv-specific,
multi-workload evidence is the appropriate next step.

## Related

- astral-sh/uv#7686 — merged pull request, “Clean up \"performance allocators\" and \"performance
  flate2\" backends.” This created the dedicated `uv-performance-memory-allocator` crate and
  centralized the target-specific mimalloc/jemalloc selection that astral-sh/uv#21622 proposes
  extending. It does not add or discuss a pure-Rust allocator.
- astral-sh/uv#19743 — open pull request, “Upgrade Windows allocator to mimalloc v3.” This is an
  active allocator experiment in the same crate. Its uv-specific warm/cold timing and peak-RSS
  results demonstrate that allocator changes have workload-dependent tradeoffs. It changes the
  default Windows allocator version rather than offering a pure-Rust opt-in.
- astral-sh/uv#399 — merged pull request, “change global allocator to jemalloc (and mimalloc on
  Windows).” This introduced the allocator strategy after measuring an approximately 10% resolver
  improvement and supplies the historical performance rationale for the current choices. It does
  not cover rusty_alloc or an optional pure-Rust implementation.

## Search scope and exclusions

Literal searches covered `rusty_alloc`, “pure Rust allocator,”
`uv-performance-memory-allocator`, “global allocator,” mimalloc, jemalloc, allocator features,
C toolchains, and build scripts across open and closed issues and open, closed, and merged pull
requests. Conceptual searches covered alternative allocators, allocation and resolver performance,
native dependency and toolchain portability, and disabling performance dependencies. Fix-oriented
review followed the original allocator-performance discussion and the current mimalloc v3 work,
including linked maintainer comments and experiments.

astral-sh/uv#396 was inspected because it discussed allocation cost and trying alternative
allocators, but it was a broad performance report closed after resolver representation and parsing
improvements; it does not track a pure-Rust allocator feature. astral-sh/uv#14574 and
astral-sh/uv#16849 were inspected because their Android and illumos build discussions mention
tikv-jemalloc limitations, but they concern specific platform support and packaging failures rather
than the general opt-in capability requested here. astral-sh/uv#19189 and its linked mimalloc fixes
were also reviewed; they concern a Windows ARM64 mimalloc v3 crash, not the proposed allocator.
