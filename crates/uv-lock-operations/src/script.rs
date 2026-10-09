use anyhow::{Result, bail};
use owo_colors::OwoColorize;

use uv_settings::{FrozenSource, LockCheck, LockedSource};
use uv_warnings::warn_user;

/// Report lockfile requirements for a Python script without an existing lockfile.
///
/// Explicit CLI flags require a lockfile. Environment settings warn instead so globally setting
/// `UV_LOCKED` or `UV_FROZEN` does not prevent running an unlocked script.
pub fn handle_missing_script_lockfile(
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
) -> Result<()> {
    if let LockCheck::Enabled(lock_check) = lock_check {
        match lock_check {
            LockedSource::Cli(_) => {
                bail!(
                    "Unable to find lockfile for Python script, but `{lock_check}` was provided. To create a lockfile, run `{}`.",
                    "uv lock --script".green(),
                );
            }
            LockedSource::Env => {
                warn_user!(
                    "No lockfile found for Python script (ignoring `{lock_check}`); run `{}` to generate a lockfile",
                    "uv lock --script".green(),
                );
            }
        }
    }

    if let Some(frozen_source) = frozen {
        match frozen_source {
            FrozenSource::Cli(_) => {
                bail!(
                    "Unable to find lockfile for Python script, but `{frozen_source}` was provided. To create a lockfile, run `{}`.",
                    "uv lock --script".green(),
                );
            }
            FrozenSource::Env => {
                warn_user!(
                    "No lockfile found for Python script (ignoring `--frozen`); run `{}` to generate a lockfile",
                    "uv lock --script".green(),
                );
            }
        }
    }

    Ok(())
}
