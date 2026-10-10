//! Commands for inspecting and modifying Python environments.

use uv_configuration::HashCheckingMode;
use uv_distribution_types::Sourced;

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

/// Require build hashes independently of runtime checking. Otherwise, verify supplied build
/// hashes only when runtime checking is enabled.
fn resolve_build_hash_checking(
    hash_checking: Option<Sourced<HashCheckingMode>>,
    build_hash_checking: HashCheckingMode,
) -> Option<Sourced<HashCheckingMode>> {
    match build_hash_checking {
        HashCheckingMode::Require => Some(HashCheckingMode::Require.into()),
        HashCheckingMode::Verify => {
            hash_checking.map(|mode| mode.map(|_| HashCheckingMode::Verify))
        }
    }
}
