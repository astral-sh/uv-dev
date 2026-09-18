//! Windows-specific utilities for uv.
//!
//! This crate provides shared Windows functionality used by both the main `uv` crate
//! and `uv-trampoline`. It supports `no_std`-friendly usage via the `std` feature flag.

#![cfg(windows)]

mod ctrl_handler;
#[cfg(feature = "std")]
mod exception;
#[cfg(feature = "std")]
mod file_identity;
mod job;
#[cfg(feature = "std")]
mod spawn;
mod wine;

pub use ctrl_handler::{CtrlHandlerError, install_ctrl_handler};
#[cfg(feature = "std")]
pub use exception::install_unhandled_exception_handler;
#[cfg(feature = "std")]
pub use file_identity::{
    FileIdentity, directory_is_case_sensitive, names_equal_ordinal, open_directory, open_file_entry,
};
pub use job::{Job, JobError};
#[cfg(feature = "std")]
pub use spawn::spawn_child;
#[cfg(feature = "std")]
pub use wine::is_wine;
