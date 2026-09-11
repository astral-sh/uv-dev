//! Tests for the benchmark-only source cache-key STATX collector.

#![cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]

mod statx {
    include!("../benches/cache_key_backends/statx.rs");
    include!("cache_key_statx/cases.rs");
}
