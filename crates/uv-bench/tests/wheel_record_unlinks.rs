//! Correctness tests for the opt-in wheel RECORD deletion experiment.

#[path = "../benches/wheel_record_unlinks/fixture.rs"]
mod fixture;

#[path = "../benches/wheel_record_unlinks/leaf.rs"]
mod leaf;

#[path = "../benches/wheel_record_unlinks/settings.rs"]
mod settings;

#[path = "../benches/wheel_record_unlinks/timing.rs"]
mod timing;

#[path = "wheel_record_unlinks/fixture_cases.rs"]
mod fixture_cases;

#[path = "wheel_record_unlinks/portable_cases.rs"]
mod portable_cases;

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
mod uring {
    include!("../benches/wheel_record_unlinks/uring.rs");
    include!("wheel_record_unlinks/uring_cases.rs");
}
