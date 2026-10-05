#![expect(
    clippy::result_large_err,
    reason = "Cross-crate errors include the discriminant in the size; keep the shared project error representation."
)]

pub mod sync;
