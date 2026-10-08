//! Commands for inspecting and modifying Python environments.

pub use pylock::PylockResolutionError;

pub mod check;
pub mod compile;
pub mod freeze;
pub mod install;
pub mod list;
pub mod show;
pub mod sync;
pub mod tree;
pub mod uninstall;

mod install_report;
mod pylock;
mod reporters;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentValidation {
    Enabled,
    Disabled,
}

impl EnvironmentValidation {
    pub const fn from_args(strict: bool) -> Self {
        if strict {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}
