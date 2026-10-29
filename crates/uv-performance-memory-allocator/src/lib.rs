//! The only purpose of this crate is to pull in `mimalloc` on Windows and x86-64 Linux and
//! `tikv-jemallocator` on most other platforms.

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", target_arch = "x86_64")
))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(all(
    not(target_os = "windows"),
    not(all(target_os = "linux", target_arch = "x86_64")),
    not(target_os = "openbsd"),
    not(target_os = "freebsd"),
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64"
    )
))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
