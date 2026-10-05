#![expect(
    clippy::result_large_err,
    reason = "Cross-crate errors include the discriminant in the size; keep the shared project error representation."
)]

//! Tool command implementations.

pub mod audit;
pub mod common;
pub mod dir;
pub mod install;
pub mod list;
pub mod run;
pub mod target;
pub mod uninstall;
pub mod update_shell;
pub mod upgrade;
